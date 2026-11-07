//! The engine contract (PRD engine-bundle-1 D-4): what a caller observes of
//! an engine, the same for every implementation behind [`Engine`]. A
//! workspace made in a folder holds one pane; text submitted to a pane runs
//! in its shell and a read shows what it printed; a pane opened beside
//! another and a tab launched with a command run what they were given;
//! Hide's metadata on a workspace and a pane reads back as written and
//! clears; a closed pane is gone; a call about a pane that does not exist is
//! the engine's refusal under a code; a tab's layout zooms, resizes, swaps,
//! moves a pane out and back, comes back in its shape when exported and
//! applied again, and refuses a layout past its limits (PRD
//! engine-bundle-3-core-owns B3, B4); and an engine whose server has gone
//! answers that nothing is running.
//!
//! [`meets_the_contract`] is the suite. Each implementation runs it in one
//! test of its own against an engine it started for the test, as each agent
//! runs its fixture through `hide-session/tests/it/adapters.rs`. Today that
//! is the core's own engine over pane holders this node starts from the
//! built holder, which needs `HIDE_TEST_HOLDER_BIN` (and `HIDE_HOLDER_CONPTY`
//! on Windows); it is ignored by default and the real-shell steps of
//! `pr.yml` run it on Linux and Windows.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use super::{Engine, EngineError, Wait, Zoom};
use crate::live::PaneResizeDirection;
use crate::recent_closed::{ClosedLayoutNode, ClosedSplitDirection};

const WAIT: Wait = Wait::Each(Duration::from_secs(10));
const SHOWN_WITHIN: Duration = Duration::from_secs(30);
/// How long a start may take to reach its program, the start wait a spawn
/// gives an agent's screen.
#[cfg(windows)]
const START_WITHIN: Duration = crate::agent_start::AGENT_START_DEADLINE;

/// A command line for the engine's default shell that prints `word-42`,
/// spelled so that the line itself does not contain what it prints.
fn computed_echo(word: &str) -> String {
    if cfg!(windows) {
        format!("Write-Output ('{word}-' + 6*7)")
    } else {
        format!("echo {word}-$((6*7))")
    }
}

/// A program that prints `word-42` and stays running.
fn computed_echo_program(word: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![
            "powershell.exe".into(),
            "-NoProfile".into(),
            "-Command".into(),
            format!("Write-Output ('{word}-' + 6*7); Start-Sleep 60"),
        ]
    } else {
        vec![
            "/bin/sh".into(),
            "-c".into(),
            format!("echo {word}-$((6*7)); sleep 60"),
        ]
    }
}

