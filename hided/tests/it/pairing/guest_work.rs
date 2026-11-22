//! One Hide's work on another, with both cores real (PRD device-connect-iroh
//! c1; B9, B10, B12, B14 to B16, B54, B60, D-55, D-74): two daemons of a shipped copy of this
//! build, each a machine of its own, pair by a link, and the guest opens a
//! folder of its host. The work runs in the guest's node, the shipped
//! `hided` the host starts from the guest's install root, and this test
//! reads it the way each operator would: `hide device list` at a terminal
//! in a project of the machine, a screen of each machine's Hide that draws
//! the pane and types into it, and the files a typed command leaves.
//!
//! The guest's build is placed in its install root by this test and not
//! sent over the connection: a development build's programs are larger than
//! a build may be (`hide_peer_protocol::build::MAX_BYTES`, a release kit's
//! size), so a host refuses them. The host placing a build a guest sends is
//! `hided::peer_dial`'s and `hided::guests`' own tests'.
//!
//! A guest's hook entries on its host's account are not this test's: a core
//! sends a machine its agents' pieces once its own first-run choice is
//! answered, which a daemon outside the app never is. `hide-host`'s kit
//! tests, `hide-agent-hooks`' `other_install` and `hided::guests`' own tests
//! hold what they write, where and until when.
//!
//! The two endpoints bind loopback with no relay (`HIDE_PEER_DIRECT`), the
//! host on a port this test found free, so the guest finds it again after
//! either daemon starts anew.

use std::ffi::OsString;
use std::io::Write as _;
use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use base64::Engine as _;
use serde_json::{Value, json};

use super::{GUEST, Ran, Terminal, Window};
use crate::support::remote_delivery::{
    Daemon, Environment, end_holders, read, successful, wait_within,
};
use crate::support::shipped;

/// The machine id the host daemon runs under.
const HOST: &str = "guest-work-host";

/// The programs a build is, which a guest's node, its panes' `hide`, its
/// hooks and its pane holders run from.
pub(super) const PROGRAMS: [&str; 4] = ["hided", "hide", "hide-agent-hooks", "hide-holder"];

/// How long one step may take: a daemon's start, a node's start from a
/// program the system checks first, a pane's shell, a dial after a restart.
pub(super) const WITHIN: Duration = Duration::from_secs(90);

/// One of the two machines: its account, its state folder and the shipped
/// `hide` at its terminal, which starts and stops its daemon.
pub(super) struct Machine {
    pub(super) terminal: Terminal,
    pub(super) hide: PathBuf,
    /// Where its endpoint for other Hides binds.
    pub(super) direct: String,
    /// The folder its terminal is in: a project of its own, since a pane is
    /// listed and typed into for a caller in a project Hide has
    /// (`hided::pane_auth`).
    pub(super) project: PathBuf,
    /// What the account's sessions set beyond the terminal's own: the login
    /// shell and `PATH` of a machine whose panes run an agent, which its
    /// daemon starts them from.
    pub(super) account: Vec<(OsString, OsString)>,
}

impl Machine {
    pub(super) fn command(&self, args: &[&str]) -> Command {
        let mut command = self.terminal.command_of(&self.hide, args);
        command
            .env("HIDE_PEER_DIRECT", &self.direct)
            // The daemon stays while this test's terminal is away from it.
            .env("HIDE_IDLE_SECS", "3600")
            .envs(self.account.iter().map(|(name, value)| (name, value)))
            .current_dir(&self.project);
        command
    }

    pub(super) fn run(&self, args: &[&str]) -> Ran {
        Ran::of(self.command(args).output().unwrap())
    }

    /// Runs `hide args` and has it succeed.
    pub(super) fn ran(&self, args: &[&str]) -> Ran {
        let ran = self.run(args);
        assert!(ran.ok, "hide {args:?}: {}", ran.text());
        ran
    }

    /// The daemon's screen connection.
    pub(super) fn screen(&self) -> Daemon {
        let state: Value =
            serde_json::from_slice(&read(&self.terminal.state.join("hided.json")).unwrap())
                .unwrap();
        Daemon {
            port: state["port"].as_u64().unwrap() as u16,
            token: state["token"].as_str().unwrap().to_owned(),
        }
    }

