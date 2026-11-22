//! The release A fault run (PRD device-connect-iroh D-34 (2)): two daemons
//! of a shipped copy of this build pair through a relay this test runs, the
//! guest opens a folder of its host, and the guest's agent in a pane there
//! (the remote mailbox fixture's stand-in, `SHIM`) sends a letter every two
//! seconds to the guest's agent at home, trying each one again under the
//! same intent until it is taken, as an agent does.
//! Meanwhile the run takes the connection down again and again, one fault
//! after another for as long as it is given: the relay restarts, one
//! machine sleeps, the guest loses its network, the host's Hide starts
//! anew. After each fault the guest has to find its host again and letters
//! have to reach its ledger again; at the end every letter the agent sent
//! is in the guest's ledger exactly once.
//!
//! A machine's sleep is every process of that machine stopped
//! (`SIGSTOP`) and continued: its timers stand still and nothing it sends
//! leaves, as in a sleep. The guest's lost network is the host's machine
//! and the relay stopped together while the guest runs on, so everything
//! the guest sends goes nowhere and its own timers run out, as when its
//! Wi-Fi goes. A network change, a new address under a live endpoint,
//! needs a network namespace or the system's packet filter, which this
//! test does not take (`docs/TESTING.md`, The release A gate).
//!
//! Its inputs are `HIDE_TEST_IROH_RELAY`, the iroh-relay that
//! `scripts/build-iroh-relay.sh` builds, and `HIDE_TEST_FAULT_RUN_MINUTES`
//! (8 when unset: about one round of the faults); `scripts/release-gate.sh`
//! runs it for two hours. Each fault is printed as one JSON line beginning
//! `fault_run:`, with how long the guest took to find its host again.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::GUEST;
use super::guest_work::{Machine, PROGRAMS, pair, place_build};
use crate::support::remote_delivery::{Agent, Environment, SHIM, quote, successful, wait_within};
use crate::support::shipped;
use hide_platform::process::OwnedChild;

/// The machine id the host daemon runs under.
const HOST: &str = "fault-run-host";

/// How long the guest may take to find its host again after a fault, and a
/// letter to reach its ledger after that. A hang guard, far above what a
/// redial takes; the run prints what each one took.
const RECOVERS_WITHIN: Duration = Duration::from_secs(180);

/// How long a step outside the faults may take.
const WITHIN: Duration = Duration::from_secs(90);

/// How long the run rests between a recovery and the next fault, so letters
/// flow through a settled connection too.
const CALM: Duration = Duration::from_secs(45);

/// The faults, in the order the run takes them and over again, each once
/// short and once long: a short one is over before the connection's idle
/// time runs out, a long one well after. A restart takes no time of its own.
const PLAN: [(Fault, Duration); 9] = [
    (Fault::RelayRestart, SHORT),
    (Fault::RelayRestart, LONG),
    (Fault::GuestSleeps, SHORT),
    (Fault::GuestSleeps, LONG),
    (Fault::HostSleeps, SHORT),
    (Fault::HostSleeps, LONG),
    (Fault::GuestNetworkLost, SHORT),
    (Fault::GuestNetworkLost, LONG),
    (Fault::HostRestarts, Duration::ZERO),
];
const SHORT: Duration = Duration::from_secs(15);
const LONG: Duration = Duration::from_secs(100);

#[derive(Clone, Copy, Debug)]
enum Fault {
    RelayRestart,
    GuestSleeps,
    HostSleeps,
    GuestNetworkLost,
    HostRestarts,
}

impl Fault {
    fn name(self) -> &'static str {
        match self {
            Self::RelayRestart => "relay_restart",
            Self::GuestSleeps => "guest_sleeps",
            Self::HostSleeps => "host_sleeps",
            Self::GuestNetworkLost => "guest_network_lost",
            Self::HostRestarts => "host_restarts",
        }
    }
}

/// The relay this run attaches both machines to, which it stops and starts.
struct Relay {
    program: PathBuf,
    config: PathBuf,
    child: Option<OwnedChild>,
    port: u16,
}