#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn shows(engine: &dyn Engine, pane: &str, text: &str) {
    let deadline = Instant::now() + SHOWN_WITHIN;
    loop {
        let read = engine.read_pane(WAIT, pane, "recent", 200);
        if read.as_ref().is_ok_and(|read| read.text.contains(text)) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "pane {pane} never showed {text:?}; last read: {read:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Waits until the pane's shell has drawn something, its prompt, so typed
/// text reaches a shell that reads it. `visible` is the drawn screen, which
/// holds a new pane's prompt.
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn prompted(engine: &dyn Engine, pane: &str) {
    let deadline = Instant::now() + SHOWN_WITHIN;
    while !engine
        .read_pane(WAIT, pane, "visible", 20)
        .is_ok_and(|read| !read.text.trim().is_empty())
    {
        assert!(Instant::now() < deadline, "pane {pane} never prompted");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn panes(engine: &dyn Engine) -> Vec<(String, String)> {
    let session = engine.session_payload(WAIT).expect("the session reads");
    let tab_of = |pane: &str| {
        session
            .layouts
            .iter()
            .find(|layout| {
                layout
                    .panes
                    .iter()
                    .any(|candidate| candidate.pane_id == pane)
            })
            .map(|layout| layout.tab_id.clone())
            .unwrap_or_default()
    };
    session
        .panes
        .iter()
        .map(|pane| (pane.pane_id.clone(), tab_of(&pane.pane_id)))
        .collect()
}

/// The rectangle `pane` is laid out in.
fn rect(engine: &dyn Engine, pane: &str) -> crate::sidebar::SessionLayoutRect {
    engine
        .pane_layout(WAIT, pane)
        .expect("the pane's layout reads")
        .panes
        .into_iter()
        .find(|laid| laid.pane_id == pane)
        .expect("the layout holds the pane")
        .rect
}

/// A tree's shape: its splits' directions and its leaves, ids left out.
fn shape(node: &ClosedLayoutNode) -> String {
    match node {
        ClosedLayoutNode::Pane { .. } => "pane".to_owned(),
        ClosedLayoutNode::Split {
            direction,
            first,
            second,
            ..
        } => format!("{direction:?}({}, {})", shape(first), shape(second)),
    }
}

/// `count` panes side by side, one more than an engine lays out in a tab
/// when it is 25.
fn row_of(count: usize, folder: &str) -> ClosedLayoutNode {
    let leaf = || ClosedLayoutNode::Pane {
        pane_id: None,
        label: None,
        cwd: Some(folder.to_owned()),
        command: None,
        env: BTreeMap::new(),
    };
    (1..count).fold(leaf(), |node, _| ClosedLayoutNode::Split {
        direction: ClosedSplitDirection::Right,
        ratio: 0.5,
        first: Box::new(node),
        second: Box::new(leaf()),
    })
}

/// The layout steps of the suite, on `pane`, the one pane of `tab`.
fn lays_out(engine: &dyn Engine, workspace: &str, tab: &str, pane: &str, folder: &str) {
    // A split at a ratio opens beside the pane, in its tab.
    let lower = engine
        .split_pane_at(
            WAIT,
            pane,
            ClosedSplitDirection::Down,
            folder,
            0.3,
            BTreeMap::new(),
        )
        .expect("a split at a ratio opens");
    assert!(panes(engine).contains(&(lower.clone(), tab.to_owned())));

    // A zoom turns on and off; a zoom of a lone pane changes nothing.
    assert!(engine.toggle_zoom(WAIT, pane).expect("the pane zooms"));
    assert!(engine.pane_layout(WAIT, pane).unwrap().zoomed);
    engine.set_zoom(WAIT, pane, Zoom::Off).unwrap();
    assert!(!engine.pane_layout(WAIT, pane).unwrap().zoomed);

    // A resize moves the split; a swap trades the two panes' places.
    let before = rect(engine, pane);
    assert!(
        engine
            .resize_pane(WAIT, pane, PaneResizeDirection::Down, 0.1)
            .expect("the pane resizes"),
        "a resize inside the limits moves the split"
    );
    assert!(rect(engine, pane).height > before.height);
    engine
        .swap_panes(WAIT, pane, &lower)
        .expect("the panes swap");
    assert!(rect(engine, pane).y > rect(engine, &lower).y);

    // An exported tab applied again comes back in its shape.
    let exported = engine.export_layout(WAIT, tab).expect("the tab exports");
    let applied = engine
        .apply_layout(WAIT, workspace, None, Some("applied"), &exported.root)
        .expect("the layout applies in a new tab");
    assert_eq!(shape(&applied.root), shape(&exported.root));
    assert_ne!(applied.tab_id, tab);
    engine.close_tab(WAIT, &applied.tab_id).unwrap();

    // A pane moves to a new tab, and back beside the pane it left.
    let moved = engine
        .move_pane_to_new_tab(WAIT, &lower, workspace, "moved")
        .expect("the pane moves to a new tab");
    let created = moved.created_tab_id.clone().expect("a tab was made for it");
    assert!(moved.changed);
    assert!(panes(engine).contains(&(moved.pane_id.clone(), created.clone())));
    let back = engine
        .move_pane_beside(WAIT, &moved.pane_id, tab, pane, true)
        .expect("the pane moves back beside");
    assert!(back.changed);
    assert!(panes(engine).contains(&(back.pane_id.clone(), tab.to_owned())));
    engine.close_pane(WAIT, &back.pane_id).unwrap();
    assert!(
        !engine.toggle_zoom(WAIT, pane).expect("a lone pane answers"),
        "a lone pane does not zoom"
    );

    // A layout past an engine's limits is refused with its reason.
    refused(
        engine.apply_layout(WAIT, workspace, None, Some("too many"), &row_of(25, folder)),
        "a layout of 25 panes",
    );
}

fn refused(answer: Result<impl std::fmt::Debug, EngineError>, what: &str) {
    match answer {
        Err(EngineError::Refused { code, .. }) => {
            assert!(!code.is_empty(), "{what}: a refusal names its code");
        }
        other => panic!("{what}: expected the engine's refusal, got {other:?}"),
    }
}

/// The suite: `engine` is a fresh engine with nothing open, `folder` a
/// folder its panes can start in, and `stop` ends the engine's server.
pub(crate) fn meets_the_contract(engine: &dyn Engine, folder: &str, stop: impl FnOnce()) {
    // A workspace made in a folder holds one tab with one pane.
    let (workspace, tab, pane) = engine
        .create_workspace(WAIT, folder, "contract", false, BTreeMap::new())
        .expect("a workspace is made");
    assert!(
        panes(engine).contains(&(pane.clone(), tab.clone())),
        "the session lists the new pane in its tab"
    );
    assert_eq!(engine.workspace_active_tab(WAIT, &workspace).unwrap(), tab);

    // Text submitted to the pane runs in its shell; a read shows its output.
    prompted(engine, &pane);
    engine
        .submit_text(WAIT, &pane, &computed_echo("submitted"))
        .expect("text is submitted");
    shows(engine, &pane, "submitted-42");

    // A pane opened beside another is in the same tab and runs what it is
    // given.
    let beside = engine
        .split_beside(WAIT, &pane, Some(folder), BTreeMap::new())
        .expect("a pane opens beside");
    assert!(panes(engine).contains(&(beside.clone(), tab.clone())));
    prompted(engine, &beside);
    engine
        .submit_text(WAIT, &beside, &computed_echo("beside"))
        .unwrap();
    shows(engine, &beside, "beside-42");

    // A tab launched with a command runs that command as its program.
    let (launched_tab, launched) = engine
        .launch_tab(
            WAIT,
            &workspace,
            None,
            "launched",
            false,
            folder,
            BTreeMap::new(),
            &computed_echo_program("launched"),
        )
        .expect("a tab is launched");
    assert!(panes(engine).contains(&(launched.clone(), launched_tab.clone())));
    shows(engine, &launched, "launched-42");

    // Hide's metadata on a workspace and a pane reads back, and clears.
    engine
        .report_workspace_token(WAIT, &workspace, "purpose", Some("engine contract"))
        .expect("a workspace token is reported");
    let purpose = |engine: &dyn Engine| {
        engine
            .workspace_tokens(WAIT, "purpose")
            .unwrap()
            .into_iter()
            .find(|(id, _)| *id == workspace)
            .and_then(|(_, value)| value)
    };
    assert_eq!(purpose(engine).as_deref(), Some("engine contract"));
    engine
        .report_workspace_token(WAIT, &workspace, "purpose", None)
        .unwrap();
    assert_eq!(purpose(engine), None);
    let token = super::tokens::PARENT_PANE_TOKEN;
    engine
        .report_pane_tokens(
            WAIT,
            &beside,
            &BTreeMap::from([(token.to_owned(), Some(pane.clone()))]),
        )
        .expect("a pane token is reported");
    assert_eq!(
        engine.pane_token(WAIT, &beside, token).unwrap(),
        Some(pane.clone())
    );
    engine
        .report_pane_tokens(WAIT, &beside, &BTreeMap::from([(token.to_owned(), None)]))
        .unwrap();
    assert_eq!(engine.pane_token(WAIT, &beside, token).unwrap(), None);

    // A closed pane is gone, and a call about it is refused.
    engine.close_pane(WAIT, &beside).expect("the pane closes");
    assert!(panes(engine).iter().all(|(id, _)| *id != beside));
    refused(
        engine.read_pane(WAIT, &beside, "recent", 10),
        "reading a closed pane",
    );
    refused(
        engine.submit_text(WAIT, &beside, "echo nothing"),
        "typing into a closed pane",
    );
    engine
        .close_tab(WAIT, &launched_tab)
        .expect("the tab closes");
    assert!(panes(engine).iter().all(|(id, _)| *id != launched));

    lays_out(engine, &workspace, &tab, &pane, folder);

    // An engine whose server has gone answers that nothing is running.
    stop();
    match engine.ping(WAIT) {
        Err(EngineError::NotRunning(_)) => {}
        other => panic!("a stopped engine says nothing is running, got {other:?}"),
    }
}

/// The core's own engine over holders started from the built holder
/// (`HIDE_TEST_HOLDER_BIN`), whose panes run `/bin/sh` (PowerShell on
/// Windows) in a folder of their own; the holders end with it (also
/// `real_agent_trust_tests`).
pub(crate) struct BuiltHolders {
    pub(crate) engine: super::own::OwnEngine<'static>,
    panes: std::sync::Arc<dyn hide_node_link::holders::PaneHolders>,
    service: std::sync::Arc<hide_node::terminal::Service>,
    /// The node's account home, which its panes' programs run in.
    pub(crate) home: std::path::PathBuf,
    folder: String,
    _root: tempfile::TempDir,
}

/// A folder for a built node's home (`h`), state (`s`) and panes (`w`). A
/// holder's socket path is limited to about a hundred bytes on Unix.
fn holder_root() -> (tempfile::TempDir, [std::path::PathBuf; 3]) {
    let root = if cfg!(unix) {
        tempfile::Builder::new()
            .prefix("ho")
            .tempdir_in("/tmp")
            .unwrap()
    } else {
        tempfile::tempdir().unwrap()
    };
    let folders = ["h", "s", "w"].map(|name| root.path().join(name));
    for dir in &folders {
        std::fs::create_dir_all(dir).unwrap();
    }
    (root, folders)
}

/// The shell the contract's command lines are written for, whatever shell
/// this account uses: a POSIX shell, and Windows PowerShell on Windows.
fn contract_shell() -> Vec<std::ffi::OsString> {
    if cfg!(windows) {
        vec!["powershell.exe".into()]
    } else {
        vec!["/bin/sh".into()]
    }
}

/// A node over `home` and `state_dir`, as a daemon's: its pane holders
/// started from the built holder (`HIDE_TEST_HOLDER_BIN`), whose panes run
/// `shell`, and the service that runs their terminals.
fn built_node(
    home: &std::path::Path,
    state_dir: &std::path::Path,
    shell: Vec<std::ffi::OsString>,
) -> (
    std::sync::Arc<hide_node::Local>,
    std::sync::Arc<hide_node::terminal::Service>,
) {
    use std::path::PathBuf;
    use std::sync::Arc;

    use hide_node::terminal::engine_panes::{LocalPanes, PaneShell};
    use hide_node::terminal::holder::{HolderFolders, Holders};
    use hide_node::terminal::{OutputSink, ReportSink};

    struct Nowhere;
    impl OutputSink for Nowhere {
        fn output(&self, _: &str, _: &[u8], _: bool) {}
        fn forget(&self, _: &str) {}
        fn clipboard(&self, _: &str, _: &[u8]) {}
    }
    impl ReportSink for Nowhere {
        fn report(&self, _: hide_node_link::terminal::TerminalReport) {}
    }

    let program = PathBuf::from(
        std::env::var_os("HIDE_TEST_HOLDER_BIN")
            .expect("HIDE_TEST_HOLDER_BIN names the built hide-holder"),
    );
    let conpty =
        cfg!(windows).then(|| {
            PathBuf::from(std::env::var_os("HIDE_HOLDER_CONPTY").expect(
                "HIDE_HOLDER_CONPTY names the pinned ConPTY runtime (scripts/fetch-conpty.py)",
            ))
        });
    let terminals = Holders::start_from(
        HolderFolders { home, state_dir },
        hide_kit::holders::Source { program, conpty },
        Arc::new(Nowhere) as Arc<dyn OutputSink>,
        Arc::new(Nowhere) as Arc<dyn ReportSink>,
    )
    .expect("the node's holders open");
    let holders = terminals.holders;
    let panes = Arc::new(LocalPanes::new(
        Arc::clone(&holders),
        Arc::clone(&terminals.service),
        PaneShell::Fixed(shell),
    ));
    (
        Arc::new(hide_node::Local::new(Some(home.to_owned())).with_panes(panes)),
        terminals.service,
    )
}

impl BuiltHolders {
    pub(crate) fn start() -> Self {
        Self::start_in(contract_shell())
    }

    /// The engine over holders whose panes run `shell`.
    pub(crate) fn start_in(shell: Vec<std::ffi::OsString>) -> Self {
        let (root, [home, state_dir, folder]) = holder_root();
        let (node, service) = built_node(&home, &state_dir, shell);
        // Asked as every own engine asks its machine's holders: through the
        // machine's node, here this machine's in-process node, by the
        // requests a device's node answers over its link (D-22).
        let linked = super::own::link_panes(node);
        linked
            .recovered(Duration::from_secs(120))
            .expect("the contract acquires pane ownership");
        Self {
            engine: super::own::OwnEngine::new(std::sync::Arc::clone(&linked)),
            panes: linked,
            service,
            home,
            folder: hide_platform::path::to_wire(&folder).unwrap(),
            _root: root,
        }
    }

    /// Ends the engine and every holder it left: the holders outlive an
    /// engine, so the test that started them ends them.
    pub(crate) fn stop(&self) {
        let left = self.engine.pane_folders(WAIT).expect("the session reads");
        self.engine.stop();
        for (pane, _) in left {
            let _ = self.panes.terminate(&pane);
        }
    }
}

#[test]
#[ignore = "needs the built pane holder: HIDE_TEST_HOLDER_BIN"]
fn the_own_engine_meets_the_engine_contract() {
    let built = BuiltHolders::start();
    meets_the_contract(&built.engine, &built.folder, || built.stop());
}

/// An ordinary program above the shell must never receive an agent's
/// launch line. This crosses the real holder and the OS process boundary.
#[test]
#[ignore = "needs the built pane holder: HIDE_TEST_HOLDER_BIN"]
fn the_own_engine_refuses_an_agent_start_over_a_running_program() {
    struct Running(BuiltHolders);
    impl Drop for Running {
        fn drop(&mut self) {
            self.0.stop();
        }
    }
    let running = Running(BuiltHolders::start());
    let built = &running.0;
    let engine = &built.engine;
    let (_, _, pane) = engine
        .create_workspace(WAIT, &built.folder, "busy", false, BTreeMap::new())
        .unwrap();
    prompted(engine, &pane);
    until("the shell alone holds its terminal", || {
        engine
            .pane_processes(WAIT, &pane)
            .is_ok_and(|processes| processes.shell_holds_terminal())
    });
    let command = if cfg!(windows) {
        "ping.exe -n 60 127.0.0.1"
    } else {
        "sleep 60"
    };
    engine.submit_text(WAIT, &pane, command).unwrap();
    until("the program runs above the shell", || {
        engine.pane_processes(WAIT, &pane).is_ok_and(|processes| {
            processes.foreground_process_group_id.is_some()
                && processes.foreground_process_group_id != processes.shell_pid
        })
    });
    let start = engine
        .prepare_start(super::AgentLaunch {
            pane_id: pane.clone(),
            name: "refused".into(),
            kind: "claude".into(),
            args: Vec::new(),
            codex_daemon: crate::codex_launch::CodexDaemon::Unsupported,
            wakes: false,
            program: None,
        })
        .unwrap();
    let error = engine.start_agent(WAIT, &start).unwrap_err();
    assert_eq!(error.code(), Some("agent_pane_busy"));
    assert!(engine.agents(WAIT).unwrap().is_empty());
}

/// A caller may inspect a tab or split as soon as creation answers. Each
/// first read crosses the real node and holder, without a readiness poll.
#[test]
#[ignore = "needs the built pane holder: HIDE_TEST_HOLDER_BIN"]
fn newly_answered_panes_can_be_read_without_waiting_for_another_event() {
    struct Running(BuiltHolders);
    impl Drop for Running {
        fn drop(&mut self) {
            self.0.stop();
        }
    }
    let running = Running(BuiltHolders::start());
    let built = &running.0;
    let engine = &built.engine;
    let (workspace, _, first) = engine
        .create_workspace(WAIT, &built.folder, "reads", true, BTreeMap::new())
        .unwrap();
    for split in [false, true] {
        for read in 0..5 {
            let pane = if split {
                engine
                    .split_beside(WAIT, &first, None, BTreeMap::new())
                    .unwrap()
            } else {
                engine
                    .create_tab(
                        WAIT,
                        &workspace,
                        &built.folder,
                        "read",
                        false,
                        BTreeMap::new(),
                    )
                    .unwrap()
                    .1
            };
            match read {
                0 => assert!(
                    engine
                        .pane_processes(WAIT, &pane)
                        .unwrap()
                        .shell_pid
                        .is_some()
                ),
                1 => {
                    engine.read_pane(WAIT, &pane, "recent", 10).unwrap();
                }
                2 => {
                    engine.read_screen(WAIT, &pane).unwrap();
                }
                3 => {
                    engine.pane_processes(WAIT, &pane).unwrap();
                }
                _ => {
                    engine.pane_identity(WAIT, &pane).unwrap();
                }
            }
            engine.close_pane(WAIT, &pane).unwrap();
        }
    }
}

/// Ends, when dropped, every holder and program a pane record under its
/// state folder names that still runs as recorded (by pid and start time),
/// whichever node started it: a test that opens one node after another over
/// that folder leaves nothing running on any exit, a panic included. What it
/// could not end fails the test, or, when the test already fails, is printed
/// beside that failure, which stays the test's.
struct RecordedProcesses(std::path::PathBuf);

impl Drop for RecordedProcesses {
    #[allow(clippy::disallowed_methods)] // polls each process's end, bounded by a deadline
    fn drop(&mut self) {
        let mut left = Vec::new();
        let records = hide_kit::layout::pane_records(&self.0);
        let entries = match std::fs::read_dir(&records) {
            Ok(entries) => entries.collect::<Vec<_>>(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                left.push(format!("{}: {error}", records.display()));
                Vec::new()
            }
        };
        for entry in entries {
            let path = match entry {
                Ok(entry) => entry.path(),
                Err(error) => {
                    left.push(format!("{}: {error}", records.display()));
                    continue;
                }
            };
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let record = match std::fs::read(&path)
                .map(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes))
            {
                Ok(Ok(record)) => record,
                // A record the test left empty names nothing.
                Ok(Err(_)) if std::fs::metadata(&path).is_ok_and(|meta| meta.len() == 0) => {
                    continue;
                }
                Ok(Err(error)) => {
                    left.push(format!("{}: {error}", path.display()));
                    continue;
                }
                Err(error) => {
                    left.push(format!("{}: {error}", path.display()));
                    continue;
                }
            };
            for process in ["holder", "program"] {
                let (Some(pid), Some(started)) = (
                    record[process]["pid"]
                        .as_u64()
                        .and_then(|pid| u32::try_from(pid).ok()),
                    record[process]["started"].as_u64(),
                ) else {
                    continue;
                };
                let runs =
                    || hide_platform::process::start_time(pid).is_ok_and(|now| now == started);
                if !runs() {
                    continue;
                }
                if let Err(error) = hide_platform::process::kill_tree(pid) {
                    left.push(format!("{process} {pid} of {}: {error}", path.display()));
                    continue;
                }
                let deadline = Instant::now() + SHOWN_WITHIN;
                while runs() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(20));
                }
                if runs() {
                    left.push(format!("{process} {pid} of {} still runs", path.display()));
                }
            }
        }
        if !left.is_empty() {
            if std::thread::panicking() {
                eprintln!("the test's cleanup also failed: {left:?}");
            } else {
                panic!("the test's cleanup failed: {left:?}");
            }
        }
    }
}

