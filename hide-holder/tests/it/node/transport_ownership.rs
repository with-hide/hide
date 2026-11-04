//! Gates surround the real holder handshake and shutdown. Local/LocalPanes,
//! the holder protocol and the program receiving input are not substituted.
use super::ownership::Connection;
use super::*;
use hide_node::terminal::{Cleanup, OpenFailure, SessionWriter, Transport};
use hide_node_link::holders::PaneInput;

struct Pause {
    entered: Sender<()>,
    resume: Receiver<()>,
}
struct Barrier {
    entered: Receiver<()>,
    resume: Option<Sender<()>>,
}
impl Barrier {
    fn pair() -> (Pause, Self) {
        let (entered, observed) = channel();
        let (resume, resumed) = channel();
        (
            Pause {
                entered,
                resume: resumed,
            },
            Self {
                entered: observed,
                resume: Some(resume),
            },
        )
    }
    fn reached(&self) {
        self.entered
            .recv_timeout(HANG_LIMIT)
            .expect("real boundary was reached");
    }
    fn release(&mut self) {
        if let Some(resume) = self.resume.take() {
            let _ = resume.send(());
        }
    }
}
impl Drop for Barrier {
    fn drop(&mut self) {
        self.release();
    }
}
impl Pause {
    fn wait(self) -> bool {
        let _ = self.entered.send(());
        self.resume.recv_timeout(HANG_LIMIT).is_ok()
    }
}

