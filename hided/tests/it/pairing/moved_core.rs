//! A core that was moved to another machine before 1.2.0 becomes two Hides
//! (PRD device-connect-iroh D-46, B45), with every process real: the
//! machine the core was moved to runs it from an older build, and the
//! machine it was moved off is a daemon in the node role whose state folder
//! holds what the move left, a record that names its core over an SSH alias.
//! The alias leads to a real `sshd`, and its key asks for a passphrase, so
//! every sign-in is the operator's at the node's window.
//!
//! The journey, as each operator would see it: the node updates its core
//! over the saved SSH login and only then asks to be split from it; while
//! the connection two Hides use cannot be made the node stays the node it
//! was, says why on its window, and its core goes on working on it; the
//! operator's disconnect is kept across the daemon's start; and once the
//! connection can be made the node becomes its own core, paired both ways
//! with the machine that kept the moved core, whose terminal, agent, letter
//! and mission go on as they were.
//!
//! The two endpoints bind loopback with no relay (`HIDE_PEER_DIRECT`). The
//! node's is told an address no machine has until the last part, which is
//! how the new connection is made to fail while the SSH login works.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

use super::guest_work::{Machine, PROGRAMS, WITHIN, free_port, left, place_build, types};
use super::{Terminal, Window, moved_off_dials_in};
use crate::support::remote_delivery::{
    Agent, Environment, end_holders, place_stand_in, quote, successful, wait_within,
};
use crate::support::shipped;
use crate::support::sshd::Sshd;

/// The machine the core was moved to, which keeps it.
const CORE: &str = "moved-core-studio";
/// The machine the core was moved off: its node until the two are split.
const ORIGINAL: &str = "moved-core-laptop";
/// The SSH alias the move saved for the core's machine.
const ALIAS: &str = "studio";
const PASSPHRASE: &str = "moved-core-passphrase";
/// The folder of the build the core ran when it was moved
/// (`hide_kit::is_build_name`).
const OLD_BUILD: &str = "00000000000001a1";
/// The version that build says it is: below any this worktree builds, so
/// the node reads its core as the older one.
const OLD_VERSION: &str = "0.0.1";
/// An address no machine has (TEST-NET-1, RFC 5737): an endpoint told to
/// bind it does not open.
const NO_ADDRESS: &str = "192.0.2.1:9";
/// How long the link may take to get where a step waits for it: a sign-in,
/// a build sent over SSH and the core started from it, a dial. A hang
/// guard, never a pause.
const LINK_WITHIN: Duration = Duration::from_secs(300);

/// What the daemon on `port` answers `GET /health`.
fn health(port: u16) -> Option<Value> {
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .ok()?;
    let mut answer = String::new();
    stream.read_to_string(&mut answer).ok()?;
    serde_json::from_str(answer.split("\r\n\r\n").nth(1)?).ok()
}

/// Waits until the daemon of `machine` answers `/health` as `wanted` reads
/// it.
fn health_becomes(machine: &Machine, what: &str, wanted: impl Fn(&Value) -> bool) -> Value {
    let mut last = Value::Null;
    wait_within(what, WITHIN, || {
        last = health(machine.screen().port).unwrap_or(Value::Null);
        Ok(wanted(&last).then(|| last.clone()))
    })
    .unwrap_or_else(|error| panic!("{error:#}: {last}"))
}

/// What the operator does at the node's window until its link is as
/// `wanted` reads it: a Retry on each wait that is theirs to end, and the
/// key's passphrase each time ssh asks for it. Answers the link as `wanted`
/// took it, or `None` when the window's connection ended first, as it does
/// when the machine becomes its own core, and how many times the passphrase
/// was typed.
async fn at_the_window(
    window: &mut Window,
    what: &str,
    wanted: impl Fn(&Value) -> bool,
) -> (Option<Value>, usize) {
    let deadline = Instant::now() + LINK_WITHIN;
    let (mut typed, mut answered, mut pressed) = (0, None::<u64>, false);
    let mut seen: Vec<String> = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let next = tokio::time::timeout(left, window.0.next())
            .await
            .unwrap_or_else(|_| panic!("{what}: the window was told {seen:?}"));
        let text = match next {
            Some(Ok(Message::Text(text))) => text,
            Some(Ok(Message::Close(_)) | Err(_)) | None => return (None, typed),
            Some(Ok(_)) => continue,
        };
        let frame: Value = serde_json::from_str(&text).unwrap();
        if frame["type"] != "core_link" {
            continue;
        }
        let link = frame["payload"].clone();
        seen.push(format!(
            "{} {} {}",
            link["phase"], link["problem"], link["split_failed"]
        ));
        if wanted(&link) {
            return (Some(link), typed);
        }
        if let Some(question) = link["sign_in"]["question"].as_u64()
            && answered != Some(question)
        {
            assert_eq!(link["sign_in"]["kind"], "secret", "{what}: {link}");
            window
                .send(
                    "core_link",
                    json!({"action": "sign_in", "question": question, "reply": "text",
                        "text": PASSPHRASE}),
                )
                .await;
            typed += 1;
            answered = Some(question);
        }
        match link["phase"].as_str() {
            Some("stopped") if !pressed => {
                window.send("core_link", json!({"action": "retry"})).await;
                pressed = true;
            }
            Some("stopped") => {}
            _ => pressed = false,
        }
    }
}