/// What a daemon on the core's own engine finds after the machine went
/// down: no holder runs, the layout save on disk is older than the last
/// pane made, and the pane records are as a power loss may leave them.
/// Every pane comes back under its own id, from its record or from the
/// save, and neither the pane whose program exited nor the one the operator
/// closed opens again, though the save still holds both:
///
/// - `kept`, in the save, whose record names the holder that ran (and on
///   Unix, where a record names its boot, an earlier boot), starts again;
/// - `unsaved`, made after the save, starts again from its record alone;
/// - `lost`, in the save, its record gone, starts again from the save;
/// - `damaged`, in the save, its record left empty before this boot,
///   starts again from the save once the node removes that record;
/// - `exited`, whose program exited, and `closed`, which the operator
///   closed (its record cancelled), stay closed.
#[test]
#[ignore = "needs the built pane holder: HIDE_TEST_HOLDER_BIN"]
#[allow(clippy::disallowed_methods)] // polls the save and the holders' ends, bounded by a deadline
fn a_restart_after_the_machine_went_down_brings_each_pane_back_under_its_id_and_no_ended_one() {
    use std::collections::BTreeSet;
    use std::sync::Arc;

    let (_root, [home, state_dir, folder]) = holder_root();
    // Dropped before the folder and after both nodes, on every exit.
    let _processes = RecordedProcesses(state_dir.clone());
    let folder = hide_platform::path::to_wire(&folder).unwrap();
    let engine_dir = hide_kit::layout::engine_dir(&state_dir);
    let open = || {
        let (node, service) = built_node(&home, &state_dir, contract_shell());
        let place = super::own::Place {
            engine_dir: engine_dir.clone(),
            home: hide_platform::path::to_wire(&home).unwrap(),
            hooks_home: Some(home.clone()),
            node: Arc::clone(&node) as Arc<dyn hide_node_link::NodeLink>,
            windows: cfg!(windows),
            definitions: Arc::new(hide_agent_adapter::definition::Catalog::read(Vec::new())),
            launches: Arc::default(),
        };
        let panes = super::own::link_panes(node);
        let engine =
            super::own::OwnEngine::open(Arc::clone(&panes), place).expect("the engine opens");
        (engine, panes, service)
    };
    let until = |what: &str, done: &dyn Fn() -> bool| {
        let deadline = Instant::now() + SHOWN_WITHIN;
        while !done() {
            assert!(Instant::now() < deadline, "{what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    let record =
        |pane: &str| hide_kit::layout::pane_record(&state_dir, &hide_kit::panes::holder_pane(pane));
    let read_record = |pane: &str| -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(record(pane)).unwrap()).unwrap()
    };

    let (first, first_panes, first_service) = open();
    let (workspace, tab, kept) = first
        .create_workspace(WAIT, &folder, "restart", true, BTreeMap::new())
        .unwrap();
    let mut saved = vec![kept.clone()];
    for _ in 0..4 {
        saved.push(
            first
                .split_beside(WAIT, &kept, None, BTreeMap::new())
                .unwrap(),
        );
    }
    for pane in &saved {
        prompted(&first, pane);
    }
    // The save as it stood before the last pane was made.
    let save = hide_kit::layout::engine_session_in(&engine_dir);
    let names_every_pane = |bytes: &[u8]| {
        let text = String::from_utf8_lossy(bytes);
        saved
            .iter()
            .all(|pane| text.contains(&format!("\"{pane}\"")))
    };
    until("the save names every pane", &|| {
        std::fs::read(&save).is_ok_and(|bytes| names_every_pane(&bytes))
    });
    let stale = std::fs::read(&save).unwrap();
    let (_, unsaved) = first
        .create_tab(WAIT, &workspace, &folder, "unsaved", false, BTreeMap::new())
        .unwrap();
    prompted(&first, &unsaved);
    let [kept, lost, damaged, exited, closed] = <[String; 5]>::try_from(saved).unwrap();
    first.close_pane(WAIT, &closed).unwrap();
    assert_eq!(
        read_record(&closed)["reservation"]["cancelled"],
        true,
        "a close is recorded as the pane's cancelled intent"
    );
    first.submit_text(WAIT, &exited, "exit").unwrap();
    until("the exited pane closes", &|| {
        panes(&first).iter().all(|(id, _)| *id != exited)
    });

    // The machine goes down: the daemon and every holder end, the save on
    // disk is the one from before `unsaved`, one record is gone and one was
    // left empty.
    first.stop();
    drop((first, first_panes, first_service));
    for pane in [&kept, &lost, &damaged, &unsaved] {
        let ran = read_record(pane);
        for process in ["holder", "program"] {
            let pid = u32::try_from(ran[process]["pid"].as_u64().unwrap()).unwrap();
            let started = ran[process]["started"].as_u64().unwrap();
            hide_platform::process::kill_tree(pid).unwrap();
            until("the pane's processes end", &|| {
                hide_platform::process::start_time(pid).map_or(true, |now| now != started)
            });
        }
    }
    std::fs::remove_file(record(&lost)).unwrap();
    std::fs::write(record(&damaged), b"").unwrap();
    std::fs::File::options()
        .write(true)
        .open(record(&damaged))
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1_000_000_000))
        .unwrap();
    if cfg!(unix) {
        let mut earlier = read_record(&kept);
        earlier["boot"] = "an-earlier-boot".into();
        std::fs::write(record(&kept), serde_json::to_vec(&earlier).unwrap()).unwrap();
    }
    std::fs::write(&save, &stale).unwrap();

    let (next, next_panes, _next_service) = open();
    let back = [&kept, &lost, &damaged, &unsaved];
    for pane in back {
        prompted(&next, pane);
    }
    let found = panes(&next);
    assert_eq!(
        found
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<BTreeSet<_>>(),
        back.iter().map(|id| (*id).clone()).collect::<BTreeSet<_>>(),
        "every pane is back under its own id, and no ended one"
    );
    let tab_of = |pane: &str| {
        found
            .iter()
            .find(|(id, _)| id == pane)
            .map(|(_, tab)| tab.clone())
            .unwrap()
    };
    for pane in [&kept, &lost, &damaged] {
        assert_eq!(tab_of(pane), tab, "{pane} is back in the saved tab");
    }
    assert_ne!(tab_of(&unsaved), tab, "the unsaved pane is gathered apart");
    for pane in back {
        next.submit_text(WAIT, pane, &computed_echo("back"))
            .unwrap();
        shows(&next, pane, "back-42");
    }
    for pane in back {
        next_panes
            .terminate(pane)
            .unwrap_or_else(|error| panic!("{pane} did not end: {error:?}"));
    }
}

