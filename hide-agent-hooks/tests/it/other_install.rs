//! A guest's hook entries on its host's account (`docs/agent-hooks.md`, More
//! than one Hide on an account; `docs/ARCHITECTURE.md`, A guest's hook
//! entries): they are there while the guest's node runs, so every agent on
//! the account starts the guest's helper at each event. Anywhere but in a
//! pane of that guest, a pane the host's app started and a terminal outside
//! Hide alike, the entry says nothing and writes nothing, so what it costs
//! is one start of the helper, the way that agent starts a hook.
//!
//! The cost is printed and never asserted. How many guests a host has at
//! work at once (`hide_peer_protocol::launch::MAX_GUEST_NODES`) was set from
//! what the Windows lane printed here, and a runner's speed on one day is no
//! test's subject (`docs/TESTING.md`).

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hide_agent_hooks::route::{self, HookTarget};
use hide_agent_hooks::{AgentRuntime, HELPER_BINARY_NAME};
use serde_json::Value;

/// Where an agent that is not the guest's runs.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Place {
    /// A pane the host's app started, which names the app's installation.
    AppPane,
    /// A terminal outside Hide, which names no pane and no installation.
    Outside,
}

/// A host's account whose agents carry one guest's entries, with the
/// guest's helper where the entries name it.
pub(crate) struct Account {
    _dir: tempfile::TempDir,
    home: PathBuf,
}

/// What `runs` starts of one entry took, the first apart: a system that
/// checks a new program's file does it on that start.
pub(crate) struct Cost {
    first: Duration,
    later: Vec<Duration>,
}

impl std::fmt::Display for Cost {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let milliseconds = |duration: Duration| duration.as_secs_f64() * 1000.0;
        let at = |part: usize| self.later[(self.later.len() - 1) * part / 100];
        write!(
            formatter,
            "first {:.1} ms, then p50 {:.1} ms, p95 {:.1} ms, max {:.1} ms over {} starts",
            milliseconds(self.first),
            milliseconds(at(50)),
            milliseconds(at(95)),
            milliseconds(at(100)),
            self.later.len()
        )
    }
}

impl Account {
    pub(crate) fn with_a_guests_entries() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap().join("home");
        let state = hide_agent_hooks::state_folder::default_state_dir(&home);
        let root = state.join("g").join("0123456789abcdef").join("i");
        let helper = root.join("current").join(HELPER_BINARY_NAME);
        std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
        #[cfg(unix)]
        crate::stand_ins::place(crate::programs::hook(), &helper);
        #[cfg(windows)]
        std::fs::copy(crate::programs::hook(), &helper).unwrap();
        let install = route::install_of_guest_root(&root);
        for runtime in AgentRuntime::ALL {
            std::fs::create_dir_all(runtime.home_directory(&home)).unwrap();
            let target = HookTarget::new(&helper, &state)
                .unwrap()
                .of_install(&install)
                .unwrap();
            hide_agent_hooks::install(runtime, &home, target).unwrap();
        }
        Self { _dir: dir, home }
    }

    /// The entry `runtime` runs before each tool call.
    fn entry(&self, runtime: AgentRuntime) -> Value {
        let raw = std::fs::read_to_string(runtime.config_path(&self.home)).unwrap();
        let document: Value = serde_json::from_str(&raw).unwrap();
        let entry = document["hooks"]["PreToolUse"][0]["hooks"][0].clone();
        assert!(entry["command"].is_string(), "{runtime:?}: {document}");
        entry
    }

    /// Every file Hide keeps on the account and every file of its agents'
    /// folders: where a helper that went on would write. A shell an agent
    /// starts the entry in keeps files of its own under the home
    /// (PowerShell 7's start profile), which are not the entry's.
    fn files(&self) -> BTreeSet<PathBuf> {
        fn walk(folder: &Path, found: &mut BTreeSet<PathBuf>) {
            for entry in std::fs::read_dir(folder).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, found);
                } else {
                    found.insert(path);
                }
            }
        }
        let state = hide_agent_hooks::state_folder::default_state_dir(&self.home);
        let mut found = BTreeSet::new();
        walk(state.parent().unwrap(), &mut found);
        for runtime in AgentRuntime::ALL {
            walk(&runtime.home_directory(&self.home), &mut found);
        }
        found
    }

    /// Starts `runtime`'s tool entry `runs` times and once more in `place`,
    /// as that agent starts a hook: an entry with `args` with no shell, any
    /// other as the last argument of `shell`. Each start has to end well,
    /// print nothing and leave the account's files as they were.
    pub(crate) fn tool_calls(
        &self,
        runtime: AgentRuntime,
        shell: &[&str],
        place: Place,
        runs: usize,
    ) -> Cost {
        let entry = self.entry(runtime);
        let before = self.files();
        let mut took: Vec<Duration> = (0..=runs)
            .map(|_| {
                let mut command = started(&entry, shell);
                for (name, _) in std::env::vars_os() {
                    let text = name.to_string_lossy();
                    // What a pane the test runs in would hand the entry: this
                    // or another terminal's pane identity, an agent's own marks.
                    if [
                        "HIDE_",
                        hide_platform::process::FOREIGN_PANE_IDENTITY,
                        "HCOORD_",
                        "CURSOR_",
                        "OPENCODE",
                        "GROK_",
                    ]
                    .iter()
                    .any(|prefix| text.starts_with(prefix))
                    {
                        command.env_remove(name);
                    }
                }
                command
                    .env(hide_platform::host::HOME_VARIABLE, &self.home)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                if let Place::AppPane = place {
                    command
                        .env(route::INSTALL_VARIABLE, route::APP_INSTALL)
                        .env("HIDE_PANE_ID", "w1:p1");
                }
                let payload = serde_json::json!({
                    "session_id": "a-guests-entry",
                    "hook_event_name": "PreToolUse",
                    "cwd": self.home,
                    "tool_name": "Bash",
                    "tool_input": {"command": "true"},
                });
                let start = Instant::now();
                let mut child = command.spawn().unwrap();
                // An entry that has nothing to say may end before it reads.
                let _ = child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(payload.to_string().as_bytes());
                let output = child.wait_with_output().unwrap();
                let elapsed = start.elapsed();
                assert!(
                    output.status.success() && output.stdout.is_empty(),
                    "{runtime:?} through {shell:?} in {place:?}: {output:?}"
                );
                elapsed
            })
            .collect();
        assert_eq!(
            self.files(),
            before,
            "{runtime:?} through {shell:?} in {place:?}: a guest's entry writes nothing there"
        );
        let first = took.remove(0);
        took.sort();
        Cost { first, later: took }
    }
}

fn started(entry: &Value, shell: &[&str]) -> Command {
    let program = entry["command"].as_str().unwrap();
    if let Some(arguments) = entry["args"].as_array() {
        let mut command = Command::new(program);
        command.args(arguments.iter().map(|argument| argument.as_str().unwrap()));
        return command;
    }
    let (shell, flags) = shell
        .split_first()
        .expect("a command entry runs in a shell");
    let mut command = Command::new(shell);
    command.args(flags).arg(program);
    command
}

/// On a Unix system both agents hand the entry's command to `sh -c`.
#[cfg(unix)]
#[test]
fn a_guests_entries_say_nothing_anywhere_but_in_that_guests_panes() {
    let account = Account::with_a_guests_entries();
    for runtime in AgentRuntime::ALL {
        for place in [Place::AppPane, Place::Outside] {
            let cost = account.tool_calls(runtime, &["sh", "-c"], place, 20);
            println!("hook cost, a guest's {runtime:?} entry through sh, {place:?}: {cost}");
        }
    }
}
