//! Agents of two paired Hides, with both cores real (PRD
//! device-connect-iroh c2; B65, B66): two daemons of a shipped copy of this
//! build, each a machine of its own, pair by a link, so the MacBook opens
//! the Mac mini. An agent in a pane of each runs the shipped `hide`, as an
//! agent does.
//!
//! The MacBook's agent writes to the Mac mini's by `core:<device>:agent:<id>`
//! and reads its reply, lists and shows that core's agents, and starts a
//! Conductor there with no parent; the Mac mini's agent answers and writes
//! nothing first. The Conductor's mission is the Mac mini's, and when it
//! closes, the MacBook's agent that asked gets its report as a letter under
//! the Mac mini's name.
//!
//! The agents are the remote mailbox fixture's stand-in (`SHIM`), which runs
//! what the test hands its pane. The two endpoints bind loopback with no
//! relay (`HIDE_PEER_DIRECT`).

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use super::guest_work::{Machine, PROGRAMS, WITHIN, free_port, pair, place_build};
use super::{GUEST, Terminal};
use crate::support::remote_delivery::{
    Agent, Environment, PaneOutput, SHIM, quote, ready_panes, successful, wait_within,
};
use crate::support::shipped;

/// The machine id the Mac mini's daemon runs under; the MacBook's is
/// [`GUEST`], the side that pairs by the link and so opens the other.
const MINI: &str = "core-letters-mini";

/// One machine, its account and the stand-in agent in its project.
struct Core {
    machine: Machine,
    agent: Agent,
}

impl Core {
    /// What `hide args` answers the agent in its own pane.
    fn asks(&mut self, args: &str) -> PaneOutput {
        self.asks_in(&self.agent.pane.clone(), args)
    }

    /// What `hide args` answers an agent in `pane` of this machine.
    fn asks_in(&mut self, pane: &str, args: &str) -> PaneOutput {
        let command = format!("{} {args}", quote(&self.agent.hide));
        self.agent
            .run_in(pane, &command)
            .unwrap_or_else(|error| panic!("hide {args}: {error:#}"))
    }

    /// Waits until this core has kept a letter to the agent that `wanted`
    /// reads, and answers what the agent then reads of it in its inbox. The
    /// wait is on the core's saved ledger, so a slow machine costs the
    /// agent's pane no commands.
    fn receives<T>(&mut self, what: &str, wanted: impl Fn(&Value) -> Option<T>) -> T {
        let state = self.machine.terminal.state.clone();
        let pane = self.agent.pane.clone();
        wait_within(what, WITHIN, || {
            let Some(saved) = saved(&state)? else {
                return Ok(None);
            };
            Ok(saved["letters"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|letter| {
                    letter["recipient"]["pane_id"] == pane.as_str() && wanted(letter).is_some()
                })
                .then_some(()))
        })
        .unwrap_or_else(|error| panic!("{error:#}"));
        let inbox = self.asks("inbox").json().unwrap();
        inbox["letters"]
            .as_array()
            .into_iter()
            .flatten()
            .find_map(&wanted)
            .unwrap_or_else(|| panic!("{what} is not in the inbox: {inbox}"))
    }
}

/// A core's ledger as its store holds it; none until the daemon has made
/// the store, which this never makes itself.
fn saved(state: &Path) -> anyhow::Result<Option<Value>> {
    let path = hide_kit::layout::delivery_store(state);
    if !path.exists() {
        return Ok(None);
    }
    let ledger = hide_core::delivery::store::load(&path)
        .map_err(|code| anyhow::anyhow!("the ledger store: {code}"))?;
    Ok(Some(serde_json::to_value(ledger)?))
}

/// The code a refused `hide` command answered with.
fn refusal(output: &PaneOutput) -> String {
    assert_ne!(output.status, 0, "it was not refused: {}", output.stdout);
    let answer: Value = [&output.stdout, &output.stderr]
        .into_iter()
        .find_map(|text| serde_json::from_str(text.trim()).ok())
        .unwrap_or_else(|| panic!("no refusal: {} {}", output.stdout, output.stderr));
    answer["reason"]
        .as_str()
        .or_else(|| answer["error"]["code"].as_str())
        .unwrap_or_else(|| panic!("no code: {answer}"))
        .to_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "two daemons, a shipped build and stand-in agents; the remote mailbox lane runs it"]