/// The window of `machine`'s Hide.
async fn window_of(machine: &Machine) -> Window {
    let screen = machine.screen();
    Window::open_at(screen.port, &screen.token, "web").await
}

/// The first address of a pane on the node, anywhere in what the core's
/// screen is told.
fn pane_on_the_node(told: &Value) -> Option<String> {
    let prefix = format!("remote:{ORIGINAL}:pane:");
    match told {
        Value::String(text) => text.starts_with(&prefix).then(|| text.clone()),
        Value::Array(items) => items.iter().find_map(pane_on_the_node),
        Value::Object(fields) => fields.values().find_map(pane_on_the_node),
        _ => None,
    }
}

/// Ends the holders of the folder a node ran its panes in, which outlive
/// the node by design.
struct EndsHolders(PathBuf);

impl Drop for EndsHolders {
    fn drop(&mut self) {
        if let Err(error) = end_holders(&self.0) {
            eprintln!("the holders of {} did not end: {error:#}", self.0.display());
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "two daemons, the system's sshd, a shipped build and a pane's holder; the remote mailbox lane runs it"]
async fn a_core_moved_before_1_2_0_becomes_two_hides_and_its_work_goes_on() {
    // A node's endpoints and its panes' sockets sit below these folders,
    // and a Unix socket address holds about a hundred bytes.
    let root = crate::fixtures::fixture_root("hm");
    let at = |name: &str| root.path().join(name);
    let environment = Environment::new(&at("e"), None).unwrap();
    let built = Path::new(env!("CARGO_BIN_EXE_hided")).parent().unwrap();
    std::fs::create_dir(at("p")).unwrap();
    for name in PROGRAMS {
        assert!(
            built.join(name).is_file(),
            "build this worktree's programs first (scripts/verify-cargo.sh cli): no {name}"
        );
        shipped::place(&built.join(name), &at("p").join(name), &environment).unwrap();
    }
    // The build the core ran when it was moved: the same code under other
    // bytes, so it is another build to every machine that reads it. The
    // build as cargo left it, with what a shipped copy has stripped.
    // Named as the machine itself names it, links followed: an update puts
    // the next build beside this one, in the install both are builds of.
    std::fs::create_dir(at("i")).unwrap();
    let old = at("i").canonicalize().unwrap().join(OLD_BUILD);
    std::fs::create_dir(&old).unwrap();
    for name in PROGRAMS {
        let source = match name {
            "hided" => built.join(name),
            _ => at("p").join(name),
        };
        // A byte copy, never an APFS clone (docs/TESTING.md, Writing a Rust
        // test, rule 8).
        std::io::copy(
            &mut std::fs::File::open(&source).unwrap(),
            &mut std::fs::File::create_new(old.join(name)).unwrap(),
        )
        .unwrap();
        std::fs::set_permissions(
            old.join(name),
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
    }
    let checkout = |folder: &Path| {
        std::fs::create_dir_all(folder).unwrap();
        let mut init = environment.command("/usr/bin/git");
        init.args(["-c", "init.defaultBranch=main", "-C"])
            .arg(folder)
            .args(["init", "-q"]);
        successful(init).unwrap();
    };
    // `project` is the folder of the machine's own project in its home.
    let machine = |home: &str,
                   state: &str,
                   id: &'static str,
                   hide: PathBuf,
                   direct: String,
                   project: &str| {
        hide_platform::fs::private::create_dir_all(&at(state)).unwrap();
        checkout(&at(home).join(project));
        // A pane's shell starts with the account's own files, never a first
        // run's questions.
        std::fs::write(at(home).join(".zshrc"), "PS1='moved-core> '\n").unwrap();
        // The agent the account runs is the fixture's stand-in, found on its
        // login shell's `PATH` before its daemon first reads that.
        place_stand_in(&environment, &at("e"), &at(home)).unwrap();
        Machine {
            // A fixture HOME: a paired core keeps no login item under it.
            terminal: Terminal::new(at(home), at(state), Some(id)),
            hide,
            direct,
            project: at(home).join(project),
            // The stand-in is found through the account's shell files
            // (`place_stand_in`), so the terminal's own values are enough.
            account: Vec::new(),
        }
    };
    // Declared before the machines, so it ends the holders after their
    // daemons stopped.
    let _holders = EndsHolders(hide_kit::layout::device_node_state(&at("mh")));
    let core_direct = format!("127.0.0.1:{}", free_port());
    // The core's own project is where its lead agent works; the original
    // machine's `project` is the core's work there, and `own` its own once
    // it is a core.
    let mut core = machine(
        "ch",
        "c",
        CORE,
        old.join("hide"),
        core_direct.clone(),
        "project",
    );
    let mut original = machine(
        "mh",
        "m",
        ORIGINAL,
        at("p").join("hide"),
        NO_ADDRESS.to_owned(),
        "own",
    );

    // What the move left on each machine. The core's: the machine it was
    // moved off, registered as a node that dials in.
    moved_off_dials_in(&core.terminal.state, ORIGINAL);
    // The original machine's: the record that names its core, and beside it
    // what it held before the move, which nothing here may touch.
    let placement = || {
        hided::placement::read(&original.terminal.state, ORIGINAL)
            .unwrap()
            .expect("the original machine's record of its core")
    };
    hided::placement::write(
        &original.terminal.state,
        &hided::placement::Placement {
            alias: ALIAS.to_owned(),
            node: CORE.to_owned(),
            program: old.join("hided").to_string_lossy().into_owned(),
            state_dir: Some(core.terminal.state.to_string_lossy().into_owned()),
            disconnected: false,
        },
    )
    .unwrap();
    let kept = hide_kit::layout::moved_out(&original.terminal.state);
    std::fs::create_dir(&kept).unwrap();
    std::fs::write(kept.join("core-state.json"), b"what the move kept").unwrap();

    // The saved SSH login: an account of the core's machine behind a real
    // server, signed in to with a key that asks for its passphrase.
    let session: Vec<(std::ffi::OsString, std::ffi::OsString)> = [
        (hided::env::HOME, at("ch").into_os_string()),
        ("HIDE_STATE_DIR", at("c").into_os_string()),
        (hided::env::HIDE_MACHINE_ID, CORE.into()),
        ("HIDE_PEER_DIRECT", core_direct.clone().into()),
        (hided::env::HIDE_DEVICE_CONNECT, "1".into()),
        // An update over this login starts the core itself, as the login
        // item does on the account's own home.
        (hided::env::HIDE_CORE_STARTER, "fixture".into()),
        ("HIDE_IDLE_SECS", "3600".into()),
        (
            "HIDE_TAILSCALE_BIN",
            at("ch").join("no-tailscale").into_os_string(),
        ),
        ("HIDE_RELAY_URL", "ws://127.0.0.1:9/ws".into()),
        (
            "XDG_CONFIG_HOME",
            at("ch").join("xdg-config").into_os_string(),
        ),
    ]
    .into_iter()
    .map(|(name, value)| (name.into(), value))
    .collect();
    let ssh = Sshd::start(&at("s"), &session).unwrap();
    let locked = at("k");
    let made = std::process::Command::new("/usr/bin/ssh-keygen")
        .args(["-q", "-t", "ed25519", "-C", "moved", "-N", PASSPHRASE, "-f"])
        .arg(&locked)
        .status()
        .unwrap();
    assert!(made.success());
    ssh.authorize(&locked.with_extension("pub")).unwrap();
    std::fs::create_dir_all(original.terminal.home.join(".ssh")).unwrap();
    std::fs::write(
        original.terminal.home.join(".ssh/config"),
        ssh.client_config(ALIAS).replace(
            &ssh.client_key.display().to_string(),
            &locked.display().to_string(),
        ),
    )
    .unwrap();

    // The core's key is its machine's, made before its daemon first reads
    // it.
    let key = hide_peer::key::MachineKey::load_or_create(
        &hide_kit::layout::machine_key(&core.terminal.state),
        CORE,
    )
    .unwrap()
    .0
    .public();

    // The core runs the build it was moved under.
    let started = core
        .command(&["connect"])
        .env("HIDE_BUILD_VERSION_OVERRIDE", OLD_VERSION)
        .env("HIDE_BUILD_ORDER_OVERRIDE", "1")
        .output()
        .unwrap();
    assert!(
        started.status.success(),
        "the moved core: {}",
        String::from_utf8_lossy(&started.stderr)
    );
    assert_eq!(
        health(core.screen().port).unwrap()["release"]["version"],
        OLD_VERSION
    );
    core.opens_its_project();

    // B45, the update first: the original machine starts on this build.
    // Nobody pressed for its first dial, so the key's passphrase stops it;
    // the operator's Retry finds the older core, and the next one updates
    // it over the saved login. The split is asked only of the updated
    // core, on the session that update signed in, and stops where the new
    // connection cannot be made: the machine links as the node it was, on
    // that same session, and says why.
    original.ran(&["connect"]);
    let mut window = window_of(&original).await;
    let (live, typed) = at_the_window(&mut window, "the link to the updated core", |link| {
        link["phase"] == "live"
    })
    .await;
    let live = live.expect("the window stays while the machine is a node");
    assert_eq!(live["split_failed"], "split_not_connected", "{live}");
    assert_eq!(typed, 2, "one sign-in found the older core, one updated it");
    let updated = PathBuf::from(placement().program);
    assert_ne!(updated, old.join("hided"));
    core.hide = updated.with_file_name("hide");
    // The update put the whole build there, the pane holder with the
    // programs: the core runs its own panes from it, and it is the build
    // the original machine starts the core's node from once that machine is
    // a core itself. The journey places it there, as every journey between
    // two Hides does: a development build is larger than a build one
    // machine sends another.
    for name in PROGRAMS {
        assert!(
            updated.with_file_name(name).is_file(),
            "the update left no {name}"
        );
    }
    place_build(updated.parent().unwrap(), &original.terminal.state, &key);
    let release = health(core.screen().port).unwrap()["release"].clone();
    assert_ne!(release["version"], OLD_VERSION, "{release}");
    assert!(!kept.join("core-placement.json").exists());
    // Whether the core's screen shows the original machine as a node that
    // dials in.
    let dials_in = || {
        let shown = core.screen().snapshot().ok()?;
        shown
            .pointer("/navigator/devices")?
            .as_array()?
            .iter()
            .find(|device| device["id"] == ORIGINAL)?
            .get("dials_in")?
            .as_bool()
    };
    let still_its_node = |when: &str| {
        core.lists(when, |listed| {
            let peer = listed["peers"].as_array()?.first()?;
            (peer["opens"] == "confirming" && dials_in() == Some(true)).then_some(())
        })
    };
    still_its_node("the node its core still takes");

    // K16: the operator ends the link at the original machine's window.
    // The choice is in the record, so the daemon that starts anew dials
    // nothing, and their reconnect takes it out again.
    window
        .send("core_link", json!({"action": "disconnect"}))
        .await;
    wait_within("the record of the operator's disconnect", WITHIN, || {
        Ok(placement().disconnected.then_some(()))
    })
    .unwrap();
    drop(window);
    original.ran(&["stop"]);
    original.ran(&["connect"]);
    assert_eq!(
        health(original.screen().port).unwrap()["core_link"],
        "disconnected"
    );
    assert!(placement().disconnected);
    let mut window = window_of(&original).await;
    let (ended, _) = at_the_window(&mut window, "the link the operator ended", |link| {
        link["phase"] == "disconnected"
    })
    .await;
    assert!(ended.is_some());
    window
        .send("core_link", json!({"action": "reconnect"}))
        .await;
    // One sign-in carries the split's asks and, when it stops again, the
    // link: nothing asks a second time.
    let (live, typed) = at_the_window(&mut window, "the link the operator restored", |link| {
        link["phase"] == "live"
    })
    .await;
    let live = live.expect("the window stays while the machine is a node");
    assert_eq!(live["split_failed"], "split_not_connected", "{live}");
    assert_eq!(typed, 1);
    assert!(!placement().disconnected);
    still_its_node("the node after its reconnect");

    // The moved core's work on the original machine, while that machine is
    // still its node: a terminal, an agent of the core, a letter it sent and
    // a mission of the core. Folders are named as each machine names them,
    // links followed, which is how a core lists a checkout.
    let home_of = |home: &str| at(home).canonicalize().unwrap();
    let project = home_of("mh").join("project");
    checkout(&project);
    let screen = core.screen();
    let mut last = Value::Null;
    wait_within("the node's machine, ready for work", WITHIN, || {
        last = screen.snapshot()?;
        let ready = last
            .pointer("/navigator/devices")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|row| row["id"] == ORIGINAL && row["host"]["state"] == "ready");
        Ok(ready.then_some(()))
    })
    .unwrap_or_else(|error| panic!("{error:#}: {}", last["navigator"]["devices"]));
    screen
        .event(
            "create_workspace",
            json!({"device_id": ORIGINAL, "path": project, "label": "On the laptop",
                "initialize_git": false}),
        )
        .unwrap();
    let pane = wait_within("the core's pane on its node", WITHIN, || {
        Ok(pane_on_the_node(&screen.snapshot()?))
    })
    .unwrap();
    // The line that leaves the shell's own process id: the same id later is
    // the same shell, in the same holder, whatever started anew around it.
    let itself = |file: &str| format!("echo $$ > {}", at(file).display());
    types(&core, &pane, &itself("f1")).await;
    let shell = left(&at("f1"));

    // The core's two agents, each the account's stand-in in a pane of its
    // own, registered from inside it as an agent registers itself: the lead
    // on the core's machine, the worker on the original one.
    let commands = |name: &str| {
        let folder = at("r").join(name);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    };
    let own_hide = at("p").join("hide");
    let mut lead = Agent::start(
        &screen,
        commands("l"),
        Environment::of_home(&home_of("ch")),
        None,
        core.hide.clone(),
        core.hide.with_file_name("hide-agent-hooks"),
        "moved-lead-session",
        "moved-lead",
        // The core's own engine is named by its folder in the daemon's
        // state (`hide_kit::layout::engine_dir`).
        &core.terminal.state.join("engine").to_string_lossy(),
    )
    .unwrap();
    let mut worker = Agent::start(
        &screen,
        commands("w"),
        Environment::of_home(&home_of("mh")),
        Some(ORIGINAL),
        own_hide.clone(),
        own_hide.with_file_name("hide-agent-hooks"),
        "moved-worker-session",
        "moved-worker",
        // A device's agents are scoped by the device.
        ORIGINAL,
    )
    .unwrap();
    assert!(
        worker.pane.starts_with(&format!("remote:{ORIGINAL}:pane:")),
        "{}",
        worker.pane
    );
    let created = lead
        .run_in_pane(&format!(
            "{} mission create --title Kept",
            quote(&core.hide)
        ))
        .unwrap()
        .json()
        .unwrap();
    let mission = created["mission"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("the mission's answer names it: {created}"))
        .to_owned();
    let sends = |intent: &str, body: &str| {
        format!(
            "{} request send moved-lead --intent {intent} --body {}",
            quote(&own_hide),
            quote(body)
        )
    };
    let before = worker
        .run_in_pane(&sends("before-the-split", "sent before the split"))
        .unwrap()
        .json()
        .unwrap();

    // The connection can be made: the daemon starts anew with an address it
    // can bind. One sign-in at its window carries the whole split, and the
    // window's connection ends with the node role, since the machine is its
    // own core from here on.
    drop(window);
    original.ran(&["stop"]);
    original.direct = format!("127.0.0.1:{}", free_port());
    original.ran(&["connect"]);
    let mut window = window_of(&original).await;
    let (stayed, typed) = at_the_window(&mut window, "the split", |_| false).await;
    assert!(stayed.is_none());
    assert_eq!(typed, 1);
    drop(window);
    health_becomes(&original, "the original machine's own core", |health| {
        health["role"] == "core"
    });

    // What the move left is beside the new core's state, with the record
    // that named the core, and nothing of it was changed.
    assert!(
        hided::placement::read(&original.terminal.state, ORIGINAL)
            .unwrap()
            .is_none()
    );
    assert!(kept.join("core-placement.json").is_file());
    assert_eq!(
        std::fs::read(kept.join("core-state.json")).unwrap(),
        b"what the move kept"
    );

    // The two are paired both ways, and on the machine that kept the moved
    // core the node it took over SSH is the paired machine's device, under
    // the id it had.
    original.opens_its_project();
    original.lists("the original machine opens the core's", |listed| {
        let peer = listed["peers"].as_array()?.first()?;
        (peer["id"] == CORE
            && peer["opens"] == "active"
            && peer["opened_by"] == "active"
            && peer["connected"] == true)
            .then_some(())
    });
    core.lists("the core's machine opens the original", |listed| {
        let peer = listed["peers"].as_array()?.first()?;
        (peer["id"] == ORIGINAL
            && peer["opens"] == "active"
            && peer["opened_by"] == "active"
            && peer["connected"] == true
            && dials_in() == Some(false))
        .then_some(())
    });

    // The core's node on the original machine starts from the core's build
    // there and takes up the panes it held: the row says ready again, on
    // the connection the two Hides made.
    let mut last = Value::Null;
    wait_within("the core's node on the original machine", WITHIN, || {
        last = screen.snapshot()?;
        let ready = last
            .pointer("/navigator/devices")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|row| {
                row["id"] == ORIGINAL && row["dials_in"] == false && row["host"]["state"] == "ready"
            });
        Ok(ready.then_some(()))
    })
    .unwrap_or_else(|error| panic!("{error:#}: {}", last["navigator"]["devices"]));

    // The terminal is the one it was: the moved core types into the same
    // shell, under the address it had, through its own node on the
    // original machine, and that machine lists it as the core's work. The
    // operator brings its tab forward first, since the worker's took the
    // front when its agent started and a pane is drawn for the tab in
    // front.
    screen
        .event(
            "remote_control",
            json!({"target_id": ORIGINAL, "request_id": "the-shell", "focus_device": true,
                "action": "focus_pane", "pane_id": pane}),
        )
        .unwrap();
    types(&core, &pane, &itself("f2")).await;
    assert_eq!(left(&at("f2")), shell);
    let local = pane
        .strip_prefix(&format!("remote:{ORIGINAL}:pane:"))
        .unwrap();
    let as_guest = |pane: &str| format!("guest:{CORE}:pane:{pane}");
    let worker_here = worker
        .pane
        .strip_prefix(&format!("remote:{ORIGINAL}:pane:"))
        .unwrap()
        .to_owned();
    // Both of the core's panes here: the shell and the worker's.
    let listed_here = original.lists("the core's work on the original machine", |listed| {
        let panes: Vec<String> = listed["guest_panes"]
            .as_array()?
            .iter()
            .filter(|guest| guest["id"] == CORE)
            .flat_map(|guest| guest["panes"].as_array().cloned().unwrap_or_default())
            .map(|pane| pane["id"].as_str().unwrap().to_owned())
            .collect();
        [local, worker_here.as_str()]
            .iter()
            .all(|pane| panes.contains(&as_guest(pane)))
            .then(|| as_guest(local))
    });
    types(&original, &listed_here, &itself("f3")).await;
    assert_eq!(left(&at("f3")), shell);

    // The worker is the moved core's agent as before: the same stand-in in
    // the same pane, whose own `hide` still reaches that core, which knows
    // it by the registration it had, takes its next letter, and holds the
    // letter and the mission from before.
    let agents = worker
        .run_in_pane(&format!("{} agent list", quote(&own_hide)))
        .unwrap();
    assert_eq!(agents.status, 0, "{} {}", agents.stdout, agents.stderr);
    assert!(
        agents.stdout.contains(&worker.registration),
        "{}",
        agents.stdout
    );
    let after = worker
        .run_in_pane(&sends("after-the-split", "sent after the split"))
        .unwrap()
        .json()
        .unwrap();
    assert_ne!(after, before);
    let inbox = lead
        .run_in_pane(&format!("{} inbox", quote(&core.hide)))
        .unwrap()
        .json()
        .unwrap()
        .to_string();
    for letter in [&before, &after] {
        let id = letter["id"].as_str().expect("the letter's id");
        assert!(
            inbox.contains(id),
            "the lead's inbox holds no {id}: {inbox}"
        );
    }
    let missions = lead
        .run_in_pane(&format!("{} mission list", quote(&core.hide)))
        .unwrap()
        .json()
        .unwrap();
    assert!(missions.to_string().contains(&mission), "{missions}");

    drop(original);
    drop(core);
    ssh.stop().unwrap();
}