#[derive(Default)]
struct Gates {
    next: Mutex<Option<(Pause, Option<Pause>)>>,
    trace: Mutex<Vec<(&'static str, usize)>>,
    sequence: AtomicUsize,
    write: Mutex<Option<Pause>>,
    writes_finished: AtomicUsize,
}
impl Gates {
    fn arm(&self, cleanup: bool) -> (Barrier, Option<Barrier>) {
        let (pause, open) = Barrier::pair();
        let (close_pause, close) = if cleanup {
            let (pause, close) = Barrier::pair();
            (Some(pause), Some(close))
        } else {
            (None, None)
        };
        assert!(
            self.next
                .lock()
                .unwrap()
                .replace((pause, close_pause))
                .is_none()
        );
        (open, close)
    }
    fn pause_write(&self) -> Barrier {
        let (pause, barrier) = Barrier::pair();
        assert!(self.write.lock().unwrap().replace(pause).is_none());
        barrier
    }
    fn trace(&self) -> Vec<(&'static str, usize)> {
        self.trace.lock().unwrap().clone()
    }
}
struct Gated {
    actual: Box<dyn Attacher>,
    gates: Arc<Gates>,
}
impl Attacher for Gated {
    fn keeps_own_size(&self, pane: &str) -> bool {
        self.actual.keeps_own_size(pane)
    }
    fn open_transport(&self, pane: &str, size: Option<GridSize>) -> Result<Transport, OpenFailure> {
        let number = self.gates.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        self.gates.trace.lock().unwrap().push(("open", number));
        let pauses = self.gates.next.lock().unwrap().take();
        let cleanup_pause = if let Some((pause, cleanup)) = pauses {
            if !pause.wait() {
                return Err(OpenFailure::retried(
                    "fixture open barrier timed out".into(),
                ));
            }
            cleanup
        } else {
            None
        };
        let mut transport = self.actual.open_transport(pane, size)?;
        let Cleanup::Other(cleanup) = std::mem::replace(&mut transport.cleanup, Cleanup::None)
        else {
            panic!("the actual holder supplies its socket shutdown");
        };
        let gates = self.gates.clone();
        transport.cleanup = Cleanup::Other(Box::new(move || {
            let resumed = cleanup_pause.is_none_or(Pause::wait);
            cleanup();
            gates
                .trace
                .lock()
                .unwrap()
                .push((if resumed { "closed" } else { "barrier_timeout" }, number));
        }));
        Ok(Transport {
            writer: Box::new(GatedWriter {
                actual: transport.writer,
                gates: self.gates.clone(),
            }),
            ..transport
        })
    }
}

/// This gate is after production writer admission but before its real
/// socket write. It proves unknown-result and shutdown ownership, not
/// the duration or scheduling of an operating-system write.
struct GatedWriter {
    actual: Box<dyn SessionWriter>,
    gates: Arc<Gates>,
}
impl GatedWriter {
    fn admitted(&self) -> std::io::Result<()> {
        let pause = self.gates.write.lock().unwrap().take();
        if pause.is_some_and(|pause| !pause.wait()) {
            return Err(std::io::Error::other("fixture write barrier timed out"));
        }
        Ok(())
    }
}
impl SessionWriter for GatedWriter {
    fn input(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.admitted()?;
        let result = self.actual.input(bytes);
        self.gates.writes_finished.fetch_add(1, Ordering::SeqCst);
        result
    }
    fn paste(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.admitted()?;
        self.actual.paste(bytes)
    }
    fn keys(&mut self, keys: &[hide_holder_protocol::Key]) -> std::io::Result<()> {
        self.admitted()?;
        self.actual.keys(keys)
    }
    fn resize(&mut self, size: GridSize) -> std::io::Result<()> {
        self.actual.resize(size)
    }
    fn redraw(&mut self, size: GridSize) -> std::io::Result<()> {
        self.actual.redraw(size)
    }
    fn scroll(
        &mut self,
        lines: i32,
        column: Option<u16>,
        row: Option<u16>,
        modifiers: u8,
    ) -> std::io::Result<()> {
        self.actual.scroll(lines, column, row, modifiers)
    }
    fn release(&mut self) -> std::io::Result<()> {
        self.actual.release()
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.actual.flush()
    }
}

fn connection(folder: &Folder) -> (Connection, Arc<Gates>, Script) {
    let gates = Arc::new(Gates::default());
    let wrapped = gates.clone();
    let connection = Connection::with_attacher(folder.home.path(), &folder.state, |actual| {
        Box::new(Gated {
            actual,
            gates: wrapped,
        })
    });
    connection.linked.recovered(HANG_LIMIT).unwrap();
    let script = Script::new(&folder.home, vec![b"input-ready".to_vec()]);
    super::start_pane(
        &connection.linked,
        &HolderStart {
            pane: "p".into(),
            program: Some(
                role_program("script")
                    .into_iter()
                    .map(|part| part.into_string().unwrap())
                    .collect(),
            ),
            cwd: hide_platform::path::to_wire(folder.home.path()).unwrap(),
            env: vec![
                (ROLE.into(), "script".into()),
                (SCRIPT.into(), script.folder.to_str().unwrap().into()),
            ],
            rows: SIZE.rows,
            cols: SIZE.cols,
            notice: None,
        },
    )
    .unwrap();
    connection.screens.wait_drawn_after("p", 0, "input-ready");
    connection.reports.wait_state("p", "controlling");
    (connection, gates, script)
}
fn release(connection: &Connection) {
    connection.service.control(TerminalControl::Release {
        pane: "p".into(),
        message: "outside the last five visible tabs".into(),
    });
}
fn attach(connection: &Connection) {
    connection.service.control(TerminalControl::Attach {
        pane: "p".into(),
        size: None,
        manual: false,
    });
}
fn heard(script: &Script, expected: &[u8]) {
    wait_until("the actual program has read the input", || {
        script.heard().len() >= expected.len()
    });
    assert_eq!(script.heard(), expected);
}

/// A successful Local reply means the resume bytes reached the transport
/// once, even when the core releases its background view during reattach.
#[test]
fn background_command_keeps_delivery_until_write_then_releases_its_connection() {
    let folder = Folder::new();
    let (connection, gates, script) = connection(&folder);
    release(&connection);
    thread::scope(|scope| {
        let (mut open, _) = gates.arm(false);
        let input = scope.spawn(|| {
            connection
                .linked
                .input("p", PaneInput::Bytes(b"resume-command".to_vec()))
        });
        open.reached();
        release(&connection);
        assert_eq!(connection.reports.states("p").last().unwrap(), "released");
        open.release();
        input.join().unwrap().unwrap();
    });
    heard(&script, b"resume-command");
    connection.service.close();
    assert_eq!(
        gates.trace(),
        [("open", 1), ("closed", 1), ("open", 2), ("closed", 2)]
    );
    assert_eq!(connection.service.attached(), 0);
}

/// Hold the old attempt before its actual hello, request the successor,
/// then hold the old socket's actual shutdown. Neither a result check nor
/// dropping Session early is enough: open 3 must follow closed 2.
#[test]
fn a_superseded_open_finishes_shutdown_before_the_next_holder_connection_opens() {
    let folder = Folder::new();
    let (connection, gates, script) = connection(&folder);
    release(&connection);
    let (mut open, cleanup) = gates.arm(true);
    let mut cleanup = cleanup.unwrap();
    attach(&connection);
    open.reached();
    release(&connection);
    attach(&connection);
    open.release();
    cleanup.reached();
    assert_eq!(gates.trace(), [("open", 1), ("closed", 1), ("open", 2)]);
    cleanup.release();
    connection
        .linked
        .input("p", PaneInput::Bytes(b"new-owner".to_vec()))
        .unwrap();
    heard(&script, b"new-owner");
    connection.service.close();
    assert_eq!(
        gates.trace(),
        [
            ("open", 1),
            ("closed", 1),
            ("open", 2),
            ("closed", 2),
            ("open", 3),
            ("closed", 3)
        ]
    );
    assert!(
        !connection
            .reports
            .states("p")
            .iter()
            .any(|state| state == "taken_over")
    );
}

/// The real command's three-second deadline expires while its handshake
/// is gated. A later attach and sentinel prove the cancelled bytes never
/// execute late, rather than merely observing an error reply.
#[test]
fn an_expired_background_command_is_refused_and_never_written_after_the_gate_opens() {
    let folder = Folder::new();
    let (connection, gates, script) = connection(&folder);
    release(&connection);
    thread::scope(|scope| {
        let (mut open, _) = gates.arm(false);
        let input = scope.spawn(|| {
            connection
                .linked
                .input("p", PaneInput::Bytes(b"must-not-run".to_vec()))
        });
        open.reached();
        release(&connection);
        assert!(matches!(
            input.join().unwrap(),
            Err(HolderError::Refused { .. })
        ));
        open.release();
        attach(&connection);
        connection
            .linked
            .input("p", PaneInput::Bytes(b"sentinel".to_vec()))
            .unwrap();
    });
    heard(&script, b"sentinel");
    connection.service.close();
    assert_eq!(
        gates
            .trace()
            .iter()
            .filter(|(event, _)| *event == "open")
            .count(),
        gates
            .trace()
            .iter()
            .filter(|(event, _)| *event == "closed")
            .count()
    );
}

/// An admitted write cannot be called unsent on timeout. Its old socket
/// must become unusable before a successor can open, even if that writer
/// has not returned yet.
#[test]
fn a_writing_timeout_is_unknown_and_a_successor_waits_for_actual_shutdown() {
    let folder = Folder::new();
    let (connection, gates, script) = connection(&folder);
    release(&connection);
    thread::scope(|scope| {
        let (mut open, cleanup) = gates.arm(true);
        let mut cleanup = cleanup.unwrap();
        let mut writing = gates.pause_write();
        let input = scope.spawn(|| {
            connection
                .linked
                .input("p", PaneInput::Bytes(b"late".to_vec()))
        });
        open.reached();
        open.release();
        writing.reached();
        release(&connection);
        assert!(matches!(
            input.join().unwrap(),
            Err(HolderError::Unreachable { .. })
        ));
        cleanup.reached();
        attach(&connection);
        assert_eq!(gates.trace(), [("open", 1), ("closed", 1), ("open", 2)]);
        cleanup.release();
        // The new input's receipt proves the successor opened after the
        // actual shutdown while the old writer remains at its gate.
        connection
            .linked
            .input("p", PaneInput::Bytes(b"sentinel".to_vec()))
            .unwrap();
        writing.release();
    });
    wait_until("both admitted writers returned", || {
        gates.writes_finished.load(Ordering::SeqCst) == 2
    });
    heard(&script, b"sentinel");
    connection.service.close();
    assert_eq!(
        gates.trace(),
        [
            ("open", 1),
            ("closed", 1),
            ("open", 2),
            ("closed", 2),
            ("open", 3),
            ("closed", 3)
        ]
    );
}

#[test]
fn a_new_visible_attach_cancels_the_deferred_release() {
    let folder = Folder::new();
    let (connection, gates, script) = connection(&folder);
    release(&connection);
    thread::scope(|scope| {
        let (mut open, _) = gates.arm(false);
        let input = scope.spawn(|| {
            connection
                .linked
                .input("p", PaneInput::Paste(b"visible".to_vec()))
        });
        open.reached();
        release(&connection);
        attach(&connection);
        open.release();
        input.join().unwrap().unwrap();
    });
    heard(&script, b"visible");
    assert_eq!(
        connection.reports.states("p").last().unwrap(),
        "controlling"
    );
    assert_eq!(gates.trace(), [("open", 1), ("closed", 1), ("open", 2)]);
    connection.service.close();
}

#[test]
fn closing_during_a_letter_gap_reports_partial_delivery_and_never_submits() {
    let folder = Folder::new();
    let (connection, _, script) = connection(&folder);
    thread::scope(|scope| {
        let letter = scope.spawn(|| {
            connection.linked.letter(
                "p",
                PaneInput::Bytes(b"body".to_vec()),
                PaneInput::Bytes(b"submit".to_vec()),
                HANG_LIMIT,
            )
        });
        heard(&script, b"body");
        connection.service.control(TerminalControl::Closing {
            pane: "p".into(),
            closing: true,
        });
        assert!(matches!(
            letter.join().unwrap(),
            Err(HolderError::Unreachable { .. })
        ));
    });
    connection.service.control(TerminalControl::Closing {
        pane: "p".into(),
        closing: false,
    });
    connection
        .linked
        .input("p", PaneInput::Bytes(b"sentinel".to_vec()))
        .unwrap();
    heard(&script, b"bodysentinel");
    connection.service.close();
}

#[test]
fn an_oversized_engine_command_is_refused_without_poisoning_the_connection() {
    let folder = Folder::new();
    let (connection, _, script) = connection(&folder);
    assert!(matches!(
        connection.linked.input(
            "p",
            PaneInput::Bytes(vec![
                b'x';
                hide_node_link::terminal::INPUT_HOLD_LIMIT_BYTES + 1
            ])
        ),
        Err(HolderError::Refused { .. })
    ));
    connection
        .linked
        .input("p", PaneInput::Bytes(b"sentinel".to_vec()))
        .unwrap();
    heard(&script, b"sentinel");
    connection.service.close();
}