/// A tab made in a plain folder whose only pane now runs in a plain folder
/// below it is still that folder's after the daemon restarts. The core's
/// record that it made the tab there does not outlive the daemon, so it reads
/// the restored session: the engine brings back the workspace's owner mark
/// with the pane, and the core places the pane's cwd under the folder the
/// mark names, where the cwd alone made the subfolder a project of its own.
#[test]
#[ignore = "needs the built pane holder: HIDE_TEST_HOLDER_BIN"]
fn a_restored_tab_whose_pane_is_in_a_subfolder_stays_in_the_folder_its_workspace_owns() {
    use std::sync::Arc;

    let (_root, [home, state_dir, folder]) = holder_root();
    let _processes = RecordedProcesses(state_dir.clone());
    // The real path, as a registration keeps it and the owner mark names it.
    let folder = hide_platform::fs::identity::canonical(&folder).unwrap();
    let sub = folder.join("복구할-작업-폴더");
    std::fs::create_dir(&sub).unwrap();
    let folder = hide_platform::path::to_wire(&folder).unwrap();
    let sub = hide_platform::path::to_wire(&sub).unwrap();
    let engine_dir = hide_kit::layout::engine_dir(&state_dir);
    let open = || {
        let (node, service) = built_node(&home, &state_dir, contract_shell());
        let place = super::own::Place {
            engine_dir: engine_dir.clone(),
            home: hide_platform::path::to_wire(&home).unwrap(),
            hooks_home: Some(home.clone()),
            node: Arc::clone(&node) as Arc<dyn hide_node_link::NodeLink>,
            windows: cfg!(windows),
            definitions: Arc::new(hide_agent_adapter::definition::Catalog::read(Vec::new())),
            launches: Arc::default(),
        };
        let panes = super::own::link_panes(node);
        let engine =
            super::own::OwnEngine::open(Arc::clone(&panes), place).expect("the engine opens");
        (engine, panes, service)
    };

    let (first, first_panes, first_service) = open();
    let (workspace, tab, root) = first
        .create_workspace(WAIT, &folder, "fixture", true, BTreeMap::new())
        .unwrap();
    // What the core writes on a plain folder's owner when it opens one.
    let mark = crate::checkout_owner::owner_mark(crate::node::TEST_NODE, &folder);
    first
        .report_workspace_token(
            WAIT,
            &workspace,
            crate::checkout_owner::OWNER_TOKEN,
            Some(&mark),
        )
        .unwrap();
    let pane = first
        .split_beside(WAIT, &root, Some(&sub), BTreeMap::new())
        .unwrap();
    prompted(&first, &pane);
    first.close_pane(WAIT, &root).unwrap();
    // The daemon ends and starts again; the pane's holder runs on.
    first.stop();
    drop((first, first_panes, first_service));

    let (next, next_panes, _next_service) = open();
    prompted(&next, &pane);
    let mut core = crate::runtime::tests::runtime();
    assert!(core.ingest_session(Ok(next.session_payload(WAIT).unwrap())));
    let navigator = &core.snapshot().navigator;
    let holding = navigator
        .workspaces
        .iter()
        .flat_map(|project| &project.checkouts)
        .find(|checkout| {
            checkout
                .tabs
                .iter()
                .any(|shown| shown.id.as_deref() == Some(tab.as_str()))
        })
        .map(|checkout| checkout.path.clone());
    assert_eq!(holding, Some(folder.clone()), "the tab stays in its folder");
    assert_eq!(
        navigator
            .workspaces
            .iter()
            .map(|project| project.path.clone())
            .collect::<Vec<_>>(),
        vec![folder],
        "no project is made for the subfolder"
    );
    next_panes
        .terminate(&pane)
        .unwrap_or_else(|error| panic!("{pane} did not end: {error:?}"));
}