impl Relay {
    fn new(program: PathBuf, folder: &Path) -> Self {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let config = folder.join("relay.toml");
        std::fs::write(
            &config,
            format!(
                "enable_relay = true\nhttp_bind_addr = \"127.0.0.1:{port}\"\n\
                 enable_quic_addr_discovery = false\nenable_metrics = false\n"
            ),
        )
        .unwrap();
        let mut relay = Self {
            program,
            config,
            child: None,
            port,
        };
        relay.start();
        relay
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn start(&mut self) {
        let mut command = Command::new(&self.program);
        command
            .arg("--dev")
            .arg("--config-path")
            .arg(&self.config)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        self.child = Some(OwnedChild::spawn(&mut command).unwrap());
        let port = self.port;
        wait_within("the relay to listen", WITHIN, || {
            Ok(std::net::TcpStream::connect(("127.0.0.1", port))
                .is_ok()
                .then_some(()))
        })
        .unwrap();
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            child.kill_tree().unwrap();
            let _ = child.wait();
        }
    }

    fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(OwnedChild::id)
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Every process of one machine: its daemon, whatever names its state
/// folder (its nodes, a guest's node, every pane's holder), and all they
/// started. Read from the system's process list each time, since a fault
/// such as a restart makes new ones.
fn processes_of(machine: &Machine) -> Vec<u32> {
    let state = machine.terminal.state.to_string_lossy().into_owned();
    let daemon: Option<u32> = std::fs::read(machine.terminal.state.join("hided.json"))
        .ok()
        .and_then(|text| serde_json::from_slice::<Value>(&text).ok())
        .and_then(|state| state["pid"].as_u64())
        .map(|pid| pid as u32);
    let listing = Command::new("ps")
        .args(["-Ao", "pid=,ppid=,command="])
        .output()
        .unwrap();
    let rows: Vec<(u32, u32, String)> = String::from_utf8_lossy(&listing.stdout)
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let pid = words.next()?.parse().ok()?;
            let ppid = words.next()?.parse().ok()?;
            Some((pid, ppid, words.collect::<Vec<_>>().join(" ")))
        })
        .collect();
    let mut found: Vec<u32> = rows
        .iter()
        .filter(|(pid, _, command)| Some(*pid) == daemon || command.contains(&state))
        .map(|(pid, _, _)| *pid)
        .collect();
    let mut at = 0;
    while at < found.len() {
        let parent = found[at];
        for (pid, ppid, _) in &rows {
            if *ppid == parent && !found.contains(pid) {
                found.push(*pid);
            }
        }
        at += 1;
    }
    found
}

fn signal(pids: &[u32], name: &str) {
    if pids.is_empty() {
        return;
    }
    // A process that ended since it was listed answers an error, which is
    // nothing to stop.
    let _ = Command::new("kill")
        .arg(format!("-{name}"))
        .args(pids.iter().map(u32::to_string))
        .status();
}

/// Stops `machines` and `relay` for `long`, as a sleep or a lost network
/// does, and continues every process it stopped. The list is read twice,
/// so a process one of them started just then is stopped too.
async fn frozen(machines: &[&Machine], relay: Option<&Relay>, long: Duration) {
    let mut stopped: Vec<u32> = relay.and_then(Relay::pid).into_iter().collect();
    for _ in 0..2 {
        let now: Vec<u32> = machines.iter().flat_map(|m| processes_of(m)).collect();
        signal(&now, "STOP");
        stopped.extend(now);
    }
    if let Some(pid) = relay.and_then(Relay::pid) {
        signal(&[pid], "STOP");
    }
    // The time the machine is away is this fault's subject.
    tokio::time::sleep(long).await; // wait-ok: the machine has been away for the fault's length
    stopped.sort_unstable();
    stopped.dedup();
    signal(&stopped, "CONT");
}

/// The letters the shell sent, by intent, as the guest's ledger holds them.
fn letters_in(state: &Path, sender: &str) -> Vec<String> {
    let path = hide_kit::layout::delivery_store(state);
    if !path.exists() {
        return Vec::new();
    }
    let ledger = hide_core::delivery::store::load(&path)
        .unwrap_or_else(|code| panic!("the guest's ledger: {code}"));
    ledger
        .letters
        .iter()
        .filter(|letter| letter.from.as_deref() == Some(sender))
        .filter(|letter| letter.intent.starts_with("fault-"))
        .map(|letter| letter.intent.clone())
        .collect()
}