    /// Has this machine's Hide open the project its terminal is in.
    pub(super) fn opens_its_project(&self) {
        let (screen, path) = (self.screen(), self.project.to_string_lossy().into_owned());
        screen
            .event(
                "create_workspace",
                json!({"path": path, "label": "Its own", "initialize_git": false}),
            )
            .unwrap();
        // The core answers the list for a caller in a project it has; any
        // other caller gets the paired machines alone, from the daemon.
        self.lists("the machine's own project", |listed| {
            listed.get("guest_panes").map(|_| ())
        });
    }

    /// The pane of this machine's own project, as `hide ps` lists it.
    pub(super) fn own_pane(&self) -> String {
        let project = self.project.to_string_lossy().into_owned();
        let mut last = Value::Null;
        wait_within("the pane of the machine's own project", WITHIN, || {
            last = self.ran(&["ps", "--json"]).json()["value"].clone();
            Ok(last["panes"].as_array().and_then(|panes| {
                let own = panes.iter().find(|pane| {
                    pane["checkout_path"]
                        .as_str()
                        .is_some_and(|path| path.ends_with(project.trim_start_matches("/private")))
                })?;
                Some(own["pane"].as_str()?.to_owned())
            }))
        })
        .unwrap_or_else(|error| panic!("{error:#}: {last}"))
    }

    /// What `hide device list` says at this machine's terminal.
    pub(super) fn listed(&self) -> Value {
        self.ran(&["device", "list", "--json"]).json()["value"].clone()
    }

    /// Waits until `hide device list` here is as `wanted` reads it.
    pub(super) fn lists<T>(&self, what: &str, wanted: impl Fn(&Value) -> Option<T>) -> T {
        let mut last = Value::Null;
        wait_within(what, WITHIN, || {
            last = self.listed();
            Ok(wanted(&last))
        })
        .unwrap_or_else(|error| panic!("{error:#}: {last}"))
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = self.command(&["stop"]).output();
        // A pane's holder outlives its node by design, so whoever had the
        // node started ends it: the pane of this machine's own project and
        // every pane a guest's node left here.
        let guests = hide_kit::layout::guests(&self.terminal.state);
        let nodes = std::fs::read_dir(guests)
            .into_iter()
            .flatten()
            .flatten()
            .map(|guest| guest.path().join(hide_peer::protocol::launch::NODE_SIDE));
        for node in std::iter::once(self.terminal.state.clone()).chain(nodes) {
            if let Err(error) = end_holders(&node) {
                eprintln!("the holders of {} did not end: {error:#}", node.display());
            }
        }
        if !std::thread::panicking() {
            return;
        }
        // A failed journey says what each daemon recorded, without the line
        // each of this test's own looks at the list left there.
        let logs = self.terminal.state.join("Logs");
        for entry in std::fs::read_dir(&logs).into_iter().flatten().flatten() {
            let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
            let lines: Vec<&str> = text
                .lines()
                .filter(|line| !line.contains("checkout_capability.refused"))
                .collect();
            let from = lines.len().saturating_sub(400);
            eprintln!(
                "--- {} ---\n{}",
                entry.path().display(),
                lines[from..].join("\n")
            );
        }
    }
}

/// A port of loopback no one holds now, for an endpoint that is found again
/// at the same address after its daemon starts anew.
pub(super) fn free_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Puts the shipped build in `programs` where a host of `state` keeps the
/// builds of the guest `key`; answers the build's name.
pub(super) fn place_build(
    programs: &Path,
    state: &Path,
    key: &hide_peer::protocol::PeerKey,
) -> String {
    shipped::place_guest_build(programs, &hide_kit::layout::guest_install_root(state, key))
}