/// A program a pane's shell runs in another folder is reported with the
/// pane, as its machine reads the program's folder, so a checkout a program
/// works in is not removed as unused while the pane's own folder is elsewhere.
#[test]
#[ignore = "needs the built pane holder: HIDE_TEST_HOLDER_BIN"]
#[allow(clippy::disallowed_methods)] // polls the pane's places, bounded by a deadline
fn a_program_run_in_another_folder_is_that_panes_foreground_folder() {
    struct Running(BuiltHolders);
    impl Drop for Running {
        fn drop(&mut self) {
            self.0.stop();
        }
    }
    let running = Running(BuiltHolders::start());
    let built = &running.0;
    let engine = &built.engine;
    let native = hide_platform::path::from_wire(&built.folder).unwrap();
    // Typed into the shell, so plain ASCII: a shell reading its line in the
    // C locale takes other bytes as keys.
    let other = native.join("other-folder");
    std::fs::create_dir(&other).unwrap();
    // The folder as the system names it, which is what its machine reads.
    let other = hide_platform::fs::identity::canonical(&other).unwrap();
    let (_, _, pane) = engine
        .create_workspace(WAIT, &built.folder, "folders", true, BTreeMap::new())
        .unwrap();
    prompted(engine, &pane);
    // A program outside the shell, started in the other folder: Windows
    // PowerShell's own location is not its process's folder, so it starts
    // `cmd.exe`, which changes its own.
    let line = if cfg!(windows) {
        format!(
            "cmd.exe /c \"cd /d {} && ping -n 60 127.0.0.1 > nul\"",
            other.display()
        )
    } else {
        format!("(cd '{}' && exec sleep 60)", other.display())
    };
    engine.submit_text(WAIT, &pane, &line).unwrap();
    let other = hide_platform::path::to_wire(&other).unwrap();
    let deadline = Instant::now() + SHOWN_WITHIN;
    let places = loop {
        let places = engine.pane_places(WAIT).unwrap();
        if places
            .iter()
            .any(|place| place.foreground_cwd.as_deref() == Some(other.as_str()))
        {
            break places;
        }
        assert!(
            Instant::now() < deadline,
            "the program's folder was never reported: {places:?}, the holder says {:?}, the pane shows {:?}",
            built.panes.processes(&pane, Duration::from_secs(5)),
            engine
                .read_pane(WAIT, &pane, "recent", 20)
                .map(|text| text.text),
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(places.iter().all(|place| place.pane_id == pane));
    assert!(
        engine
            .pane_folders(WAIT)
            .unwrap()
            .contains(&(pane.clone(), Some(other))),
        "the cleanup's read guards the program's folder"
    );
}

/// PRD engine-bundle-3-core-owns B28, D-9: a pane on the core's own engine
/// names itself `HIDE_PANE_ID` and no Herdr pane, says its terminal is
/// Hide's, and its proof names the shell its record holds, by the start time
/// the system gives that shell.
#[test]
#[ignore = "needs the built pane holder: HIDE_TEST_HOLDER_BIN"]
fn an_own_engine_pane_names_itself_and_proves_its_recorded_shell() {
    let built = BuiltHolders::start();
    let engine = &built.engine;
    // What the daemon was started with reaches its holders; a Herdr pane's
    // variables among it must not reach the pane's program.
    let env = BTreeMap::from([
        ("HERDR_PANE_ID".to_owned(), "w9:p9".to_owned()),
        (
            "HERDR_SOCKET_PATH".to_owned(),
            "/nowhere/herdr.sock".to_owned(),
        ),
    ]);
    let (_, _, pane) = engine
        .create_workspace(WAIT, &built.folder, "identity", false, env)
        .expect("a workspace is made");
    prompted(engine, &pane);
    let line = if cfg!(windows) {
        "Write-Output \"id=$env:HIDE_PANE_ID term=$env:TERM_PROGRAM herdr=$(@(Get-ChildItem env:HERDR_* -ErrorAction SilentlyContinue).Count)\""
    } else {
        "echo \"id=$HIDE_PANE_ID term=$TERM_PROGRAM herdr=$(env | grep -c '^HERDR_')\""
    };
    engine.submit_text(WAIT, &pane, line).unwrap();
    shows(engine, &pane, &format!("id={pane} term=hide herdr=0"));

    let proof = engine
        .pane_identity(WAIT, &pane)
        .expect("the pane is proved");
    let shell = engine.pane_processes(WAIT, &pane).unwrap().shell_pid;
    assert_eq!(proof.pane_id, pane);
    assert_eq!(Some(u32::try_from(proof.shell_pid).unwrap()), shell);
    assert_eq!(
        proof.shell_started,
        Some(
            hide_platform::process::start_time(u32::try_from(proof.shell_pid).unwrap())
                .expect("the shell runs")
        ),
        "the record names the shell by the start the system gives it"
    );
    // A start that typed into this pane checks it by the same record, with
    // no node or engine socket to ask.
    let inspected = engine
        .inspect_pane(WAIT, &pane)
        .expect("the own engine answers for its pane");
    assert_eq!(inspected.terminal_id, proof.terminal_id);
    assert_eq!(inspected.shell_pid, proof.shell_pid);
    assert_eq!(Some(inspected.shell_started), proof.shell_started);
    built.stop();
}

/// A first input launched as a start's argument reaches the program npm's
/// Windows launcher runs exactly as the start decision gave it
/// (with-hide/hide-private#573): each start types its line into a real
/// pane's PowerShell, which hands it to `Start-Process`, in Windows
/// PowerShell 5.1 and in PowerShell 7, whose passing of native arguments
/// differs. One input holds what `cmd.exe` would read (`%`, `%PATH%`, `&`,
/// `^`, `"`), backslashes before a quote and a line break; the other is
/// 16 KiB starting with `-`, the agent journey's `start` cell. The stand-in
/// Codex is a script under npm's own launcher, run by the runner's Node.
#[cfg(windows)]
#[test]
#[ignore = "needs the built pane holder: HIDE_TEST_HOLDER_BIN"]
fn a_launched_first_input_reaches_npms_program_whole_in_windows_powershell() {
    launched_input_reaches_npms_program("powershell.exe");
}

#[cfg(windows)]
#[test]
#[ignore = "needs the built pane holder: HIDE_TEST_HOLDER_BIN"]
fn a_launched_first_input_reaches_npms_program_whole_in_powershell_7() {
    launched_input_reaches_npms_program("pwsh.exe");
}

#[cfg(windows)]
fn launched_input_reaches_npms_program(shell: &str) {
    struct Running(BuiltHolders);
    impl Drop for Running {
        fn drop(&mut self) {
            self.0.stop();
        }
    }
    let running = Running(BuiltHolders::start_in(vec![shell.into()]));
    let built = &running.0;
    let engine = &built.engine;
    engine.set_agent_definitions(std::sync::Arc::new(
        hide_agent_adapter::definition::Catalog::read(Vec::new()),
    ));

    // A local npm install of Codex whose script writes the words it was
    // given to the file its pane names.
    let install = tempfile::tempdir().unwrap();
    let form = npm_install(
        install.path(),
        "require('fs').writeFileSync(process.env.HIDE_TEST_ARGV, JSON.stringify(process.argv.slice(2)));",
    );

    let short = "100% of %PATH% & a ^ b \"quoted\" a\\\"b c:\\dir\\\nnext line";
    let long: String = std::iter::once('-')
        .chain("%PATH% & ^ \"x\" \\\" ".chars().cycle())
        .take(16 * 1024)
        .collect();
    for (index, input) in [short, long.as_str()].into_iter().enumerate() {
        let written = install.path().join(format!("argv-{index}.json"));
        let env = BTreeMap::from([(
            "HIDE_TEST_ARGV".to_owned(),
            written.to_str().unwrap().to_owned(),
        )]);
        let (_, _, pane) = engine
            .create_workspace(WAIT, &built.folder, &format!("launch-{index}"), false, env)
            .expect("a workspace is made");
        prompted(engine, &pane);
        until("the shell alone holds its terminal", || {
            engine
                .pane_processes(WAIT, &pane)
                .is_ok_and(|processes| processes.shell_holds_terminal())
        });
        let words = crate::start_prompt::launched("codex", &[], input, true, Some(&form))
            .expect("the start carries the input")
            .expect("Codex declares a launch");
        let start = engine
            .prepare_start(super::AgentLaunch {
                pane_id: pane.clone(),
                name: format!("launched-{index}"),
                kind: "codex".into(),
                args: words.clone(),
                codex_daemon: crate::codex_launch::CodexDaemon::Unsupported,
                wakes: false,
                program: super::StartProgram::of("codex", &form),
            })
            .expect("the start is prepared");
        engine
            .start_agent(WAIT, &start)
            .unwrap_or_else(|error| panic!("{shell} takes the start: {error:?}"));
        let given = || -> Option<Vec<String>> {
            serde_json::from_str(&std::fs::read_to_string(&written).ok()?).ok()
        };
        // A 16 KiB line is typed key by key into the shell's line editor.
        until_within("the program records its arguments", START_WITHIN, || {
            given().is_some()
        });
        let given = given().unwrap();
        assert_eq!(given, words, "{shell} changed a word on its way");
        assert_eq!(
            given.last().map(String::as_str),
            Some(input.replace('\n', "\u{2028}").as_str()),
            "{shell} did not hand the input over as its last word"
        );
    }
}

/// After the machine went down, a restarted pane's agent resumes through
/// the file its node starts the program from, as a start and a wake do,
/// never by the program's name, and every word of its resume reaches that
/// program whole (with-hide/hide-private#595). On Windows the node reads
/// npm's own launcher as the program it runs (Node and the package's
/// script), and the line runs in Windows PowerShell 5.1 and in PowerShell
/// 7: by the name, PowerShell would pick npm's `.ps1`, and 5.1 drops the
/// quotes inside an argument it hands on. Elsewhere the file is a script
/// `/bin/sh` runs. The resume, from an operator's definition, holds a JSON
/// argument (`"` and a space) and a session with both.
#[cfg(unix)]
#[test]
#[ignore = "needs the built pane holder: HIDE_TEST_HOLDER_BIN"]
fn a_restarted_agent_resumes_through_its_nodes_file_with_every_word_whole() {
    resumes_through_the_nodes_file("/bin/sh");
}

#[cfg(windows)]
#[test]
#[ignore = "needs the built pane holder: HIDE_TEST_HOLDER_BIN"]
fn a_restarted_agent_resumes_through_npms_program_with_every_word_whole_in_windows_powershell() {
    resumes_through_the_nodes_file("powershell.exe");
}

#[cfg(windows)]
#[test]
#[ignore = "needs the built pane holder: HIDE_TEST_HOLDER_BIN"]
fn a_restarted_agent_resumes_through_npms_program_with_every_word_whole_in_powershell_7() {
    resumes_through_the_nodes_file("pwsh.exe");
}

/// A node that answers how the stand-in agent's program starts with `form`,
/// as a node answers `Call::AgentInstalled`, and passes every other call to
/// the machine's node.
struct AnsweringStart {
    node: std::sync::Arc<dyn hide_node_link::NodeLink>,
    program: &'static str,
    form: hide_node_link::protocol::StartForm,
}

impl hide_node_link::NodeLink for AnsweringStart {
    fn call(
        &self,
        call: hide_node_link::protocol::Call,
        timeout: Duration,
    ) -> Result<hide_node_link::LinkAnswer, hide_node_link::LinkError> {
        match call {
            hide_node_link::protocol::Call::AgentInstalled { name } if name == self.program => Ok(
                hide_node_link::LinkAnswer::Parsed(serde_json::to_value(Some(&self.form)).unwrap()),
            ),
            other => self.node.call(other, timeout),
        }
    }

    fn reader_features(&self) -> Option<&hide_node_link::sessions::ReaderFeatures> {
        self.node.reader_features()
    }

    fn in_process(&self) -> bool {
        self.node.in_process()
    }
}

/// A local npm install of Codex under `install` whose script runs
/// `script` under npm's own launcher, and how a node answers for it
/// (`Call::AgentInstalled`, hide-host's `agent_start_form`): the launcher
/// read as the program it runs, Node found on the runner's `PATH`.
#[cfg(windows)]
fn npm_install(install: &std::path::Path, script: &str) -> hide_node_link::protocol::StartForm {
    use hide_node_link::protocol::StartForm;
    use hide_platform::programs::{self, Start};
    let package = install.join("node_modules").join("@openai").join("codex");
    std::fs::create_dir_all(package.join("bin")).unwrap();
    std::fs::write(
        package.join("package.json"),
        r#"{"name":"@openai/codex","bin":{"codex":"bin/codex.js"}}"#,
    )
    .unwrap();
    std::fs::write(
        package.join("bin").join("codex.js"),
        format!("#!/usr/bin/env node\n{script}\n"),
    )
    .unwrap();
    let bin = install.join("node_modules").join(".bin");
    std::fs::create_dir_all(&bin).unwrap();
    let launcher = bin.join("codex.cmd");
    std::fs::write(
        &launcher,
        include_str!("../../../hide-platform/tests/fixtures/launchers/npm-script.cmd"),
    )
    .unwrap();
    let search = std::env::var_os("PATH").expect("the runner's PATH holds Node");
    match programs::start(&launcher, &search) {
        Start::Direct { program, leading } => StartForm::Direct {
            program: program.into_os_string().into_string().unwrap(),
            leading: leading
                .into_iter()
                .map(|word| word.into_string().unwrap())
                .collect(),
        },
        Start::Batch => panic!("npm's launcher is read as the program it runs"),
    }
}

/// The stand-in's program as its node would answer for it, and the file it
/// writes the words it was given into, one to a line.
fn stand_in_start(
    install: &std::path::Path,
) -> (hide_node_link::protocol::StartForm, std::path::PathBuf) {
    let given = install.join("argv");
    #[cfg(unix)]
    let form = {
        use hide_node_link::protocol::StartForm;
        let script = install.join("standin.sh");
        std::fs::write(
            &script,
            "for word; do printf '%s\\n' \"$word\"; done > \"${0%/*}/argv\"\n",
        )
        .unwrap();
        StartForm::Direct {
            program: "/bin/sh".into(),
            leading: vec![script.to_str().unwrap().to_owned()],
        }
    };
    #[cfg(windows)]
    let form = npm_install(
        install,
        &format!(
            "require('fs').writeFileSync({}, process.argv.slice(2).map((word) => word + '\\n').join(''));",
            serde_json::to_string(given.to_str().unwrap()).unwrap()
        ),
    );
    (form, given)
}

#[allow(clippy::disallowed_methods)] // polls the save and the holders' ends, bounded by a deadline
fn resumes_through_the_nodes_file(shell: &str) {
    use std::sync::Arc;

    const SESSION: &str = "s \"1\" two";
    let (_root, [home, state_dir, folder]) = holder_root();
    // Dropped before the folder and after both nodes, on every exit.
    let _processes = RecordedProcesses(state_dir.clone());
    let folder = hide_platform::path::to_wire(&folder).unwrap();
    let engine_dir = hide_kit::layout::engine_dir(&state_dir);
    let install = tempfile::tempdir().unwrap();
    let (form, given) = stand_in_start(install.path());
    // An operator's definition of an agent whose resume holds a JSON
    // argument, as Claude Code's `--settings` takes one.
    let definitions = Arc::new(hide_agent_adapter::definition::Catalog::read(vec![
        hide_agent_adapter::definition::UserFile::Text {
            name: "standin.toml".into(),
            text: concat!(
                "schema = 1\nid = \"standin\"\nname = \"Stand-in\"\n",
                "executables = [\"standin\"]\nhooks = \"none\"\n\n[resume]\n",
                "session = [\"--settings\", '{\"a\": \"b c\"}', \"--resume\", \"{session}\"]\n",
            )
            .into(),
        },
    ]));
    assert!(
        definitions.rejected.is_empty(),
        "{:?}",
        definitions.rejected
    );
    let open = || {
        let (node, service) = built_node(&home, &state_dir, vec![shell.into()]);
        let place = super::own::Place {
            engine_dir: engine_dir.clone(),
            home: hide_platform::path::to_wire(&home).unwrap(),
            hooks_home: Some(home.clone()),
            node: Arc::new(AnsweringStart {
                node: Arc::clone(&node) as Arc<dyn hide_node_link::NodeLink>,
                program: "standin",
                form: form.clone(),
            }),
            windows: cfg!(windows),
            definitions: Arc::clone(&definitions),
            launches: Arc::default(),
        };
        let panes = super::own::link_panes(node);
        let engine =
            super::own::OwnEngine::open(Arc::clone(&panes), place).expect("the engine opens");
        (engine, panes, service)
    };
    let record =
        |pane: &str| hide_kit::layout::pane_record(&state_dir, &hide_kit::panes::holder_pane(pane));

    let (first, first_panes, first_service) = open();
    let (_, _, pane) = first
        .create_workspace(WAIT, &folder, "resume", true, BTreeMap::new())
        .unwrap();
    prompted(&first, &pane);
    let save = hide_kit::layout::engine_session_in(&engine_dir);
    until("the save names the pane", || {
        std::fs::read(&save)
            .is_ok_and(|bytes| String::from_utf8_lossy(&bytes).contains(&format!("\"{pane}\"")))
    });
    // The machine goes down with the agent running in the pane: the daemon
    // and the pane's holder end, and the save says what the pane ran.
    first.stop();
    drop((first, first_panes, first_service));
    let ran: serde_json::Value =
        serde_json::from_slice(&std::fs::read(record(&pane)).unwrap()).unwrap();
    for process in ["holder", "program"] {
        let pid = u32::try_from(ran[process]["pid"].as_u64().unwrap()).unwrap();
        let started = ran[process]["started"].as_u64().unwrap();
        hide_platform::process::kill_tree(pid).unwrap();
        until("the pane's processes end", || {
            hide_platform::process::start_time(pid).map_or(true, |now| now != started)
        });
    }
    fn give_agent(value: &mut serde_json::Value, pane: &str) -> bool {
        match value {
            serde_json::Value::Object(object)
                if object.get("id").and_then(|id| id.as_str()) == Some(pane)
                    && object.contains_key("cwd") =>
            {
                object.insert(
                    "agent".into(),
                    serde_json::json!({"kind": "standin", "session": SESSION}),
                );
                true
            }
            serde_json::Value::Object(object) => {
                object.values_mut().any(|value| give_agent(value, pane))
            }
            serde_json::Value::Array(values) => {
                values.iter_mut().any(|value| give_agent(value, pane))
            }
            _ => false,
        }
    }
    let mut saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&save).unwrap()).unwrap();
    assert!(give_agent(&mut saved, &pane), "the save holds the pane");
    std::fs::write(&save, serde_json::to_vec(&saved).unwrap()).unwrap();

    // The engine runs the restore and its resumes while it is held.
    let (_next, next_panes, _next_service) = open();
    let words = || -> Option<Vec<String>> {
        let text = std::fs::read_to_string(&given).ok()?;
        text.ends_with('\n')
            .then(|| text.lines().map(str::to_owned).collect())
    };
    // The resume waits for the restarted shell, then is typed key by key.
    until_within(
        "the resumed program records its words",
        crate::agent_start::AGENT_START_DEADLINE,
        || words().is_some(),
    );
    assert_eq!(
        words().unwrap(),
        ["--settings", "{\"a\": \"b c\"}", "--resume", SESSION],
        "{shell} changed a word of the resume on its way"
    );
    next_panes
        .terminate(&pane)
        .unwrap_or_else(|error| panic!("{pane} did not end: {error:?}"));
}