/// How many of `sender`'s letters in the guest's ledger its recipient has not
/// taken in yet.
fn open_letters(state: &Path, sender: &str) -> usize {
    let ledger = hide_core::delivery::store::load(&hide_kit::layout::delivery_store(state))
        .unwrap_or_else(|code| panic!("the guest's ledger: {code}"));
    ledger
        .letters
        .iter()
        .filter(|letter| letter.from.as_deref() == Some(sender) && letter.open())
        .count()
}

/// The numbers the shell wrote to `file`, one a line.
fn numbers(file: &Path) -> Vec<u64> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split_whitespace().next()?.parse().ok())
        .collect()
}

/// Whether the guest has its host again: the connection, and its node
/// there answering.
fn guest_has_its_host(guest: &Machine) -> bool {
    let connected = guest.run(&["device", "list", "--json"]);
    let peer = connected.ok
        && connected.json()["value"]["peers"]
            .as_array()
            .and_then(|peers| peers.first())
            .is_some_and(|peer| peer["connected"] == true);
    peer && guest.screen().snapshot().is_ok_and(|snapshot| {
        snapshot
            .pointer("/navigator/devices")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|row| row["id"] == HOST && row["host"]["state"] == "ready")
    })
}

/// What the agent in the guest's pane runs: letter after letter, each until
/// it is taken (`ok`, from the core or from the node's box while the core
/// is away), stopping after the letter it is on once `stop` exists. It runs
/// under the agent, so the node proves it a caller of the agent's pane.
fn sender_script(folder: &Path) -> String {
    let at = |name: &str| quote(folder.join(name));
    format!(
        r#"hide="$HIDE_INSTALL_CLI"
n=0
while [ ! -e {stop} ]; do
  n=$((n + 1))
  while :; do
    answer=$("$hide" request send fault-recipient --kind progress --intent "fault-$n" --body "letter $n" 2>&1)
    case "$answer" in *'"ok":true'*) break ;; esac
    printf '%s %s %s\n' "$n" "$(date +%s)" "$(printf '%s' "$answer" | tr '\n' ' ' | cut -c1-300)" >> {retries}
    sleep 1
  done
  case "$answer" in *queued*) printf '%s queued\n' "$n" >> {sent} ;; *) printf '%s\n' "$n" >> {sent} ;; esac
  sleep 2
done
: > {done}
"#,
        stop = at("stop"),
        retries = at("retries"),
        sent = at("sent"),
        done = at("done"),
    )
}

/// What the recipient's agent runs: it takes in every letter its inbox
/// lists, shown or folded, as an agent reading its letters does, until
/// `stop` exists. A
/// letter nobody takes in stays open, and a mailbox holds at most
/// `OPEN_LIMIT` open letters, which two hours of letters pass. A pane of
/// the machine's own core is handed no `HIDE_INSTALL_CLI`, so it names its
/// `hide`.
fn recipient_script(folder: &Path, hide: &Path) -> String {
    format!(
        r#"hide={hide}
log={log}
for sig in HUP INT QUIT TERM PIPE TTIN TTOU; do
  trap "printf 'signal %s %s\\n' $sig \"\$(date +%s)\" >> \"$log.signals\"; exit 1" $sig
done
while [ ! -e {stop} ]; do
  printf 'inbox %s\n' "$(date +%s)" > "$log.now"
  inbox=$("$hide" inbox 2>&1)
  printf 'inbox %s\n' "$(printf '%s' "$inbox" | tr '\n' ' ' | cut -c1-300)" >> "$log"
  ids=$(printf '%s' "$inbox" | grep -o 'letter-[0-9]*' | sort -u)
  for id in $ids; do
    printf 'ack %s %s\n' "$id" "$(date +%s)" > "$log.now"
    answer=$("$hide" request ack "$id" 2>&1)
    printf 'ack %s %s\n' "$id" "$(printf '%s' "$answer" | tr '\n' ' ' | cut -c1-200)" >> "$log"
  done
  tail -n 40 "$log" > "$log.tmp" && mv "$log.tmp" "$log"
  # A backlog is read again at once; an empty inbox is read a second later.
  [ -n "$ids" ] || sleep 1
done
"#,
        stop = quote(folder.join("stop")),
        log = quote(folder.join("read.log")),
        hide = quote(hide),
    )
}