/// Pairs the two by the host's link, both operators saying yes, and answers
/// the id the guest lists its host under.
pub(super) async fn pair(host: &Machine, guest: &Machine) -> String {
    let screen = host.screen();
    let mut window = Window::open_at(screen.port, &screen.token, "web").await;
    window
        .send(
            "pairing_share",
            json!({"request_id": "s1", "language": "en"}),
        )
        .await;
    let shared = window.result("share").await;
    assert_eq!(shared["ok"], true, "{shared}");
    let mut adding = guest.command(&["device", "add", shared["link"].as_str().unwrap()]);
    adding.stdin(Stdio::piped());
    let adding = tokio::task::spawn_blocking(move || {
        let mut child = adding.spawn().unwrap();
        child.stdin.take().unwrap().write_all(b"y\n").unwrap();
        Ran::of(child.wait_with_output().unwrap())
    });
    let asked = window
        .frame("the guest's ask", |frame| {
            frame["type"] == "pairing"
                && frame["payload"]["asks"]
                    .as_array()
                    .is_some_and(|asks| !asks.is_empty())
        })
        .await;
    window
        .send(
            "pairing_answer",
            json!({"request_id": "a1", "ask": asked["payload"]["asks"][0]["id"], "allow": true}),
        )
        .await;
    assert_eq!(window.result("answer").await["ok"], true);
    let added = adding.await.unwrap();
    assert!(added.ok, "{}", added.text());
    guest.lists("the guest opens its host", |listed| {
        let peer = listed["peers"].as_array()?.first()?;
        (peer["opens"] == "active" && peer["connected"] == true)
            .then(|| peer["id"].as_str().unwrap().to_owned())
    })
}