/// Ends the holder and the program of each of `panes` as the machine going
/// down does, by the pids and start times its record names, and waits for
/// each to be gone.
fn end_holders_of(state_dir: &std::path::Path, panes: &[&str]) {
    for pane in panes {
        let record = hide_kit::layout::pane_record(state_dir, &hide_kit::panes::holder_pane(pane));
        let ran: serde_json::Value =
            serde_json::from_slice(&std::fs::read(record).unwrap()).unwrap();
        for process in ["holder", "program"] {
            let pid = u32::try_from(ran[process]["pid"].as_u64().unwrap()).unwrap();
            let started = ran[process]["started"].as_u64().unwrap();
            hide_platform::process::kill_tree(pid).unwrap();
            until("the pane's processes end", || {
                hide_platform::process::start_time(pid).map_or(true, |now| now != started)
            });
        }
    }
}

/// Issue 653: the machine goes down with a Claude Code and a Codex running in
/// two panes, every holder and the daemon ending, and the next daemon opens a
/// new shell in each pane and resumes each agent in its own conversation. The
/// stand-in programs record the words they were started with, so the check is
/// what the agents were given, not what was typed. Each pane's shell runs with
/// the stand-ins first on its `PATH`.
#[cfg(unix)]
#[test]
#[ignore = "needs the built pane holder: HIDE_TEST_HOLDER_BIN"]
#[allow(clippy::disallowed_methods)] // polls the save, bounded by a deadline
fn a_restart_after_every_holder_is_gone_resumes_claude_and_codex_in_their_own_conversations() {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;

    use hide_agent_hooks::counters::{Change, Reporter, Said, apply};
    use hide_agent_hooks::report::HookTurn;

    use crate::agent_start::{ShellStart, start_at_shell_reusing_name};
    use crate::codex_launch::CodexDaemon;

    const CLAUDE: &str = "0b8f1f3e-5c2a-4a8e-9a54-2d4c1f6b7e90";
    const CODEX: &str = "0192f3a4-7b6c-7d8e-9f01-23456789abcd";
    let (root, [home, state_dir, folder]) = holder_root();
    let _processes = RecordedProcesses(state_dir.clone());
    let folder = hide_platform::path::to_wire(&folder).unwrap();
    let engine_dir = hide_kit::layout::engine_dir(&state_dir);
    let bin = root.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let args_of = |program: &str| root.path().join(format!("{program}-args"));
    for program in ["claude", "codex"] {
        let path = bin.join(program);
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexec sleep 600\n",
                args_of(program).display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let runs = |program: &str| std::fs::read_to_string(args_of(program)).unwrap_or_default();
    let shell = vec![
        "/usr/bin/env".into(),
        format!("PATH={}:/usr/bin:/bin", bin.display()).into(),
        "/bin/sh".into(),
    ];
    let open = || {
        let (node, service) = built_node(&home, &state_dir, shell.clone());
        let place = super::own::Place {
            engine_dir: engine_dir.clone(),
            home: hide_platform::path::to_wire(&home).unwrap(),
            hooks_home: Some(home.clone()),
            node: Arc::clone(&node) as Arc<dyn hide_node_link::NodeLink>,
            windows: false,
            definitions: Arc::new(hide_agent_adapter::definition::Catalog::read(Vec::new())),
            launches: Arc::default(),
        };
        let panes = super::own::link_panes(node);
        let engine =
            super::own::OwnEngine::open(Arc::clone(&panes), place).expect("the engine opens");
        (engine, panes, service)
    };

    let (first, first_panes, first_service) = open();
    let (_, _, claude) = first
        .create_workspace(WAIT, &folder, "reboot", false, BTreeMap::new())
        .unwrap();
    let codex = first
        .split_beside(WAIT, &claude, None, BTreeMap::new())
        .unwrap();
    let node = hide_node::Local::new(Some(home.clone()));
    for (pane, kind, name, session) in [
        (&claude, "claude", "worker-claude", CLAUDE),
        (&codex, "codex", "worker-codex", CODEX),
    ] {
        prompted(&first, pane);
        let start = first
            .prepare_start(super::AgentLaunch {
                pane_id: pane.clone(),
                name: name.into(),
                kind: kind.into(),
                args: Vec::new(),
                codex_daemon: CodexDaemon::Unsupported,
                wakes: false,
                program: None,
            })
            .expect("the start is prepared");
        start_at_shell_reusing_name(
            &first,
            Some(&node),
            &format!("contract:{name}"),
            ShellStart::Plain,
            &start,
            SHOWN_WITHIN,
            &|| Ok(()),
        )
        .unwrap_or_else(|error| panic!("{kind} starts: {error:?}"));
        until(&format!("{kind} runs"), || runs(kind) == "\n");
        // Its hook names the conversation and says its turn was running.
        first
            .hook_session(pane, kind, session)
            .expect("the hook's session is taken");
        apply(
            &home,
            pane,
            Change::None,
            Reporter {
                agent: Some(kind),
                session: Some(session),
            },
            Said::Turn(HookTurn::Working),
            None,
        )
        .unwrap();
    }
    let save = hide_kit::layout::engine_session_in(&engine_dir);
    until("the save names both conversations", || {
        std::fs::read(&save).is_ok_and(|bytes| {
            let text = String::from_utf8_lossy(&bytes);
            text.contains(CLAUDE) && text.contains(CODEX)
        })
    });

    // The machine goes down: the daemon, every holder and both agents end.
    first.stop();
    drop((first, first_panes, first_service));
    end_holders_of(&state_dir, &[&claude, &codex]);

    let (next, next_panes, _next_service) = open();
    until_within(
        "each agent is started again",
        crate::agent_start::AGENT_START_DEADLINE,
        || runs("claude").lines().count() == 2 && runs("codex").lines().count() == 2,
    );
    assert_eq!(runs("claude"), format!("\n--resume {CLAUDE}\n"));
    assert_eq!(runs("codex"), format!("\nresume {CODEX}\n"));
    until("each resumed agent holds its own conversation", || {
        next.agents(WAIT).is_ok_and(|agents| {
            [(&claude, CLAUDE), (&codex, CODEX)]
                .iter()
                .all(|(pane, session)| {
                    agents.iter().any(|agent| {
                        agent.pane_id == **pane
                            && agent.agent_session.as_ref().map(|s| s.value.as_str())
                                == Some(*session)
                    })
                })
        })
    });
    assert!(
        next.session(WAIT).unwrap().not_resumed.is_empty(),
        "both were resumed"
    );
    for pane in [&claude, &codex] {
        next_panes
            .terminate(pane)
            .unwrap_or_else(|error| panic!("{pane} did not end: {error:?}"));
    }
}

/// PRD engine-bundle-3-core-owns B18's hook cells: on the core's own engine
/// a Claude whose hook said its conversation is put to sleep in its pane by
/// its own quit key and woken there resuming that conversation by id, the
/// way the pane menu's Sleep and Wake run (`agent_sleep_engine`). The
/// stand-in `claude` records each start's arguments and rests until Ctrl+C
/// ends it, as Claude Code quits on one press into an empty composer.
#[cfg(unix)]
#[test]
#[ignore = "needs the built pane holder: HIDE_TEST_HOLDER_BIN"]
fn an_own_engine_claude_slept_with_its_hooked_conversation_wakes_resuming_it() {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use crate::agent_sleep::WakeMode;
    use crate::agent_sleep_engine::{WakeOutcome, WakeRequest, end_agent, start_agent};
    use crate::agent_start::{ShellStart, start_at_shell_reusing_name};
    use crate::codex_launch::CodexDaemon;
    use crate::recent_closed::{ClosedAgent, resume_arguments};
    use hide_node_link::terminal::{KeyTarget, TerminalControl, TerminalNode};

    const SESSION: &str = "0b8f1f3e-5c2a-4a8e-9a54-2d4c1f6b7e90";
    let built = BuiltHolders::start();
    let engine = &built.engine;
    engine.set_agent_definitions(std::sync::Arc::new(
        hide_agent_adapter::definition::Catalog::read(Vec::new()),
    ));
    let root = Path::new(&built.folder).parent().unwrap().to_owned();
    let (bin, args) = (root.join("bin"), root.join("claude-args"));
    std::fs::create_dir_all(&bin).unwrap();
    let claude = bin.join("claude");
    std::fs::write(
        &claude,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexec sleep 600\n",
            args.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    let starts = || std::fs::read_to_string(&args).unwrap_or_default();
    let env = BTreeMap::from([(
        "PATH".to_owned(),
        format!("{}:/usr/bin:/bin", bin.display()),
    )]);
    let (_, _, pane) = engine
        .create_workspace(WAIT, &built.folder, "sleep", false, env)
        .expect("a workspace is made");
    prompted(engine, &pane);
    let node = hide_node::Local::new(Some(root.join("h")));
    let start = engine
        .prepare_start(super::AgentLaunch {
            pane_id: pane.clone(),
            name: "sleeper".into(),
            kind: "claude".into(),
            args: Vec::new(),
            codex_daemon: CodexDaemon::Unsupported,
            wakes: false,
            program: None,
        })
        .expect("the start is prepared");
    start_at_shell_reusing_name(
        engine,
        Some(&node),
        "contract:sleep",
        ShellStart::Plain,
        &start,
        SHOWN_WITHIN,
        &|| Ok(()),
    )
    .unwrap_or_else(|error| panic!("the agent starts: {error:?}"));
    until("the agent runs and rests", || {
        starts() == "\n"
            && engine
                .agent_state(WAIT, &pane)
                .is_ok_and(|state| state.status == "idle")
    });

    // Its hook says which conversation it runs; the session projects it.
    engine
        .hook_session(&pane, "claude", SESSION)
        .expect("the hook's session is taken");
    let listed = engine.agents(WAIT).unwrap();
    let session = listed[0]
        .agent_session
        .as_ref()
        .map(|session| session.value.clone());
    assert_eq!(session.as_deref(), Some(SESSION));

    // From the sleep's start until the wake has run, the core holds the pane
    // asleep at its node (`TerminalControl::Asleep`, from the sleep record),
    // so the operator's keys go nowhere; the quit key and the resume line the
    // engine types still reach the agent.
    let asleep = |asleep: bool| {
        built.service.control(TerminalControl::Asleep {
            pane: pane.clone(),
            asleep,
        });
    };
    asleep(true);
    built
        .service
        .key(KeyTarget::Pane(pane.clone()), b"stray".to_vec(), 0);
    end_agent(engine, &node, &pane, "claude")
        .unwrap_or_else(|error| panic!("the agent sleeps: {error:?}"));
    let args = resume_arguments(&ClosedAgent {
        kind: "claude".into(),
        session_id: session.clone(),
        ..ClosedAgent::default()
    })
    .expect("Claude resumes by id");
    let woke = start_agent(
        engine,
        &node,
        &WakeRequest {
            pane_id: pane.clone(),
            kind: "claude".into(),
            name: "sleeper".into(),
            mode: WakeMode::Resume,
            args,
            cwd: None,
            session_id: session,
            codex_daemon: CodexDaemon::Unsupported,
        },
    );
    assert_eq!(woke, WakeOutcome::Started);
    until("the resume runs", || starts().lines().count() == 2);
    asleep(false);
    assert!(
        !engine
            .read_pane(WAIT, &pane, "recent", 200)
            .unwrap()
            .text
            .contains("stray"),
        "the operator's key reached a sleeping agent's pane"
    );
    assert_eq!(starts(), format!("\n--resume {SESSION}\n"));
    let woken = engine.agents(WAIT).unwrap();
    assert_eq!(
        woken[0]
            .agent_session
            .as_ref()
            .map(|session| session.value.as_str()),
        Some(SESSION),
        "the woken agent runs the conversation it resumed"
    );
    built.stop();
}

fn until(what: &str, done: impl Fn() -> bool) {
    until_within(what, SHOWN_WITHIN, done);
}

#[allow(clippy::disallowed_methods)] // a polling helper bounded by a deadline
fn until_within(what: &str, within: Duration, done: impl Fn() -> bool) {
    let deadline = Instant::now() + within;
    while !done() {
        assert!(Instant::now() < deadline, "{what} never happened");
        std::thread::sleep(Duration::from_millis(100));
    }
}
