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
/// text reaches a shell that reads it. `visible` is Herdr's drawn screen and
/// the own engine's last rows of screen and scrollback, which both hold a
/// new pane's prompt.
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
    holders: std::sync::Arc<hide_node::terminal::holder::Holders>,
    service: std::sync::Arc<hide_node::terminal::Service>,
    folder: String,
    _root: tempfile::TempDir,
}

impl BuiltHolders {
    fn start() -> Self {
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
        let conpty = cfg!(windows).then(|| {
            PathBuf::from(std::env::var_os("HIDE_HOLDER_CONPTY").expect(
                "HIDE_HOLDER_CONPTY names the pinned ConPTY runtime (scripts/fetch-conpty.py)",
            ))
        });
        // A holder's socket path is limited to about a hundred bytes on Unix.
        let root = if cfg!(unix) {
            tempfile::Builder::new()
                .prefix("ho")
                .tempdir_in("/tmp")
                .unwrap()
        } else {
            tempfile::tempdir().unwrap()
        };
        let (home, state_dir, folder) = (
            root.path().join("h"),
            root.path().join("s"),
            root.path().join("w"),
        );
        for dir in [&home, &state_dir, &folder] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let terminals = Holders::start_from(
            Some((
                HolderFolders {
                    home: &home,
                    state_dir: &state_dir,
                },
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
        Self {
            engine: super::own::OwnEngine::new(panes),
            holders,
            service: terminals.service,
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
            let _ = self.holders.terminate(&self.service, &pane);
        }
    }
}

#[test]
#[ignore = "needs the built pane holder: HIDE_TEST_HOLDER_BIN"]
fn the_own_engine_meets_the_engine_contract() {
    let built = BuiltHolders::start();
    meets_the_contract(&built.engine, &built.folder, || built.stop());
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