/// The panes of the guest's work as the host's terminal lists them, by the
/// address the host types into them at.
fn guest_panes(listed: &Value) -> Vec<String> {
    listed["guest_panes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|guest| guest["id"] == GUEST)
        .flat_map(|guest| guest["panes"].as_array().cloned().unwrap_or_default())
        .map(|pane| pane["id"].as_str().unwrap().to_owned())
        .collect()
}

/// Waits for the file a command typed into a pane leaves, and reads it.
pub(super) fn left(file: &Path) -> String {
    wait_within(&format!("{} from the pane", file.display()), WITHIN, || {
        Ok(std::fs::read_to_string(file)
            .ok()
            .filter(|text| !text.is_empty()))
    })
    .unwrap()
}

/// The grid a screen of either machine draws a pane at.
const GRID: (u16, u16) = (31, 97);

/// Types `line` and Enter into `pane` as a screen of `machine`'s Hide does
/// at [`GRID`].
pub(super) async fn types(machine: &Machine, pane: &str, line: &str) {
    types_at(machine, pane, line, GRID).await;
}

/// Types `line` and Enter into `pane` as a screen of `machine`'s Hide does:
/// it shows the pane at `grid` (rows, columns), has it drawn whole, and
/// sends the keys.
pub(super) async fn types_at(machine: &Machine, pane: &str, line: &str, grid: (u16, u16)) {
    let screen = machine.screen();
    let mut window = Window::open_at(screen.port, &screen.token, "web").await;
    // A pane on another machine is drawn while that machine is the one in
    // front, which choosing it in the rail makes it.
    if let Some(device) = pane
        .strip_prefix("remote:")
        .and_then(|rest| rest.split_once(":pane:"))
    {
        window
            .send("focus_device", json!({"device_id": device.0}))
            .await;
    }
    window
        .send(
            "terminal_viewport",
            json!({"pane_id": pane, "rows": grid.0, "cols": grid.1, "new_view": true}),
        )
        .await;
    window
        .frame("the pane drawn on the screen", |frame| {
            frame["type"] == "terminal"
                && frame["payload"]["chunks"]
                    .as_array()
                    .is_some_and(|chunks| chunks.iter().any(|chunk| chunk["pane_id"] == pane))
        })
        .await;
    let keys = base64::engine::general_purpose::STANDARD.encode(format!("{line}\r"));
    window
        .send("key", json!({"pane_id": pane, "bytes_base64": keys}))
        .await;
}

/// The process whose command line names `words`, each of them.
pub(super) fn process_naming(words: &[&str]) -> Option<u32> {
    let listing = Command::new("ps")
        .args(["-Ao", "pid=,command="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&listing.stdout)
        .lines()
        .filter(|line| words.iter().all(|word| line.contains(word)))
        .find_map(|line| line.split_whitespace().next()?.parse().ok())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "two daemons, a shipped build and a pane's holder; the remote mailbox lane runs it"]
async fn a_guests_work_runs_on_its_host_from_the_guests_build_and_outlives_each_core() {
    // A node's endpoints and its panes' sockets sit below the host's state
    // folder, and a Unix socket address holds about a hundred bytes.
    let root = crate::fixtures::fixture_root("hg");
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
    // A folder with a checkout of its own is what opens with a terminal.
    let checkout = |folder: &Path| {
        std::fs::create_dir_all(folder).unwrap();
        let mut init = environment.command("/usr/bin/git");
        init.args(["-c", "init.defaultBranch=main", "-C"])
            .arg(folder)
            .args(["init", "-q"]);
        successful(init).unwrap();
    };
    let machine = |home: &str, state: &str, id: &'static str, direct: String| {
        hide_platform::fs::private::create_dir_all(&at(state)).unwrap();
        checkout(&at(home).join("own"));
        // A pane's shell starts with the account's own files, never a first
        // run's questions.
        std::fs::write(at(home).join(".zshrc"), "PS1='guest-work> '\n").unwrap();
        Machine {
            // A fixture HOME: a paired core keeps no login item under it.
            terminal: Terminal::new(at(home), at(state), Some(id)),
            hide: at("p").join("hide"),
            direct,
            project: at(home).join("own"),
            account: Vec::new(),
        }
    };
    let host = machine("hh", "h", HOST, format!("127.0.0.1:{}", free_port()));
    let guest = machine("gh", "g", GUEST, "127.0.0.1:0".to_owned());

    // The guest's key is its machine's, made before its daemon first reads
    // it, so its build is in its install root before it first asks for its
    // node.
    let key = hide_peer::key::MachineKey::load_or_create(
        &hide_kit::layout::machine_key(&guest.terminal.state),
        GUEST,
    )
    .unwrap()
    .0
    .public();
    host.ran(&["connect"]);
    guest.ran(&["connect"]);
    host.opens_its_project();
    guest.opens_its_project();
    let build = place_build(&at("p"), &host.terminal.state, &key);
    let host_id = pair(&host, &guest).await;
    assert_eq!(host_id, HOST);

    // B9: the guest opens a folder of its host, and the work is a pane of
    // the guest's node there.
    let project = at("hh").join("project");
    checkout(&project);
    let screen = guest.screen();
    // The host is a device of the guest's core once its node there answers.
    let mut last = Value::Null;
    wait_within("the guest's node on its host", WITHIN, || {
        last = screen.snapshot()?;
        let ready = last
            .pointer("/navigator/devices")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|row| row["id"] == host_id.as_str() && row["host"]["state"] == "ready");
        Ok(ready.then_some(()))
    })
    .unwrap_or_else(|error| panic!("{error:#}: {}", last["navigator"]["devices"]));
    screen
        .event(
            "create_workspace",
            json!({"device_id": host_id, "path": project, "label": "On the host",
                "initialize_git": false}),
        )
        .unwrap();
    // B10: the host's terminal lists it as the guest's.
    let pane = host.lists("the guest's pane on its host", |listed| {
        guest_panes(listed).into_iter().next()
    });
    assert!(pane.starts_with(&format!("guest:{GUEST}:pane:")), "{pane}");
    let local = pane
        .strip_prefix(&format!("guest:{GUEST}:pane:"))
        .unwrap()
        .to_owned();
    let on_guest = format!("remote:{HOST}:pane:{local}");
    let node_runs = || process_naming(&[build.as_str(), "node", "serve", "--guest"]);
    let node = node_runs().expect("the guest's node, a process of the guest's build");
    // The line that leaves the shell's own process id: the same id later is
    // the same shell, whatever started anew around it.
    let itself = |file: &str| format!("echo $$ > {}", at(file).display());
    let asks_its_core = |file: &str| {
        format!(
            "\"$HIDE_INSTALL_CLI\" ps --json > {} 2>&1",
            at(file).display()
        )
    };

    // B10: a screen of the host's Hide draws the pane and types into it
    // through the node's tap: the pane's terminal is the screen's grid, and
    // the keys are the shell's input.
    types(&host, &pane, &itself("f1")).await;
    let shell = left(&at("f1"));
    types(&host, &pane, &format!("stty size > {}", at("f9").display())).await;
    assert_eq!(
        left(&at("f9")).trim(),
        format!("{} {}", GRID.0, GRID.1).as_str()
    );
    // D-77 (5): `hide pane send` in a pane of the host types into it too.
    let sends = format!(
        "{} pane send '{pane}' '{}' --enter",
        host.hide.display(),
        itself("f10")
    );
    types(&host, &host.own_pane(), &sends).await;
    assert_eq!(left(&at("f10")), shell);
    // B9: the guest's operator types into it through the guest's own core.
    types(&guest, &on_guest, &itself("f2")).await;
    assert_eq!(left(&at("f2")), shell);
    // B11: a `hide` in the pane is the guest's own, and its core answers.
    types(&guest, &on_guest, &asks_its_core("f3")).await;
    let answered = left(&at("f3"));
    assert!(answered.contains("\"ok\":true"), "{answered}");
    // B10, D-74: the pane has one grid, the one the screen that sized it
    // last draws at. The guest's screen at its own grid sizes the program
    // to it; the host's screen drawing it next sizes it back, so the
    // program draws at the grid the host shows and its frames are not
    // refused as the wrong size.
    let size_into = |file: &str| format!("stty size > {}", at(file).display());
    types_at(&guest, &on_guest, &size_into("f11"), (27, 88)).await;
    assert_eq!(left(&at("f11")).trim(), "27 88");
    types(&host, &pane, &size_into("f12")).await;
    assert_eq!(
        left(&at("f12")).trim(),
        format!("{} {}", GRID.0, GRID.1).as_str()
    );
    // B10: the host's screen says which folder the pane was started in, as
    // the guest's node told it, so the pane stands under the project there.
    let folder = project.to_string_lossy().into_owned();
    let in_folder = |path: &Value| {
        path.as_str()
            .is_some_and(|path| path.ends_with(folder.trim_start_matches("/private")))
    };
    let mut shown = Value::Null;
    wait_within("the pane's folder on the host's screen", WITHIN, || {
        shown = host.screen().snapshot()?["navigator"]["guest_panes"].clone();
        Ok(in_folder(&shown[0]["panes"][0]["folder"]).then_some(()))
    })
    .unwrap_or_else(|error| panic!("{error:#}: {shown}"));
    // D-50: the guest may open the host, so the host's core tells it its
    // own Needs You and Done on the stream the two cores talk on, and the
    // guest's screen holds them under the host. Nothing waits on the host,
    // so the counts are zero; the host, which may not open the guest, is
    // told nothing of the guest's.
    let mut told = Value::Null;
    wait_within("the host's own work on the guest's screen", WITHIN, || {
        told = guest.screen().snapshot()?["navigator"]["peer_work"].clone();
        Ok((told[0]["key"].is_string() && told[0]["needs_you"] == 0).then_some(()))
    })
    .unwrap_or_else(|error| panic!("{error:#}: {told}"));
    let of_guest = host.screen().snapshot().unwrap()["navigator"]["peer_work"].clone();
    assert!(of_guest.as_array().is_none_or(Vec::is_empty), "{of_guest}");
    // B54, D-55: `hide` at a terminal of the host outside every pane, in the
    // folder only the guest's pane lives in, is the guest's own `hide`. The
    // guest's core answers it with the pane of that folder, which the
    // host's core has no project for.
    let handed = Ran::of(
        host.command(&["ps", "--json"])
            .current_dir(&project)
            .output()
            .unwrap(),
    );
    assert!(handed.ok, "{}", handed.text());
    let panes = handed.json()["value"]["panes"].clone();
    assert!(
        panes
            .as_array()
            .is_some_and(|panes| panes.iter().any(|pane| in_folder(&pane["checkout_path"]))),
        "{panes}"
    );
    // B14: the guest's Hide ends, and its work goes on: the host lists and
    // types into the pane as before. B12: a command in the pane that needs
    // the guest's core is told at once that it is away.
    guest.ran(&["stop"]);
    host.lists("the host sees its guest gone", |listed| {
        let peer = listed["peers"].as_array()?.first()?;
        (peer["connected"] == false).then_some(())
    });
    assert_eq!(guest_panes(&host.listed()), [pane.as_str()]);
    types(&host, &pane, &itself("f4")).await;
    assert_eq!(left(&at("f4")), shell);
    types(&host, &pane, &asks_its_core("f5")).await;
    let away = left(&at("f5"));
    assert!(away.contains("owner_offline"), "{away}");

    // B16: the host's Hide starts anew with its guest still away. The node
    // ended with the daemon and the pane did not: it is listed from what the
    // host saved, a new node holds it, and the same shell takes the line.
    host.ran(&["stop"]);
    wait_within("the node to end with its host", WITHIN, || {
        Ok(node_runs().is_none().then_some(()))
    })
    .unwrap();
    host.ran(&["connect"]);
    assert_eq!(
        host.lists("the saved pane", |listed| {
            Some(guest_panes(listed)).filter(|panes| !panes.is_empty())
        }),
        [pane.as_str()]
    );
    types(&host, &pane, &itself("f6")).await;
    assert_eq!(left(&at("f6")), shell);
    let resumed = node_runs().expect("the node the host started anew");
    assert_ne!(resumed, node);

    // B15: the guest's Hide comes back, finds its host by itself and has
    // its pane again.
    guest.ran(&["connect"]);
    guest.lists("the guest opens its host again", |listed| {
        let peer = listed["peers"].as_array()?.first()?;
        (peer["connected"] == true).then_some(())
    });
    types(&guest, &on_guest, &itself("f7")).await;
    assert_eq!(left(&at("f7")), shell);

    // B60: the guest's node is killed. The host starts it again by itself,
    // the pane stays in its holder, and the host's daemon is the one it was.
    let daemon = host.screen().port;
    let killed = Command::new("kill")
        .arg("-9")
        .arg(resumed.to_string())
        .status()
        .unwrap();
    assert!(killed.success());
    wait_within("the node started after the kill", WITHIN, || {
        Ok(node_runs().filter(|pid| *pid != resumed))
    })
    .unwrap();
    assert_eq!(host.screen().port, daemon);
    types(&host, &pane, &itself("f8")).await;
    assert_eq!(left(&at("f8")), shell);

    // B10, D-74: the host's operator ends one pane of the guest's work from
    // a window, and the rest of that work stays.
    wait_within("the guest's link to its node started anew", WITHIN, || {
        Ok(guest
            .screen()
            .snapshot()?
            .pointer("/navigator/devices")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|row| row["id"] == host_id.as_str() && row["host"]["state"] == "ready")
            .then_some(()))
    })
    .unwrap();
    let second = at("hh").join("second");
    checkout(&second);
    guest
        .screen()
        .event(
            "create_workspace",
            json!({"device_id": host_id, "path": second, "label": "Second",
                "initialize_git": false}),
        )
        .unwrap();
    let added = host.lists("the guest's second pane on its host", |listed| {
        guest_panes(listed).into_iter().find(|id| *id != pane)
    });
    let screen = host.screen();
    let mut closing = Window::open_at(screen.port, &screen.token, "web").await;
    closing
        .send("guest_pane_close", json!({"pane_id": added}))
        .await;
    host.lists("the closed pane gone and the other kept", |listed| {
        (guest_panes(listed) == [pane.clone()]).then_some(())
    });

    // B60, B26: the host's operator takes the guest off and ends its work.
    // The pane closes with no word from the guest, the node ends, and
    // nothing of the guest is left on the host's account.
    let screen = host.screen();
    let mut window = Window::open_at(screen.port, &screen.token, "web").await;
    window
        .send(
            "pairing_remove",
            json!({"request_id": "x1", "device": GUEST, "direction": "incoming",
                "end_work": true}),
        )
        .await;
    assert_eq!(window.result("remove").await["ok"], true);
    host.lists("the guest's work ended", |listed| {
        guest_panes(listed).is_empty().then_some(())
    });
    wait_within("the guest's node and folder to go", WITHIN, || {
        let folder = hide_kit::layout::guest_dir(&host.terminal.state, &key);
        Ok((node_runs().is_none() && !folder.exists()).then_some(()))
    })
    .unwrap();
}
