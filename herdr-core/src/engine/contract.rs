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
//! runs its fixture through `hide-session/tests/it/adapters.rs`: the pinned
//! Herdr in a private server, which needs `HIDE_E2E_HERDR_BIN`, and the
//! core's own engine over pane holders this node starts from the built
//! holder, which needs `HIDE_TEST_HOLDER_BIN` (and `HIDE_HOLDER_CONPTY` on
//! Windows). Both are ignored by default; the real-shell steps of `pr.yml`
//! run them.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use super::{Engine, EngineError, Wait, Zoom};
use crate::live::PaneResizeDirection;
use crate::recent_closed::{ClosedLayoutNode, ClosedSplitDirection};

const WAIT: Wait = Wait::Each(Duration::from_secs(10));
const SHOWN_WITHIN: Duration = Duration::from_secs(30);

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

#[test]
#[ignore = "needs the pinned Herdr: HIDE_E2E_HERDR_BIN"]
fn the_pinned_herdr_meets_the_engine_contract() {
    use crate::agent_start::real_herdr_tests::PrivateHerdr;
    let herdr = PrivateHerdr::new(cfg!(unix).then(|| "/bin/sh".to_owned()));
    let engine = herdr.engine();
    let folder = herdr.home.to_string_lossy().into_owned();
    meets_the_contract(engine.as_ref(), &folder, || drop(herdr));
}

/// The core's own engine over holders started from the built holder
/// (`HIDE_TEST_HOLDER_BIN`), whose panes run `/bin/sh` (PowerShell on
/// Windows) in a folder of their own; the holders end with it.
struct BuiltHolders {
    engine: super::own::OwnEngine<'static>,
    panes: std::sync::Arc<dyn hide_node_link::holders::PaneHolders>,
    service: std::sync::Arc<hide_node::terminal::Service>,
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

/// A node over `home` and `state_dir`, as a daemon's: its pane holders
/// started from the built holder (`HIDE_TEST_HOLDER_BIN`), whose panes run
/// `/bin/sh` (PowerShell on Windows), and the service that runs their
/// terminals.
fn built_node(
    home: &std::path::Path,
    state_dir: &std::path::Path,
) -> (
    std::sync::Arc<hide_node::Local>,
    std::sync::Arc<hide_node::terminal::Service>,
) {
    use std::path::PathBuf;
    use std::sync::Arc;

    use hide_node::terminal::engine_panes::{LocalPanes, PaneShell};
    use hide_node::terminal::holder::{HolderFolders, Holders};
    use hide_node::terminal::{Attacher, Mode, OutputSink, ReportSink, SessionParts};

    struct NoHerdr;
    impl Attacher for NoHerdr {
        fn open(&self, _: &str, _: Mode, _: u16, _: u16) -> Result<SessionParts, String> {
            Err("this test runs no Herdr".to_owned())
        }
    }
    struct Nowhere;
    impl OutputSink for Nowhere {
        fn output(&self, _: &str, _: &[u8], _: bool) {}
        fn forget(&self, _: &str) {}
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
        Some((
            HolderFolders { home, state_dir },
            hide_kit::holders::Source { program, conpty },
        )),
        Box::new(NoHerdr),
        Arc::new(Nowhere) as Arc<dyn OutputSink>,
        Arc::new(Nowhere) as Arc<dyn ReportSink>,
    )
    .expect("the node's holders open");
    let holders = terminals.holders.expect("the holders are on");
    // The contract's command lines are a POSIX shell's and Windows
    // PowerShell's, whatever shell this account uses.
    let shell = if cfg!(windows) {
        vec!["powershell.exe".into()]
    } else {
        vec!["/bin/sh".into()]
    };
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
    fn start() -> Self {
        let (root, [home, state_dir, folder]) = holder_root();
        let (node, service) = built_node(&home, &state_dir);
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
            folder: hide_platform::path::to_wire(&folder).unwrap(),
            _root: root,
        }
    }

    /// Ends the engine and every holder it left: the holders outlive an
    /// engine, so the test that started them ends them.
    fn stop(&self) {
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
                    engine.foreground_program(WAIT, &pane).unwrap();
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
        let (node, service) = built_node(&home, &state_dir);
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
    // no node or Herdr socket to ask.
    let inspected = engine
        .inspect_pane(WAIT, &pane)
        .expect("the own engine answers for its pane")
        .expect("the pane is inspected");
    assert_eq!(inspected.terminal_id, proof.terminal_id);
    assert_eq!(inspected.shell_pid, proof.shell_pid);
    assert_eq!(Some(inspected.shell_started), proof.shell_started);
    built.stop();
}

/// PRD engine-bundle-3-core-owns B18's hook cells: on the core's own engine
/// a Claude whose hook said its conversation is put to sleep in its pane by
/// its own quit key and woken there resuming that conversation by id, the
/// way the pane menu's Sleep and Wake run (`agent_sleep_herdr`). The
/// stand-in `claude` records each start's arguments and rests until Ctrl+C
/// ends it, as Claude Code quits on one press into an empty composer.
#[cfg(unix)]
#[test]
#[ignore = "needs the built pane holder: HIDE_TEST_HOLDER_BIN"]
fn an_own_engine_claude_slept_with_its_hooked_conversation_wakes_resuming_it() {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use crate::agent_sleep::WakeMode;
    use crate::agent_sleep_herdr::{WakeOutcome, WakeRequest, end_agent, start_agent};
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

#[cfg(unix)]
#[allow(clippy::disallowed_methods)] // a polling helper bounded by a deadline
fn until(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + SHOWN_WITHIN;
    while !done() {
        assert!(Instant::now() < deadline, "{what} never happened");
        std::thread::sleep(Duration::from_millis(100));
    }
}