async fn agents_of_two_paired_hides_write_read_and_report_across() {
    let root = crate::fixtures::fixture_root("hc");
    // A core lists a checkout by its real path, and `/tmp` is a link on
    // macOS.
    let base = root.path().canonicalize().unwrap();
    let at = |name: &str| base.join(name);
    let built = Path::new(env!("CARGO_BIN_EXE_hided")).parent().unwrap();
    // Each account as the remote mailbox fixture makes one: its own home,
    // startup files and project.
    let account = |name: &str, state: &str| {
        hide_platform::fs::private::create_dir_all(&at(state)).unwrap();
        Environment::new(&at(name), Some(&at(state))).unwrap()
    };
    let (mini_env, mac_env) = (account("m", "ms"), account("g", "gs"));
    std::fs::create_dir(at("p")).unwrap();
    for name in PROGRAMS {
        assert!(
            built.join(name).is_file(),
            "build this worktree's programs first (scripts/verify-cargo.sh cli): no {name}"
        );
        shipped::place(&built.join(name), &at("p").join(name), &mini_env).unwrap();
    }
    // The stand-in agent, as each account's `claude`.
    std::fs::write(at("provider.c"), SHIM).unwrap();
    let mut compiler = mini_env.command("/usr/bin/cc");
    compiler
        .arg("-O1")
        .arg(at("provider.c"))
        .arg("-o")
        .arg(mini_env.home.join("bin/claude"));
    successful(compiler).unwrap();
    std::fs::copy(
        mini_env.home.join("bin/claude"),
        mac_env.home.join("bin/claude"),
    )
    .unwrap();
    let machine = |environment: &Environment, state: &str, id: &'static str, direct: String| {
        let project = environment.home.join("project");
        for arguments in [
            vec!["-c", "init.defaultBranch=main", "init", "-q"],
            // A branch for a start's worktree needs a commit to stand on.
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
        Machine {
            // A fixture HOME: a paired core keeps no login item under it.
            terminal: Terminal::new(environment.home.clone(), at(state), Some(id)),
            hide: at("p").join("hide"),
            direct,
            project,
            // The daemon starts a pane's shell and finds an agent's program
            // as the account does, whatever shell runs the tests: from its
            // login shell and its `PATH`, and with no global completion
            // setup, which on Linux asks the pane a question about the
            // runner's folders before the shell's first prompt.
            account: environment
                .values()
                .into_iter()
                .filter(|(name, _)| {
                    ["SHELL", "PATH", "skip_global_compinit"]
                        .iter()
                        .any(|session| name == *session)
                })
                .collect(),
        }
    };
    let mini = machine(&mini_env, "ms", MINI, format!("127.0.0.1:{}", free_port()));
    let mac = machine(&mac_env, "gs", GUEST, "127.0.0.1:0".to_owned());

    // The MacBook's build is in the Mac mini's install root before it first
    // asks for its node there, as a release's would be sent.
    let key = hide_peer::key::MachineKey::load_or_create(
        &hide_kit::layout::machine_key(&mac.terminal.state),
        GUEST,
    )
    .unwrap()
    .0
    .public();
    mini.ran(&["connect"]);
    mac.ran(&["connect"]);
    mini.opens_its_project();
    mac.opens_its_project();
    place_build(&at("p"), &mini.terminal.state, &key);
    assert_eq!(pair(&mini, &mac).await, MINI);

    let agent = |machine: &Machine, environment: &Environment, name: &str, commands: &str| {
        std::fs::create_dir_all(at(commands)).unwrap();
        Agent::start(
            &machine.screen(),
            at(commands),
            environment.clone(),
            None,
            at("p").join("hide"),
            at("p").join("hide-agent-hooks"),
            &format!("fixture-{name}-session"),
            name,
            // A core's own engine is named by its folder in the daemon's
            // state (`hide_kit::layout::engine_dir`).
            &machine.terminal.state.join("engine").to_string_lossy(),
        )
        .unwrap_or_else(|error| panic!("the agent {name}: {error:#}"))
    };
    let mut mini = Core {
        agent: agent(&mini, &mini_env, "worker", "cm"),
        machine: mini,
    };
    let mut mac = Core {
        agent: agent(&mac, &mac_env, "asker", "cg"),
        machine: mac,
    };
    let worker_there = format!("core:{MINI}:agent:{}", mini.agent.registration);
    let asker_there = format!("core:{GUEST}:agent:{}", mac.agent.registration);

    // B66: the MacBook's agent reads the Mac mini's agents, and writes to
    // one by its address there.
    let listed = mac
        .asks(&format!("agent list --live --machine {MINI} --json"))
        .value()
        .unwrap();
    assert!(
        listed["items"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|agent| agent["id"] == mini.agent.registration.as_str()),
        "the Mac mini's worker is not listed: {listed}"
    );
    let shown = mac
        .asks(&format!("agent show {worker_there} --json"))
        .value()
        .unwrap();
    assert_eq!(shown["id"], mini.agent.registration.as_str(), "{shown}");
    let sent = mac
        .asks(&format!(
            "request send {worker_there} --intent ask-1 --kind request --body 'Is the build green?'"
        ))
        .json()
        .unwrap();
    let sent_id = sent["id"].as_str().unwrap().to_owned();
    // It arrives under the MacBook's name, and the reply names the letter
    // the MacBook's agent sent.
    let (arrived, from) = mini.receives("the MacBook's letter", |letter| {
        (letter["intent"] == "ask-1").then(|| {
            (
                letter["id"].as_str().unwrap().to_owned(),
                letter["sender"]["name"].as_str().unwrap().to_owned(),
            )
        })
    });
    assert!(
        from.starts_with("hide:core:") && from.ends_with(":asker"),
        "the sender does not name its machine: {from}"
    );
    mini.asks(&format!(
        "request reply {arrived} --intent answer-1 --body 'It is green.'"
    ))
    .json()
    .unwrap();
    let (reply_to, from) = mac.receives("the Mac mini's reply", |letter| {
        (letter["intent"] == "answer-1").then(|| {
            (
                letter["reply_to"].as_str().unwrap_or_default().to_owned(),
                letter["sender"]["name"].as_str().unwrap().to_owned(),
            )
        })
    });
    assert_eq!(reply_to, sent_id);
    assert!(from.ends_with(":worker"), "{from}");

    // The Mac mini does not open the MacBook, so its agent writes nothing
    // there first, and no pane of another core is typed into.
    let first = mini.asks(&format!(
        "request send {asker_there} --intent first-1 --kind request --body 'Hello'"
    ));
    assert_eq!(refusal(&first), "machine_not_permitted");
    let typed = mac.asks(&format!(
        "pane send core:{MINI}:pane:{} hello",
        mini.agent.pane
    ));
    assert_eq!(refusal(&typed), "machine_not_permitted");

    // B65: the MacBook's agent starts a Conductor on the Mac mini with no
    // parent. The stand-in reports no session of its own, which a real
    // agent's hook does as it starts, so this test reports it from the pane
    // the Mac mini starts.
    let project = mini.machine.project.to_string_lossy().into_owned();
    let spawn = format!(
        "agent spawn --machine {MINI} --name lead --intent lead-1 --kind claude --repo {} --branch lead",
        quote(&project)
    );
    let before = ready_panes(&mini_env.home).unwrap();
    let finished = AtomicBool::new(false);
    let (started, lead_pane) = std::thread::scope(|scope| {
        let reporter = scope.spawn(|| {
            let pane = wait_within("the Conductor's pane on the Mac mini", WITHIN, || {
                if finished.load(Ordering::Relaxed) {
                    return Ok(Some(None));
                }
                Ok(ready_panes(&mini_env.home)?
                    .into_iter()
                    .find(|pane| !before.contains(pane))
                    .map(Some))
            })
            .unwrap();
            if let Some(pane) = &pane {
                mini.agent
                    .report_session(pane, "fixture-lead-session")
                    .unwrap();
            }
            pane
        });
        let mut started = mac.asks(&spawn);
        // On a slow machine the start answers before its agent's session
        // is in, and says so: the same command asked again answers it once
        // the Mac mini has registered the agent.
        let unfinished = started.status != 0 && refusal(&started) == "native_identity_unavailable";
        if !unfinished {
            finished.store(true, Ordering::Relaxed);
        }
        let pane = reporter.join().unwrap();
        if unfinished {
            let state = mini.machine.terminal.state.clone();
            wait_within(
                "the Mac mini's registration of the Conductor",
                WITHIN,
                || {
                    let Some(saved) = saved(&state)? else {
                        return Ok(None);
                    };
                    Ok(saved["agents"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|agent| agent["name"] == "lead" && agent["ended"] == false)
                        .then_some(()))
                },
            )
            .unwrap_or_else(|error| panic!("{error:#}"));
            started = mac.asks(&spawn);
        }
        (started, pane)
    });
    let started = started
        .value()
        .unwrap_or_else(|error| panic!("the start on the Mac mini: {error:#}"));
    let lead_pane = lead_pane.expect("the Conductor's pane on the Mac mini");
    // The work is the Mac mini's: its core registered the Conductor, and
    // the MacBook's agent is its origin there.
    let lead = mini
        .asks_in(&lead_pane, "agent show here --json")
        .value()
        .unwrap();
    assert_eq!(lead["id"], started["id"], "{lead} {started}");
    assert!(lead["parent"].is_null(), "{lead}");

    // Its mission is the Mac mini's, and closing it reports to the
    // MacBook's agent that asked, once, under the Mac mini's name.
    // The Conductor's worktree is a checkout its core reads after the
    // start answers, with no window open on the Mac mini, as on a machine
    // nobody sits at; `hide mission create` says to ask again until then.
    let screen = mini.machine.screen();
    let checkout = lead["project"].as_str().unwrap().to_owned();
    let mut read = Value::Null;
    wait_within("the Mac mini's read of the new checkout", WITHIN, || {
        let snapshot = screen.snapshot()?;
        read = snapshot["git_worktrees"]["worktrees"].clone();
        let listed = read
            .as_array()
            .into_iter()
            .flatten()
            .any(|worktree| worktree["path"] == checkout.as_str());
        Ok((listed && snapshot["git_worktrees_loading"] == false).then_some(()))
    })
    .unwrap_or_else(|error| panic!("{error:#}: {checkout} is not in {read}"));
    let created = mini.asks_in(&lead_pane, "mission create --title 'Fix the build'");
    assert_eq!(created.status, 0, "{} {}", created.stdout, created.stderr);
    let closed = mini.asks_in(
        &lead_pane,
        "mission close --as cancelled --report 'Nothing to fix: the build is green.'",
    );
    assert_eq!(closed.status, 0, "{} {}", closed.stdout, closed.stderr);
    let report = |letter: &Value| {
        (letter["kind"] == "report").then(|| {
            (
                letter["sender"]["name"].as_str().unwrap().to_owned(),
                letter["body"].as_str().unwrap_or_default().to_owned(),
            )
        })
    };
    let (from, body) = mac.receives("the closed mission's report", report);
    assert!(
        from.starts_with("hide:core:") && from.ends_with(":lead"),
        "{from}"
    );
    assert!(
        body.contains("\"Fix the build\" closed as cancelled")
            && body.contains("Nothing to fix: the build is green."),
        "{body}"
    );
    let inbox = mac.asks("inbox").json().unwrap();
    let reports = inbox["letters"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|letter| report(letter).is_some())
        .count();
    assert_eq!(reports, 1, "{inbox}");
}