/// Hands `script` to the stand-in agent in `pane` through its run order, as
/// the fixture hands every command, without waiting for it: it runs for the
/// whole run.
fn hand_over(home: &Path, pane: &str, script: &Path) {
    let order = home.join(".fixture-run").join(pane.replace(':', "-"));
    hide_platform::fs::private::create_dir_all(order.parent().unwrap()).unwrap();
    std::fs::write(order.with_extension("tmp"), script.display().to_string()).unwrap();
    std::fs::rename(order.with_extension("tmp"), &order).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "the release A gate: two daemons, a relay of its own and minutes of faults; scripts/release-gate.sh runs it"]
async fn every_letter_a_guests_pane_sends_through_the_faults_lands_in_its_ledger_once() {
    let relay_program = PathBuf::from(std::env::var_os("HIDE_TEST_IROH_RELAY").expect(
        "HIDE_TEST_IROH_RELAY names the iroh-relay this run starts: build it with scripts/build-iroh-relay.sh",
    ));
    let minutes: u64 = std::env::var("HIDE_TEST_FAULT_RUN_MINUTES")
        .map(|value| {
            value
                .parse()
                .expect("HIDE_TEST_FAULT_RUN_MINUTES is a number of minutes")
        })
        .unwrap_or(8);
    let root = crate::fixtures::fixture_root("hf");
    // A core lists a checkout by its real path, and `/tmp` is a link on
    // macOS.
    let base = root.path().canonicalize().unwrap();
    let at = |name: &str| base.join(name);
    let mut relay = Relay::new(relay_program, &base);
    let built = Path::new(env!("CARGO_BIN_EXE_hided")).parent().unwrap();
    let account = |name: &str, state: &str| {
        hide_platform::fs::private::create_dir_all(&at(state)).unwrap();
        Environment::new(&at(name), Some(&at(state))).unwrap()
    };
    let (host_env, guest_env) = (account("h", "hs"), account("g", "gs"));
    std::fs::create_dir(at("p")).unwrap();
    for name in PROGRAMS {
        assert!(
            built.join(name).is_file(),
            "build this worktree's programs first (scripts/verify-cargo.sh cli): no {name}"
        );
        shipped::place(&built.join(name), &at("p").join(name), &host_env).unwrap();
    }
    // The stand-in agent, as each account's `claude`: the letters' sender
    // on the host and their recipient at home.
    std::fs::write(at("provider.c"), SHIM).unwrap();
    let mut compiler = guest_env.command("/usr/bin/cc");
    compiler
        .arg("-O1")
        .arg(at("provider.c"))
        .arg("-o")
        .arg(guest_env.home.join("bin/claude"));
    successful(compiler).unwrap();
    std::fs::copy(
        guest_env.home.join("bin/claude"),
        host_env.home.join("bin/claude"),
    )
    .unwrap();
    let url = relay.url();
    let machine = |environment: &Environment, state: &str, id: &'static str| {
        let project = environment.home.join("project");
        for arguments in [
            vec!["-c", "init.defaultBranch=main", "init", "-q"],
            vec![
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "start",
            ],
        ] {
            let mut git = environment.command("/usr/bin/git");
            git.arg("-C").arg(&project).args(arguments);
            successful(git).unwrap();
        }
        let mut account: Vec<_> = environment
            .values()
            .into_iter()
            .filter(|(name, _)| {
                ["SHELL", "PATH", "skip_global_compinit"]
                    .iter()
                    .any(|session| name == *session)
            })
            .collect();
        // The product's endpoint on every interface, attached to this run's
        // relay instead of the device relay.
        account.push(("HIDE_PEER_RELAY".into(), url.clone().into()));
        Machine {
            terminal: super::Terminal::new(environment.home.clone(), at(state), Some(id)),
            hide: at("p").join("hide"),
            direct: String::new(),
            project,
            account,
        }
    };
    let host = machine(&host_env, "hs", HOST);
    let guest = machine(&guest_env, "gs", GUEST);
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
    place_build(&at("p"), &host.terminal.state, &key);
    assert_eq!(pair(&host, &guest).await, HOST);

    // The letters' recipient: the guest's own agent in its own project.
    std::fs::create_dir(at("cr")).unwrap();
    let recipient = Agent::start(
        &guest.screen(),
        at("cr"),
        guest_env.clone(),
        None,
        at("p").join("hide"),
        at("p").join("hide-agent-hooks"),
        "fault-recipient-session",
        "fault-recipient",
        &guest.terminal.state.join("engine").to_string_lossy(),
    )
    .unwrap_or_else(|error| panic!("the recipient: {error:#}"));

    // The sender: the guest's agent in a pane of its work on its host.
    wait_within("the guest's node on its host", WITHIN, || {
        Ok(guest_has_its_host(&guest).then_some(()))
    })
    .unwrap();
    guest
        .screen()
        .event(
            "create_workspace",
            json!({"device_id": HOST, "path": host_env.home.join("project"),
                "label": "Faults", "initialize_git": false}),
        )
        .unwrap();
    std::fs::create_dir(at("cs")).unwrap();
    let sender = Agent::start(
        &guest.screen(),
        at("cs"),
        host_env.clone(),
        Some(HOST),
        at("p").join("hide"),
        at("p").join("hide-agent-hooks"),
        "fault-sender-session",
        "fault-sender",
        HOST,
    )
    .unwrap_or_else(|error| panic!("the sender: {error:#}"));
    let run = at("run");
    std::fs::create_dir(&run).unwrap();
    let reads = run.join("read.sh");
    std::fs::write(&reads, recipient_script(&run, &at("p").join("hide"))).unwrap();
    let recipient_pane = recipient.own_pane(&recipient.pane).unwrap().to_owned();
    hand_over(&guest_env.home, &recipient_pane, &reads);
    let script = run.join("send.sh");
    std::fs::write(&script, sender_script(&run)).unwrap();
    hand_over(
        &host_env.home,
        &sender.own_pane(&sender.pane).unwrap(),
        &script,
    );
    let sent = run.join("sent");
    let sender = sender.registration.clone();
    wait_within("the first letter in the guest's ledger", WITHIN, || {
        Ok((!letters_in(&guest.terminal.state, &sender).is_empty()).then_some(()))
    })
    .unwrap_or_else(|error| {
        panic!(
            "{error:#}: retries {}",
            std::fs::read_to_string(run.join("retries")).unwrap_or_default()
        )
    });
    wait_within("the recipient to take a letter in", WITHIN, || {
        let state = &guest.terminal.state;
        Ok((letters_in(state, &sender).len() > open_letters(state, &sender)).then_some(()))
    })
    .unwrap_or_else(|error| {
        panic!(
            "{error:#}: its order taken {}, its panes {:?}, its log:\n{}",
            !guest_env
                .home
                .join(".fixture-run")
                .join(recipient_pane.replace(':', "-"))
                .exists(),
            crate::support::remote_delivery::ready_panes(&guest_env.home),
            std::fs::read_to_string(run.join("read.log")).unwrap_or_default()
        )
    });

    // The faults, one after another, until the time is up.
    let ends = Instant::now() + Duration::from_secs(minutes * 60);
    let mut turn = 0usize;
    let mut report = Vec::new();
    while Instant::now() < ends {
        let (fault, long) = PLAN[turn % PLAN.len()];
        turn += 1;
        let before = letters_in(&guest.terminal.state, &sender).len();
        let started = Instant::now();
        match fault {
            Fault::RelayRestart => {
                relay.stop();
                // The relay is away for this long: its absence is the fault.
                tokio::time::sleep(long).await; // wait-ok: the relay has been away for the fault's length
                relay.start();
            }
            Fault::GuestSleeps => frozen(&[&guest], None, long).await,
            Fault::HostSleeps => frozen(&[&host], None, long).await,
            Fault::GuestNetworkLost => frozen(&[&host], Some(&relay), long).await,
            Fault::HostRestarts => {
                host.ran(&["stop"]);
                host.ran(&["connect"]);
            }
        }
        let over = Instant::now();
        let found = wait_within("the guest to find its host again", RECOVERS_WITHIN, || {
            Ok(guest_has_its_host(&guest).then_some(()))
        });
        let back = over.elapsed();
        let flowing = found.and_then(|()| {
            wait_within(
                "a letter to reach the guest's ledger again",
                RECOVERS_WITHIN,
                || Ok((letters_in(&guest.terminal.state, &sender).len() > before).then_some(())),
            )
        });
        let line = json!({
            "turn": turn,
            "fault": fault.name(),
            "away_s": (over - started).as_secs(),
            "found_again_s": back.as_secs_f64(),
            "letters_before": before,
            "letters_now": letters_in(&guest.terminal.state, &sender).len(),
        });
        eprintln!("fault_run: {line}");
        report.push(line);
        if let Err(error) = flowing {
            panic!(
                "{error:#} after {}: {report:?}\nretries: {}",
                fault.name(),
                std::fs::read_to_string(run.join("retries")).unwrap_or_default()
            );
        }
        // The calm between two faults, in which letters flow through a
        // settled connection.
        tokio::time::sleep(CALM).await; // wait-ok: the calm between two faults has passed
    }

    // The shell stops after the letter it is on; then every letter it sent
    // is in the guest's ledger, each once.
    std::fs::write(run.join("stop"), b"").unwrap();
    wait_within("the sender to stop", RECOVERS_WITHIN, || {
        Ok(run.join("done").exists().then_some(()))
    })
    .unwrap();
    let mut written = numbers(&sent);
    let last = *written.last().expect("the shell sent letters");
    written.sort_unstable();
    assert_eq!(
        written,
        (1..=last).collect::<Vec<_>>(),
        "the shell's own record of what it sent"
    );
    let mut held = Vec::new();
    wait_within(
        "every letter in the guest's ledger",
        RECOVERS_WITHIN,
        || {
            held = letters_in(&guest.terminal.state, &sender);
            Ok((held.len() as u64 >= last).then_some(()))
        },
    )
    .unwrap_or_else(|error| panic!("{error:#}: {} of {last}", held.len()));
    let mut wanted: Vec<String> = (1..=last).map(|n| format!("fault-{n}")).collect();
    wanted.sort_unstable();
    held.sort_unstable();
    let queued = std::fs::read_to_string(&sent)
        .unwrap_or_default()
        .lines()
        .filter(|line| line.ends_with("queued"))
        .count();
    let retried = numbers(&run.join("retries")).len();
    eprintln!(
        "fault_run: {}",
        json!({"faults": report.len(), "letters": last, "held": held.len(),
            "kept_while_away": queued, "tries_again": retried,
            "not_taken_in": open_letters(&guest.terminal.state, &sender)})
    );
    let read = std::fs::read_to_string(run.join("read.log")).unwrap_or_default();
    let hides = Command::new("ps")
        .args(["-Ao", "pid=,etime=,command="])
        .output()
        .unwrap();
    eprintln!(
        "fault_run: the recipient at {} is in {}, its hide calls still running:\n{}\nits last reads:\n{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        format!(
            "{} {}",
            std::fs::read_to_string(run.join("read.log.now")).unwrap_or_default(),
            std::fs::read_to_string(run.join("read.log.signals")).unwrap_or_default()
        ),
        String::from_utf8_lossy(&hides.stdout)
            .lines()
            .filter(
                |line| line.contains(&*at("p").join("hide").to_string_lossy())
                    && (line.contains(" inbox") || line.contains(" ack "))
            )
            .collect::<Vec<_>>()
            .join("\n"),
        read.lines().rev().take(6).collect::<Vec<_>>().join("\n")
    );
    assert_eq!(held, wanted, "each letter the shell sent, once");
    drop(recipient);
}
