use std::io::Write;
use std::sync::mpsc::{Receiver, Sender, channel};

use super::*;

pub(super) const WAIT: Duration = Duration::from_secs(5);

/// What a stand-in session writes, one JSON value per call, in the form the
/// tests read: its input, its resizes and redraws, its wheels and its
/// release. Paste and named keys are refused, as by a transport that does
/// not read them.
pub(super) struct FakeWriter(pub(super) Box<dyn Write + Send>);

impl FakeWriter {
    fn line(&mut self, value: serde_json::Value) -> std::io::Result<()> {
        let mut line = value.to_string();
        line.push('\n');
        self.0.write_all(line.as_bytes())?;
        self.0.flush()
    }
}

impl SessionWriter for FakeWriter {
    fn input(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.line(json!({"type": "terminal.input", "bytes": protocol::encode_base64(bytes)}))
    }

    fn paste(&mut self, _text: &[u8]) -> std::io::Result<()> {
        Err(std::io::Error::other("this stand-in takes no paste"))
    }

    fn keys(&mut self, _keys: &[hide_holder_protocol::Key]) -> std::io::Result<()> {
        Err(std::io::Error::other("this stand-in takes no named keys"))
    }

    fn resize(&mut self, size: GridSize) -> std::io::Result<()> {
        self.line(json!({"type": "terminal.resize", "rows": size.rows, "cols": size.cols}))
    }

    /// Asked for the whole screen at the size it has, as a resize to it.
    fn redraw(&mut self, size: GridSize) -> std::io::Result<()> {
        self.resize(size)
    }

    fn scroll(
        &mut self,
        lines: i32,
        column: Option<u16>,
        row: Option<u16>,
        modifiers: u8,
    ) -> std::io::Result<()> {
        self.line(json!({
            "type": "terminal.scroll", "lines": lines, "column": column,
            "row": row, "modifiers": modifiers,
        }))
    }

    fn release(&mut self) -> std::io::Result<()> {
        self.line(json!({"type": "terminal.release"}))
    }

    /// Refused, as by a holder built before submissions; [`holder_like`]
    /// makes it take one whole.
    fn submit(&mut self, _submit: &hide_holder_protocol::Submit) -> std::io::Result<()> {
        Err(std::io::Error::other("this stand-in takes no submission"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

/// A stand-in session's events as the test sends them; its sender dropped
/// is the session's end.
pub(super) struct FakeEvents(pub(super) Receiver<SessionEvent>);

impl SessionEvents for FakeEvents {
    fn next_event(&mut self) -> SessionEvent {
        self.0.recv().unwrap_or(SessionEvent::Ended {
            category: "transport_eof",
            message: "The stand-in session ended".to_owned(),
        })
    }
}

/// Each key a session's writer wrote, with when it wrote it.
pub(super) type Writes = Arc<Mutex<Vec<(Instant, Vec<u8>)>>>;

/// A session's writes as the test reads them, one value per line.
pub(super) struct LineSink {
    lines: Sender<serde_json::Value>,
    pending: Vec<u8>,
    writes: Writes,
}

impl Write for LineSink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line = self.pending.drain(..=end).collect::<Vec<_>>();
            let value: serde_json::Value = serde_json::from_slice(&line[..line.len() - 1]).unwrap();
            if value["type"] == "terminal.input" {
                let bytes = protocol::decode_base64(value["bytes"].as_str().unwrap()).unwrap();
                self.writes.lock().unwrap().push((Instant::now(), bytes));
            }
            self.lines
                .send(value)
                .map_err(|_| std::io::Error::other("closed"))?;
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// One session the fake attacher opened.
pub(super) struct Opened {
    pub(super) pane: String,
    pub(super) size: GridSize,
    pub(super) output: Sender<SessionEvent>,
    pub(super) input: Receiver<serde_json::Value>,
    pub(super) writes: Writes,
}

impl Opened {
    pub(super) fn frame(&self, size: GridSize, full: bool, bytes: &[u8]) {
        self.output
            .send(SessionEvent::Frame {
                width: size.cols,
                height: size.rows,
                full,
                bytes: bytes.to_vec(),
            })
            .unwrap();
    }

    /// The session ends with `category`, as its transport names it.
    fn close(&self, category: &'static str, message: &str) {
        self.output
            .send(SessionEvent::Ended {
                category,
                message: message.to_owned(),
            })
            .unwrap();
    }

    /// The next line the session was asked to write.
    fn next_line(&self) -> serde_json::Value {
        self.input.recv_timeout(WAIT).expect("a line was written")
    }

    pub(super) fn next_input(&self) -> Vec<u8> {
        loop {
            let line = self.next_line();
            if line["type"] == "terminal.input" {
                return protocol::decode_base64(line["bytes"].as_str().unwrap()).unwrap();
            }
        }
    }

    fn assert_no_input(&self) {
        let until = Instant::now() + Duration::from_millis(100);
        while let Ok(line) = self
            .input
            .recv_timeout(until.saturating_duration_since(Instant::now()))
        {
            assert_ne!(
                line["type"], "terminal.input",
                "keys reached the launch shell before the fork finished"
            );
        }
    }
}

/// Opens a stand-in session at the size it is asked for; a pane attached
/// with no size is refused, as one that keeps no size of its own is.
pub(super) struct FakeAttacher {
    pub(super) opened: Mutex<Sender<Opened>>,
    pub(super) refuse: Mutex<Option<String>>,
}

impl Attacher for FakeAttacher {
    fn open_transport(&self, pane: &str, size: Option<GridSize>) -> Result<Transport, OpenFailure> {
        if let Some(reason) = self.refuse.lock().unwrap().clone() {
            return Err(OpenFailure::retried(reason));
        }
        let size = size.ok_or_else(|| {
            OpenFailure::retried(format!("Pane {pane} has no reported terminal size"))
        })?;
        let (output, events) = channel();
        let (written, input) = channel();
        let writes = Writes::default();
        self.opened
            .lock()
            .unwrap()
            .send(Opened {
                pane: pane.to_owned(),
                size,
                output,
                input,
                writes: Arc::clone(&writes),
            })
            .unwrap();
        Ok(Transport {
            events: Box::new(FakeEvents(events)),
            writer: Box::new(FakeWriter(Box::new(LineSink {
                lines: written,
                pending: Vec::new(),
                writes,
            }))),
            cleanup: Cleanup::None,
            size: None,
            reads_keys: false,
            submissions: session::Submissions::Refused,
        })
    }
}

/// `transport` as a holder that names `Capability::Submit` takes input: a
/// submission whole, its submit written its gap after its text, with the
/// writes after it behind it, as the holder's own tests prove of it.
pub(super) fn holder_like(mut transport: Transport) -> Transport {
    transport.writer = Box::new(Submitting(transport.writer));
    transport.submissions = session::Submissions::Whole;
    transport
}

struct Submitting(Box<dyn SessionWriter>);

impl Submitting {
    fn part(&mut self, part: &hide_holder_protocol::SubmitPart) -> std::io::Result<()> {
        match part {
            hide_holder_protocol::SubmitPart::Input(bytes)
            | hide_holder_protocol::SubmitPart::Paste(bytes) => self.0.input(bytes),
            hide_holder_protocol::SubmitPart::Keys(keys) => self.0.keys(keys),
        }
    }
}

impl SessionWriter for Submitting {
    fn input(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.0.input(bytes)
    }
    fn paste(&mut self, text: &[u8]) -> std::io::Result<()> {
        self.0.paste(text)
    }
    fn keys(&mut self, keys: &[hide_holder_protocol::Key]) -> std::io::Result<()> {
        self.0.keys(keys)
    }
    fn resize(&mut self, size: GridSize) -> std::io::Result<()> {
        self.0.resize(size)
    }
    fn redraw(&mut self, size: GridSize) -> std::io::Result<()> {
        self.0.redraw(size)
    }
    fn scroll(
        &mut self,
        lines: i32,
        column: Option<u16>,
        row: Option<u16>,
        modifiers: u8,
    ) -> std::io::Result<()> {
        self.0.scroll(lines, column, row, modifiers)
    }
    fn release(&mut self) -> std::io::Result<()> {
        self.0.release()
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
    fn submit(&mut self, submit: &hide_holder_protocol::Submit) -> std::io::Result<()> {
        for part in &submit.text {
            self.part(part)?;
        }
        #[allow(clippy::disallowed_methods)] // the holder's gap, as a test double keeps it
        thread::sleep(submit.gap);
        self.part(&submit.enter)
    }
}

#[derive(Default)]
pub(super) struct Outputs {
    written: Mutex<Vec<(String, Vec<u8>, bool)>>,
    forgotten: Mutex<Vec<String>>,
    /// The pane whose next output the sink panics on, told first.
    panic_on: Mutex<Option<(String, Sender<()>)>>,
    /// The pane whose next output the sink holds until the test lets it go
    /// on.
    hold_on: Mutex<Option<Hold>>,
}

pub(super) struct Hold {
    pane: String,
    /// Told when the sink took the output.
    told: Sender<()>,
    go_on: Receiver<()>,
}

impl OutputSink for Outputs {
    fn output(&self, pane: &str, bytes: &[u8], full: bool) {
        let panicking = self
            .panic_on
            .lock()
            .unwrap()
            .take_if(|(panicking, _)| panicking == pane);
        if let Some((_, told)) = panicking {
            let _ = told.send(());
            panic!("the sink failed on {pane}");
        }
        let holding = self
            .hold_on
            .lock()
            .unwrap()
            .take_if(|hold| hold.pane == pane);
        if let Some(hold) = holding {
            let _ = hold.told.send(());
            let _ = hold.go_on.recv();
        }
        self.written
            .lock()
            .unwrap()
            .push((pane.to_owned(), bytes.to_vec(), full));
    }

    fn forget(&self, pane: &str) {
        self.forgotten.lock().unwrap().push(pane.to_owned());
    }
}

pub(super) struct Reports(Mutex<Sender<TerminalReport>>);

impl ReportSink for Reports {
    fn report(&self, report: TerminalReport) {
        let _ = self.0.lock().unwrap().send(report);
    }
}

struct Harness {
    service: Service,
    opened: Receiver<Opened>,
    reports: Receiver<TerminalReport>,
    outputs: Arc<Outputs>,
    refuse: Arc<FakeAttacher>,
}

fn harness(policy: RetryPolicy) -> Harness {
    harness_with(policy, || Cleanup::None)
}

/// A harness whose sessions each end with what `cleanup` makes.
fn harness_with(
    policy: RetryPolicy,
    cleanup: impl Fn() -> Cleanup + Send + Sync + 'static,
) -> Harness {
    harness_taking(policy, cleanup, session::Submissions::Whole)
}

/// A harness whose sessions take submissions as `submissions` says: a
/// holder's, or a holder's built before them.
fn harness_taking(
    policy: RetryPolicy,
    cleanup: impl Fn() -> Cleanup + Send + Sync + 'static,
    submissions: session::Submissions,
) -> Harness {
    let (opened_tx, opened) = channel();
    let (reports_tx, reports) = channel();
    let attacher = Arc::new(FakeAttacher {
        opened: Mutex::new(opened_tx),
        refuse: Mutex::new(None),
    });
    let outputs = Arc::new(Outputs::default());
    struct Shared<C>(Arc<FakeAttacher>, C, session::Submissions);
    impl<C: Fn() -> Cleanup + Send + Sync> Attacher for Shared<C> {
        fn open_transport(
            &self,
            pane: &str,
            size: Option<GridSize>,
        ) -> Result<Transport, OpenFailure> {
            let mut transport = self.0.open_transport(pane, size)?;
            transport.cleanup = (self.1)();
            Ok(match self.2 {
                session::Submissions::Whole => holder_like(transport),
                session::Submissions::Refused => transport,
            })
        }
    }
    let service = Service::start(
        Box::new(Shared(Arc::clone(&attacher), cleanup, submissions)),
        Arc::clone(&outputs) as Arc<dyn OutputSink>,
        Arc::new(Reports(Mutex::new(reports_tx))),
        policy,
    )
    .unwrap();
    Harness {
        service,
        opened,
        reports,
        outputs,
        refuse: attacher,
    }
}

const SIZE: GridSize = GridSize { rows: 24, cols: 80 };

impl Harness {
    fn attach(&self, pane: &str, size: Option<GridSize>) {
        self.service.control(TerminalControl::Attach {
            pane: pane.into(),
            size,
            manual: false,
        });
    }

    fn opened(&self) -> Opened {
        self.opened.recv_timeout(WAIT).expect("a session opened")
    }

    /// Reports until one matches, failing after the wait.
    fn report_where(&self, mut matches: impl FnMut(&TerminalReport) -> bool) -> TerminalReport {
        loop {
            let report = self.reports.recv_timeout(WAIT).expect("a report");
            if matches(&report) {
                return report;
            }
        }
    }

    fn state(&self, pane: &str, wanted: &str) -> PaneTerminalState {
        match self.report_where(|report| {
            matches!(report, TerminalReport::State { pane: p, state } if p == pane && state.state == wanted)
        }) {
            TerminalReport::State { state, .. } => state,
            _ => unreachable!(),
        }
    }

    /// Attaches `pane` in control and returns its session.
    fn controlling(&self, pane: &str) -> Opened {
        self.attach(pane, Some(SIZE));
        let opened = self.opened();
        self.state(pane, "controlling");
        opened
    }

    fn written(&self) -> Vec<(String, Vec<u8>, bool)> {
        self.outputs.written.lock().unwrap().clone()
    }

    fn wait_written(&self, count: usize) -> Vec<(String, Vec<u8>, bool)> {
        let started = Instant::now();
        loop {
            let written = self.written();
            if written.len() >= count {
                return written;
            }
            assert!(
                started.elapsed() < WAIT,
                "output did not arrive: {written:?}"
            );
            // The reader thread delivers on its own; wait for its report.
            let _ = self.reports.recv_timeout(Duration::from_millis(20));
        }
    }

    fn key(&self, pane: &str, bytes: &[u8]) {
        self.service
            .key(KeyTarget::Pane(pane.into()), bytes.to_vec(), unix_ms());
    }
}

#[test]
fn an_attach_without_a_size_waits_for_one_then_attaches_at_it() {
    let harness = harness(RetryPolicy::Automatic);
    harness.attach("w1:p1", None);
    harness.report_where(|report| {
        matches!(report, TerminalReport::Note { kind, .. } if kind == "terminal.attach_deferred")
    });
    let waiting = harness.state("w1:p1", "waiting_size");
    assert!(waiting.message.unwrap().contains("Waiting"));
    harness.service.control(TerminalControl::Resize {
        pane: "w1:p1".into(),
        size: GridSize {
            rows: 30,
            cols: 100,
        },
    });
    let opened = harness.opened();
    assert_eq!(
        opened.size,
        GridSize {
            rows: 30,
            cols: 100
        }
    );
    harness.state("w1:p1", "controlling");
    // The settled size is sent to the session it opened.
    assert_eq!(opened.next_line()["type"], "terminal.resize");
}

#[test]
fn a_key_reaches_the_writer_and_an_enter_is_reported_at_once() {
    let harness = harness(RetryPolicy::Automatic);
    let opened = harness.controlling("w1:p1");
    harness.key("w1:p1", b"ls");
    assert_eq!(opened.next_input(), b"ls");
    let first = harness.report_where(|report| matches!(report, TerminalReport::Input { .. }));
    assert!(matches!(
        first,
        TerminalReport::Input {
            submitted: false,
            focus: true,
            ..
        }
    ));
    harness.key("w1:p1", b"\r");
    assert_eq!(opened.next_input(), b"\r");
    let enter = harness.report_where(|report| matches!(report, TerminalReport::Input { .. }));
    assert!(matches!(
        enter,
        TerminalReport::Input {
            submitted: true,
            ..
        }
    ));
}

#[test]
fn a_pasted_return_or_an_escaped_one_is_not_a_submit() {
    let harness = harness(RetryPolicy::Automatic);
    let opened = harness.controlling("w1:p1");
    harness.key("w1:p1", b"\x1b[200~one\rtwo\x1b[201~");
    opened.next_input();
    let pasted = harness.report_where(|report| matches!(report, TerminalReport::Input { .. }));
    assert!(matches!(
        pasted,
        TerminalReport::Input {
            submitted: false,
            ..
        }
    ));
    // Inside the window an escaped return is still not reported as one.
    harness.key("w1:p1", b"\x1b\r");
    opened.next_input();
    harness.key("w1:p1", b"\r");
    opened.next_input();
    let enter = harness.report_where(|report| matches!(report, TerminalReport::Input { .. }));
    assert!(matches!(
        enter,
        TerminalReport::Input {
            submitted: true,
            ..
        }
    ));
}

#[test]
fn keys_typed_while_the_session_opens_are_written_first_in_order() {
    let harness = harness(RetryPolicy::Automatic);
    harness.attach("w1:p1", Some(SIZE));
    harness.key("w1:p1", b"one");
    harness.key("w1:p1", b"two");
    let opened = harness.opened();
    // The session may open on its own thread before or between the keys;
    // either way they are written first and in order.
    let mut written = Vec::new();
    while written.len() < b"onetwo".len() {
        written.extend(opened.next_input());
    }
    assert_eq!(written, b"onetwo");
    harness.key("w1:p1", b"three");
    assert_eq!(opened.next_input(), b"three");
}

#[test]
fn a_frame_at_a_foreign_grid_waits_for_a_full_frame_at_the_view_grid() {
    let harness = harness(RetryPolicy::Automatic);
    let opened = harness.controlling("w1:p1");
    harness.service.view("w1:p1", SIZE, true);
    opened.frame(GridSize { rows: 10, cols: 10 }, true, b"foreign");
    opened.frame(SIZE, false, b"partial");
    opened.frame(SIZE, true, b"whole");
    opened.frame(SIZE, false, b"next");
    let written = harness.wait_written(2);
    assert_eq!(written[0].1, b"\x1bcwhole".to_vec());
    assert!(written[0].2, "a reset frame draws the whole screen");
    assert_eq!(written[1].1, b"next".to_vec());
    assert!(!written[1].2);
}

/// A control session asks Herdr for the grid the view reported, the one
/// the frame guard accepts, not an older settled size.
#[test]
fn a_control_session_attaches_at_the_grid_the_view_reported() {
    let harness = harness(RetryPolicy::Automatic);
    let view = GridSize { rows: 18, cols: 41 };
    harness.service.view("w1:p1", view, true);
    harness.attach("w1:p1", Some(GridSize { rows: 25, cols: 50 }));
    let opened = harness.opened();
    assert_eq!(opened.size, view);
    opened.frame(view, true, b"attach frame");
    assert_eq!(harness.wait_written(1)[0].1, b"\x1bcattach frame".to_vec());
}

/// A new session for a pane on screen draws nothing until a full frame at
/// the pane's grid arrives, so the last screen stays up instead of a blank.
#[test]
fn retry_preserves_the_canvas_until_a_matching_replacement_frame() {
    let harness = harness(RetryPolicy::Automatic);
    let opened = harness.controlling("w1:p1");
    opened.frame(SIZE, true, b"last valid screen");
    harness.wait_written(1);
    harness.service.control(TerminalControl::Attach {
        pane: "w1:p1".into(),
        size: Some(SIZE),
        manual: true,
    });
    let second = harness.opened();
    second.frame(GridSize { rows: 9, cols: 2 }, true, b"wrong grid");
    second.frame(SIZE, false, b"partial");
    second.frame(SIZE, true, b"replacement");
    let written = harness.wait_written(2);
    assert_eq!(
        written.len(),
        2,
        "nothing was drawn in between: {written:?}"
    );
    assert_eq!(written[1].1, b"\x1bcreplacement".to_vec());
}

/// R6, R7. A pane keeps its session while its canvas is rebuilt, and the
/// view that comes back has an empty grid; Herdr sends nothing more until
/// the pane produces output. A size that did not change is not sent on, but
/// one reported after a new view is, so Herdr draws a full frame for it.
#[test]
fn a_rebuilt_view_for_an_attached_pane_is_given_a_frame_to_draw() {
    let harness = harness(RetryPolicy::Automatic);
    let opened = harness.controlling("w1:p1");
    assert_eq!(opened.next_line()["type"], "terminal.resize");
    opened.frame(SIZE, true, b"first");
    harness.wait_written(1);
    let resize = || {
        harness.service.control(TerminalControl::Resize {
            pane: "w1:p1".into(),
            size: SIZE,
        })
    };
    resize();
    assert!(
        opened
            .input
            .recv_timeout(Duration::from_millis(100))
            .is_err(),
        "a size that did not change was forwarded to Herdr"
    );
    harness.service.view("w1:p1", SIZE, true);
    resize();
    let line = opened.next_line();
    assert_eq!(
        (
            line["type"].clone(),
            line["rows"].clone(),
            line["cols"].clone()
        ),
        (json!("terminal.resize"), json!(24), json!(80))
    );
}

/// A sink that panics on one pane's output stops only the thread that
/// handed it over: every other pane's output still reaches the sink.
#[test]
fn a_sink_that_panics_on_one_output_does_not_stop_the_others() {
    let harness = harness(RetryPolicy::Automatic);
    let first = harness.controlling("w1:p1");
    let second = harness.controlling("w1:p2");
    let (told, panicked) = channel();
    *harness.outputs.panic_on.lock().unwrap() = Some(("w1:p1".into(), told));
    first.frame(SIZE, true, b"lost");
    panicked
        .recv_timeout(WAIT)
        .expect("the sink was handed the output");
    second.frame(SIZE, true, b"kept");
    let written = harness.wait_written(1);
    assert_eq!(written[0].0, "w1:p2");
    assert!(written[0].1.ends_with(b"kept"), "{written:?}");
}

/// A control from the core never waits on output: while the sink is busy
/// with one pane's frame, releasing another pane returns at once, and its
/// output is forgotten once the sink is free.
#[test]
fn a_control_returns_while_the_sink_is_busy_with_another_panes_frame() {
    let harness = harness(RetryPolicy::Automatic);
    let first = harness.controlling("w1:p1");
    let _second = harness.controlling("w1:p2");
    let (told, entered) = channel();
    let (go_on, held) = channel();
    *harness.outputs.hold_on.lock().unwrap() = Some(Hold {
        pane: "w1:p1".into(),
        told,
        go_on: held,
    });
    first.frame(SIZE, true, b"large");
    entered.recv_timeout(WAIT).expect("the sink took the frame");
    let service = Arc::clone(&harness.service.shared);
    let (returned, released) = channel();
    thread::spawn(move || {
        service.run(|inner, shared| {
            inner.control(
                shared,
                TerminalControl::Release {
                    pane: "w1:p2".into(),
                    message: "detached".into(),
                },
            )
        });
        let _ = returned.send(());
    });
    let waited = released.recv_timeout(Duration::from_secs(2));
    go_on.send(()).unwrap();
    waited.expect("the release waited for another pane's frame");
    let started = Instant::now();
    while !harness
        .outputs
        .forgotten
        .lock()
        .unwrap()
        .contains(&"w1:p2".to_owned())
    {
        assert!(
            started.elapsed() < WAIT,
            "the release's forget never reached the sink"
        );
        thread::yield_now();
    }
}

/// Two readers printing at once each hand over only their own frames: once
/// one reader's frame was handed, it goes back to reading its pane, even
/// while the sink is busy with the other reader's frame that queued behind
/// it.
#[test]
fn a_reader_goes_back_to_its_pane_while_another_readers_frame_is_handed() {
    let harness = harness(RetryPolicy::Automatic);
    let first = harness.controlling("w1:p1");
    let second = harness.controlling("w1:p2");
    let hold = |pane: &str| {
        let (told, entered) = channel();
        let (go_on, held) = channel();
        *harness.outputs.hold_on.lock().unwrap() = Some(Hold {
            pane: pane.into(),
            told,
            go_on: held,
        });
        (entered, go_on)
    };
    let (first_entered, first_go_on) = hold("w1:p1");
    first.frame(SIZE, true, b"first");
    first_entered
        .recv_timeout(WAIT)
        .expect("the sink took the first pane's frame");
    second.frame(SIZE, true, b"second");
    let started = Instant::now();
    while harness.service.shared.deliveries_lock().queue.is_empty() {
        assert!(
            started.elapsed() < WAIT,
            "the second pane's frame never queued"
        );
        thread::yield_now();
    }
    let (second_entered, second_go_on) = hold("w1:p2");
    first_go_on.send(()).unwrap();
    second_entered
        .recv_timeout(WAIT)
        .expect("the sink took the second pane's frame");
    // The first pane's reader reads on while the second frame is in the
    // sink: its session's end reaches the core.
    first.close("holder_failed", "pane exited");
    let deadline = Instant::now() + Duration::from_secs(2);
    let ended = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match harness.reports.recv_timeout(left) {
            Ok(TerminalReport::State { pane, state })
                if pane == "w1:p1" && state.state == "ended" =>
            {
                break true;
            }
            Ok(_) => {}
            Err(_) => break false,
        }
    };
    second_go_on.send(()).unwrap();
    assert!(
        ended,
        "the first pane's reader was handing the second pane's frame"
    );
}

/// A pane told it is closing before this node tracks it (a device's new
/// link is told so before the core attaches the pane) refuses keys as a
/// closing pane, not as one with no terminal.
#[test]
fn a_pane_told_closing_before_this_node_tracks_it_refuses_keys_as_closing() {
    let harness = harness(RetryPolicy::Automatic);
    harness.service.control(TerminalControl::Closing {
        pane: "w1:p1".into(),
        closing: true,
    });
    harness.key("w1:p1", b"x");
    let refused = harness.report_where(|report| matches!(report, TerminalReport::Error { .. }));
    assert!(
        matches!(&refused, TerminalReport::Error { kind, .. } if kind == "terminal.close_pending"),
        "{refused:?}"
    );
}

#[test]
fn an_ended_session_says_so_on_the_pane_and_a_closing_one_does_not() {
    let harness = harness(RetryPolicy::Automatic);
    let opened = harness.controlling("w1:p1");
    opened.close("holder_failed", "pane exited");
    let state = harness.state("w1:p1", "ended");
    assert_eq!(state.exit_category.as_deref(), Some("holder_failed"));
    assert!(state.message.unwrap().contains("Retrying in 5 seconds"));
    let written = harness.wait_written(1);
    assert_eq!(written[0].1, b"\r\n[pane exited]\r\n".to_vec());

    let second = harness.controlling("w1:p2");
    harness.service.control(TerminalControl::Closing {
        pane: "w1:p2".into(),
        closing: true,
    });
    harness.key("w1:p2", b"x");
    harness.report_where(|report| {
        matches!(report, TerminalReport::Error { kind, .. } if kind == "terminal.close_pending")
    });
    second.close("transport_eof", "The pane's holder connection ended");
    harness.state("w1:p2", "closing");
    assert_eq!(
        harness.written().len(),
        1,
        "nothing is drawn over a closing pane"
    );
}

#[test]
fn an_attach_to_a_pane_already_attached_is_answered_with_its_state() {
    let harness = harness(RetryPolicy::Automatic);
    let _opened = harness.controlling("w1:p1");
    // The core asks for the attach the pane's start already made.
    harness.attach("w1:p1", Some(SIZE));
    harness.state("w1:p1", "controlling");
}

#[test]
fn a_released_pane_ends_its_session_and_its_output_is_forgotten() {
    let harness = harness(RetryPolicy::Automatic);
    let _opened = harness.controlling("w1:p1");
    harness.service.control(TerminalControl::Release {
        pane: "w1:p1".into(),
        message: "detached".into(),
    });
    let state = harness.state("w1:p1", "released");
    assert_eq!(state.retry_decision, "on_next_visit");
    assert_eq!(
        harness.outputs.forgotten.lock().unwrap().as_slice(),
        ["w1:p1"]
    );
    // It attaches again when it is shown again.
    harness.attach("w1:p1", Some(SIZE));
    harness.opened();
}

#[test]
fn keys_for_a_sleeping_pane_are_dropped_until_it_wakes() {
    let harness = harness(RetryPolicy::Automatic);
    let opened = harness.controlling("w1:p1");
    harness.service.control(TerminalControl::Asleep {
        pane: "w1:p1".into(),
        asleep: true,
    });
    harness.key("w1:p1", b"lost");
    harness.service.control(TerminalControl::Asleep {
        pane: "w1:p1".into(),
        asleep: false,
    });
    harness.key("w1:p1", b"kept");
    assert_eq!(opened.next_input(), b"kept");
}

/// A dropped file's path and the bytes a screen answers its terminal with are
/// as much typing as a key: neither reaches the shell under a sleeping agent,
/// and a paste already held waits for the agent with what was typed behind it.
#[test]
fn a_paste_and_control_bytes_for_a_sleeping_pane_wait_for_the_agent() {
    let harness = harness(RetryPolicy::Automatic);
    let opened = harness.controlling("w1:p1");
    let generation = harness.service.shared.lock().panes["w1:p1"]
        .current_generation
        .unwrap();
    harness.service.control(TerminalControl::AttachmentHold {
        pane: "w1:p1".into(),
        intent: "i1".into(),
    });
    harness.key("w1:p1", b"\r");
    harness.service.control(TerminalControl::Asleep {
        pane: "w1:p1".into(),
        asleep: true,
    });
    let deliver = |harness: &Harness| {
        harness.service.control(TerminalControl::AttachmentDeliver {
            intent: "i1".into(),
            generation,
            paste: protocol::encode_base64(b"'/tmp/a.png' "),
        });
    };
    deliver(&harness);
    harness.report_where(|report| {
        matches!(
            report,
            TerminalReport::AttachmentDelivered { written: false, .. }
        )
    });
    harness.service.control(TerminalControl::Write {
        pane: "w1:p1".into(),
        data: protocol::encode_base64(b"\x1b[?1;2c"),
    });
    harness.service.control(TerminalControl::Asleep {
        pane: "w1:p1".into(),
        asleep: false,
    });
    deliver(&harness);
    // Only the paste and the key typed before the agent slept arrive, and in order.
    assert_eq!(opened.next_input(), b"'/tmp/a.png' \r");
}

#[test]
fn keys_sent_before_the_core_names_their_creation_reach_the_pane_it_makes() {
    let harness = harness(RetryPolicy::Automatic);
    harness.service.key(
        KeyTarget::Request("req-1".into()),
        b"echo".to_vec(),
        unix_ms(),
    );
    harness.service.control(TerminalControl::RequestOpen {
        request: "req-1".into(),
        fork: false,
    });
    harness.service.key(
        KeyTarget::Request("req-1".into()),
        b" hi\r".to_vec(),
        unix_ms(),
    );
    harness.service.control(TerminalControl::RequestResolve {
        request: "req-1".into(),
        pane: "w1:p5".into(),
    });
    harness.attach("w1:p5", Some(SIZE));
    let opened = harness.opened();
    assert_eq!(opened.pane, "w1:p5");
    assert_eq!(opened.next_input(), b"echo hi\r");
}

#[test]
fn bare_pane_keys_wait_for_the_fork_and_reach_only_its_terminal() {
    let harness = harness(RetryPolicy::Automatic);
    let fork = harness.controlling("w1:p2");
    harness.service.control(TerminalControl::RequestOpen {
        request: "fork-1".into(),
        fork: true,
    });
    harness.service.control(TerminalControl::RequestHold {
        request: "fork-1".into(),
        pane: "w1:p2".into(),
    });
    harness.key("w1:p2", b"hello\r");
    fork.assert_no_input();
    harness.service.control(TerminalControl::RequestResolve {
        request: "fork-1".into(),
        pane: "w1:p2".into(),
    });
    assert_eq!(fork.next_input(), b"hello\r");
    harness.key("w1:p2", b"next\r");
    assert_eq!(fork.next_input(), b"next\r");
}

/// Opens a fork request holding keys for `pane` and types `keys` into it,
/// one key each, as the operator typed them while the fork started.
fn fork_holding(harness: &Harness, pane: &str, keys: &[&[u8]]) {
    harness.service.control(TerminalControl::RequestOpen {
        request: "fork-1".into(),
        fork: true,
    });
    harness.service.control(TerminalControl::RequestHold {
        request: "fork-1".into(),
        pane: pane.into(),
    });
    for key in keys {
        harness.key(pane, key);
    }
}

/// The keys `opened`'s writer wrote, each with when, once they amount to
/// `expected`.
fn written(opened: &Opened, expected: &[u8]) -> Vec<(Instant, Vec<u8>)> {
    let until = Instant::now() + WAIT;
    loop {
        let writes = opened.writes.lock().unwrap().clone();
        if writes.iter().map(|(_, bytes)| bytes.len()).sum::<usize>() >= expected.len() {
            assert_eq!(writes.concat_bytes(), expected);
            return writes;
        }
        assert!(Instant::now() < until, "the writer wrote {writes:?}");
        std::thread::yield_now();
    }
}

trait ConcatBytes {
    fn concat_bytes(&self) -> Vec<u8>;
}

impl ConcatBytes for Vec<(Instant, Vec<u8>)> {
    fn concat_bytes(&self) -> Vec<u8> {
        self.iter().flat_map(|(_, bytes)| bytes.clone()).collect()
    }
}

/// When the writer wrote the key that holds `byte` of everything written.
fn written_at(writes: &[(Instant, Vec<u8>)], byte: usize) -> Instant {
    let mut seen = 0;
    for (at, bytes) in writes {
        seen += bytes.len();
        if seen > byte {
            return *at;
        }
    }
    panic!("byte {byte} was not written");
}

#[test]
fn a_held_enter_reaches_the_fork_a_moment_after_its_writer_wrote_the_keys_before_it() {
    // Codex reads an Enter that arrives with the keys before it as a
    // newline of a paste; the operator had pressed it after a pause. As
    // Herdr's `agent.prompt` waits from its text's flush, the gap is
    // measured between the writer's own writes, and a key typed while the
    // Enter waits goes behind it.
    let harness = harness(RetryPolicy::Automatic);
    let fork = harness.controlling("w1:p2");
    fork_holding(&harness, "w1:p2", &[b"hello", b"\r", b"more"]);
    harness.service.control(TerminalControl::RequestResolve {
        request: "fork-1".into(),
        pane: "w1:p2".into(),
    });
    harness.key("w1:p2", b"!");
    let writes = written(&fork, b"hello\rmore!");
    assert!(written_at(&writes, 5) >= written_at(&writes, 4) + input::REPLAYED_SUBMIT_GAP);
}

#[test]
fn a_held_enter_waits_for_an_attaching_forks_writer_to_write_the_keys_before_it() {
    // The fork's terminal attaches after its keys were released: the queue
    // writes nothing until it has, and the gap runs from the writer's write.
    let harness = harness(RetryPolicy::Automatic);
    fork_holding(&harness, "w1:p2", &[b"hello", b"\r"]);
    harness.service.control(TerminalControl::RequestResolve {
        request: "fork-1".into(),
        pane: "w1:p2".into(),
    });
    harness.attach("w1:p2", Some(SIZE));
    let opened = harness.opened();
    let writes = written(&opened, b"hello\r");
    assert!(written_at(&writes, 5) >= written_at(&writes, 4) + input::REPLAYED_SUBMIT_GAP);
}

#[test]
fn a_letter_typed_into_the_fork_keeps_the_released_keys_behind_it_in_order() {
    // A letter's hold and the replay queue never split the operator's keys:
    // the queue writes nothing while a letter is typed and keeps the order
    // typed, the Enter apart from the keys before it.
    let harness = harness(RetryPolicy::Automatic);
    let fork = harness.controlling("w1:p2");
    harness.service.shared.run(|inner, _| {
        inner
            .panes
            .get_mut("w1:p2")
            .unwrap()
            .letter
            .get_or_insert_with(LetterHold::default)
            .letters += 1;
    });
    fork_holding(&harness, "w1:p2", &[b"hello", b"\r"]);
    harness.service.control(TerminalControl::RequestResolve {
        request: "fork-1".into(),
        pane: "w1:p2".into(),
    });
    harness.key("w1:p2", b"!");
    fork.assert_no_input();
    harness.service.shared.run(|inner, _| {
        inner.panes.get_mut("w1:p2").unwrap().letter = None;
        inner.pump_replay("w1:p2", Instant::now());
    });
    let writes = written(&fork, b"hello\r!");
    assert!(written_at(&writes, 5) >= written_at(&writes, 4) + input::REPLAYED_SUBMIT_GAP);
}

/// Holds `pane`'s keys for one letter, as `type_letter` begins one, so a
/// test ends it with `Inner::end_letter` when it chooses.
fn begin_letter(harness: &Harness, pane: &str) {
    harness.service.shared.run(|inner, _| {
        inner
            .panes
            .get_mut(pane)
            .unwrap()
            .letter
            .get_or_insert_with(LetterHold::default)
            .letters += 1;
    });
}

#[test]
fn keys_a_letter_held_go_ahead_of_a_forks_keys_released_during_it() {
    // "x" was typed during the letter, before the fork held the pane, so
    // it is written before the fork's keys when the letter ends.
    let harness = harness(RetryPolicy::Automatic);
    let fork = harness.controlling("w1:p2");
    begin_letter(&harness, "w1:p2");
    harness.key("w1:p2", b"x");
    fork_holding(&harness, "w1:p2", &[b"hello", b"\r"]);
    harness.service.control(TerminalControl::RequestResolve {
        request: "fork-1".into(),
        pane: "w1:p2".into(),
    });
    fork.assert_no_input();
    harness
        .service
        .shared
        .run(|inner, _| inner.end_letter("w1:p2", Instant::now()));
    let writes = written(&fork, b"xhello\r");
    assert!(written_at(&writes, 6) >= written_at(&writes, 5) + input::REPLAYED_SUBMIT_GAP);
}

#[test]
fn a_letter_ending_without_held_keys_keeps_a_fork_queue_that_dropped_keys_past_its_cap() {
    // "z" crossed the queue's cap and was reported; the keys queued before
    // it are still written once the letter ends.
    let harness = harness(RetryPolicy::Automatic);
    let fork = harness.controlling("w1:p2");
    begin_letter(&harness, "w1:p2");
    let full = vec![b'y'; hide_node_link::terminal::INPUT_HOLD_LIMIT_BYTES];
    fork_holding(&harness, "w1:p2", &[&full]);
    harness.service.control(TerminalControl::RequestResolve {
        request: "fork-1".into(),
        pane: "w1:p2".into(),
    });
    harness.key("w1:p2", b"z");
    harness.report_where(|report| {
        matches!(report, TerminalReport::RequestDiscarded { request, reason }
            if request == "fork-1" && reason == "limit")
    });
    harness
        .service
        .shared
        .run(|inner, _| inner.end_letter("w1:p2", Instant::now()));
    assert_eq!(input_bytes(&fork, full.len()), full);
}

#[test]
fn keys_a_letter_held_ahead_of_a_full_fork_queue_drop_the_latest_typed_and_report_it() {
    let harness = harness(RetryPolicy::Automatic);
    let fork = harness.controlling("w1:p2");
    begin_letter(&harness, "w1:p2");
    harness.key("w1:p2", b"x");
    let full = vec![b'y'; hide_node_link::terminal::INPUT_HOLD_LIMIT_BYTES];
    fork_holding(&harness, "w1:p2", &[&full]);
    harness.service.control(TerminalControl::RequestResolve {
        request: "fork-1".into(),
        pane: "w1:p2".into(),
    });
    harness
        .service
        .shared
        .run(|inner, _| inner.end_letter("w1:p2", Instant::now()));
    harness.report_where(|report| {
        matches!(report, TerminalReport::RequestDiscarded { request, reason }
            if request == "fork-1" && reason == "limit")
    });
    assert_eq!(input_bytes(&fork, 1), b"x");
    fork.assert_no_input();
}

#[test]
fn a_holder_built_before_submissions_drops_a_forks_released_keys_and_reports_them() {
    let harness = harness_taking(
        RetryPolicy::Automatic,
        || Cleanup::None,
        session::Submissions::Refused,
    );
    let fork = harness.controlling("w1:p2");
    fork_holding(&harness, "w1:p2", &[b"hello", b"\r"]);
    harness.service.control(TerminalControl::RequestResolve {
        request: "fork-1".into(),
        pane: "w1:p2".into(),
    });
    harness.report_where(|report| {
        matches!(report, TerminalReport::RequestDiscarded { request, reason }
            if request == "fork-1" && reason == "unsupported")
    });
    fork.assert_no_input();
}

#[test]
fn released_keys_whose_submission_would_outlast_the_holds_age_are_dropped_and_reported() {
    // Herdr's Windows submission: one that cannot finish its gap before its
    // deadline is not started, so no text is left without its submit.
    let harness = harness(RetryPolicy::Automatic);
    fork_holding(&harness, "w1:p2", &[b"hello", b"\r"]);
    harness.service.control(TerminalControl::RequestResolve {
        request: "fork-1".into(),
        pane: "w1:p2".into(),
    });
    // The fork's terminal attaches with less than a gap of the hold left.
    harness.service.shared.run(|inner, _| {
        let replay = inner
            .panes
            .get_mut("w1:p2")
            .unwrap()
            .replay
            .as_mut()
            .unwrap();
        replay.ends = Instant::now() + input::REPLAYED_SUBMIT_GAP / 2;
    });
    harness.attach("w1:p2", Some(SIZE));
    let opened = harness.opened();
    harness.report_where(|report| {
        matches!(report, TerminalReport::RequestDiscarded { request, reason }
            if request == "fork-1" && reason == "timeout")
    });
    opened.assert_no_input();
}

#[test]
fn released_keys_whose_fork_never_takes_them_are_dropped_and_reported_at_the_holds_age() {
    let harness = harness(RetryPolicy::Automatic);
    fork_holding(&harness, "w1:p2", &[b"hello", b"\r"]);
    harness.service.control(TerminalControl::RequestResolve {
        request: "fork-1".into(),
        pane: "w1:p2".into(),
    });
    let deadline = harness
        .service
        .shared
        .lock()
        .next_deadline()
        .expect("the released keys own a deadline");
    assert!(deadline <= Instant::now() + input::HELD_INPUT_MAX_AGE);
    harness.service.shared.run(|inner, shared| {
        inner.tick(shared, deadline);
    });
    assert!(
        harness.service.shared.lock().panes["w1:p2"]
            .replay
            .is_none()
    );
    harness.report_where(|report| {
        matches!(report, TerminalReport::RequestDiscarded { request, reason }
            if request == "fork-1" && reason == "timeout")
    });
}

#[test]
fn unconfirmed_fork_input_is_discarded_and_reported_only_when_keys_were_held() {
    for typed in [true, false] {
        let harness = harness(RetryPolicy::Automatic);
        let fork = harness.controlling("w1:p2");
        harness.service.control(TerminalControl::RequestOpen {
            request: "fork-1".into(),
            fork: true,
        });
        harness.service.control(TerminalControl::RequestHold {
            request: "fork-1".into(),
            pane: "w1:p2".into(),
        });
        if typed {
            harness.key("w1:p2", b"lost");
        }
        harness.service.control(TerminalControl::RequestDiscard {
            request: "fork-1".into(),
            reason: "unconfirmed".into(),
        });
        fork.assert_no_input();
        // The pane survives and accepts a fresh intent after the deadline.
        harness.key("w1:p2", b"retry\r");
        assert_eq!(fork.next_input(), b"retry\r");
        let mut lost = 0;
        loop {
            match harness.reports.recv_timeout(WAIT).expect("input report") {
                TerminalReport::RequestDiscarded { request, reason }
                    if request == "fork-1" && reason == "unconfirmed" =>
                {
                    lost += 1
                }
                TerminalReport::Input {
                    submitted: true, ..
                } => break,
                _ => {}
            }
        }
        assert_eq!(lost, usize::from(typed));
    }
}

#[test]
fn fork_caps_report_failure_and_refuse_the_tail_until_completion() {
    for timed_out in [false, true] {
        let harness = harness(RetryPolicy::Automatic);
        let fork = harness.controlling("w1:p2");
        harness.service.control(TerminalControl::RequestOpen {
            request: "fork-1".into(),
            fork: true,
        });
        harness.service.control(TerminalControl::RequestHold {
            request: "fork-1".into(),
            pane: "w1:p2".into(),
        });
        if timed_out {
            harness.key("w1:p2", b"lost");
            harness.service.shared.run(|inner, shared| {
                inner.tick(shared, Instant::now() + input::FORK_HOLD_MAX_AGE);
            });
        } else {
            harness.key(
                "w1:p2",
                &vec![b'x'; hide_node_link::terminal::INPUT_HOLD_LIMIT_BYTES],
            );
            harness.key("w1:p2", b"overflow");
        }
        let reason = if timed_out { "timeout" } else { "limit" };
        harness.report_where(|report| {
            matches!(report,
            TerminalReport::RequestDiscarded { request, reason: actual }
                if request == "fork-1" && actual == reason)
        });
        harness.key("w1:p2", b"tail");
        fork.assert_no_input();
        harness.service.control(TerminalControl::RequestResolve {
            request: "fork-1".into(),
            pane: "w1:p2".into(),
        });
        harness.key("w1:p2", b"retry\r");
        assert_eq!(fork.next_input(), b"retry\r");
    }
}

#[test]
fn a_paste_is_written_before_the_keys_typed_behind_it() {
    let harness = harness(RetryPolicy::Automatic);
    let opened = harness.controlling("w1:p1");
    let generation = harness.service.shared.lock().panes["w1:p1"]
        .current_generation
        .unwrap();
    harness.service.control(TerminalControl::AttachmentHold {
        pane: "w1:p1".into(),
        intent: "i1".into(),
    });
    harness.key("w1:p1", b"\r");
    harness.service.control(TerminalControl::AttachmentDeliver {
        intent: "i1".into(),
        generation,
        paste: protocol::encode_base64(b"'/tmp/a.png' "),
    });
    assert_eq!(opened.next_input(), b"'/tmp/a.png' \r");
    harness.report_where(|report| {
        matches!(
            report,
            TerminalReport::AttachmentDelivered { written: true, .. }
        )
    });
}

/// B1 at the node: once the core says the keyboard left a pane, however it
/// left, the next key into that pane is reported as moving it back at once;
/// a key into the pane the keyboard is in moves nothing.
#[test]
fn a_key_into_a_pane_the_keyboard_left_without_a_key_moves_it_back() {
    let harness = harness(RetryPolicy::Automatic);
    let _first = harness.controlling("w1:p1");
    let _second = harness.controlling("w1:p2");
    let moved_to = |harness: &Harness, wanted: &str| {
        harness.report_where(|report| {
            matches!(report, TerminalReport::Input { pane, focus: true, .. } if pane == wanted)
        });
    };
    harness.key("w1:p1", b"a");
    moved_to(&harness, "w1:p1");
    harness.key("w1:p1", b"b");
    assert!(
        !harness
            .reports
            .recv_timeout(Duration::from_millis(100))
            .is_ok_and(|report| matches!(report, TerminalReport::Input { focus: true, .. })),
        "a key into the keyboard's pane moved it"
    );
    // Herdr moved the keyboard to p2 on its own; the screen still types
    // into p1.
    harness.service.control(TerminalControl::Focus {
        pane: Some("w1:p2".into()),
    });
    harness.key("w1:p1", b"c");
    moved_to(&harness, "w1:p1");
}

#[test]
fn a_failed_attach_is_retried_while_shown_and_reported_with_its_reason() {
    let harness = harness(RetryPolicy::Automatic);
    *harness.refuse.refuse.lock().unwrap() = Some("no herdr".into());
    harness.service.control(TerminalControl::Shown {
        panes: vec!["w1:p1".into()],
    });
    harness.attach("w1:p1", Some(SIZE));
    let state = harness.state("w1:p1", "unavailable");
    assert_eq!(state.exit_category.as_deref(), Some("spawn_failed"));
    assert_eq!(state.retry_decision, "automatic_bounded");
    assert!(state.message.unwrap().contains("Retrying in 5 seconds"));
    // The reason is written into that pane's own output once, where the
    // terminal would have been.
    let written = harness.wait_written(1);
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].0, "w1:p1");
    assert!(String::from_utf8_lossy(&written[0].1).contains("no herdr"));
    let due = harness.service.shared.lock().panes["w1:p1"]
        .recovery
        .as_ref()
        .and_then(|recovery| recovery.due)
        .unwrap();
    *harness.refuse.refuse.lock().unwrap() = None;
    harness
        .service
        .shared
        .run(|inner, shared| inner.tick(shared, due));
    harness.opened();
    let state = harness.state("w1:p1", "controlling");
    assert_eq!(state.attempt, 2);
}

/// B10: keys held for a pane whose retries run out are dropped, and keys
/// typed after that are not kept for a later Reconnect.
#[test]
fn input_held_for_a_pane_whose_retries_ran_out_is_not_written_on_reconnect() {
    let harness = harness(RetryPolicy::Automatic);
    *harness.refuse.refuse.lock().unwrap() = Some("no herdr".into());
    harness.service.control(TerminalControl::Shown {
        panes: vec!["w1:p1".into()],
    });
    harness.attach("w1:p1", Some(SIZE));
    harness.state("w1:p1", "unavailable");
    harness.key("w1:p1", b"make deploy");
    let now = Instant::now();
    {
        let mut inner = harness.service.shared.lock();
        let recovery = inner
            .panes
            .get_mut("w1:p1")
            .and_then(|entry| entry.recovery.as_mut())
            .unwrap();
        recovery.retries = RETRY_SECONDS.len();
        recovery.due = Some(now);
    }
    harness
        .service
        .shared
        .run(|inner, shared| inner.tick(shared, now));
    let exhausted = harness.state("w1:p1", "unavailable");
    assert_eq!(exhausted.retry_decision, "manual");
    harness.key("w1:p1", b"\r");
    *harness.refuse.refuse.lock().unwrap() = None;
    harness.service.control(TerminalControl::Attach {
        pane: "w1:p1".into(),
        size: Some(SIZE),
        manual: true,
    });
    let opened = harness.opened();
    harness.state("w1:p1", "controlling");
    harness.key("w1:p1", b"ok");
    assert_eq!(opened.next_input(), b"ok");
}

#[test]
fn a_device_node_leaves_a_failed_attach_to_the_operator() {
    let harness = harness(RetryPolicy::Manual);
    *harness.refuse.refuse.lock().unwrap() = Some("no herdr".into());
    harness.attach("w1:p1", Some(SIZE));
    let state = harness.state("w1:p1", "unavailable");
    assert_eq!(state.retry_decision, "manual");
    assert!(
        harness.service.shared.lock().panes["w1:p1"]
            .recovery
            .is_none()
    );
}

#[test]
fn a_node_keeps_at_most_its_cap_of_panes_attached() {
    let harness = harness(RetryPolicy::Automatic);
    // The sessions stay open for as long as the test holds them.
    let mut sessions = Vec::new();
    for index in 0..MAX_ATTACHED_PANES {
        harness.attach(&format!("p{index}"), Some(SIZE));
        sessions.push(harness.opened());
    }
    harness.service.control(TerminalControl::Shown {
        panes: vec!["over".into()],
    });
    harness.attach("over", Some(SIZE));
    let state = harness.state("over", "unavailable");
    assert_eq!(state.exit_category.as_deref(), Some("attach_limit"));
    assert_eq!(state.retry_decision, "manual");
    // B18: the refused pane waits to be shown again; the retry clock never
    // tries it, so keys typed into it are refused rather than held for a
    // retry that would discard them.
    assert!(
        harness.service.shared.lock().panes["over"]
            .recovery
            .is_none()
    );
    let later = Instant::now() + Duration::from_secs(60);
    harness
        .service
        .shared
        .run(|inner, shared| inner.tick(shared, later));
    assert_eq!(
        harness.service.shared.lock().panes["over"]
            .lifecycle
            .attempt,
        state.attempt
    );
    // Another pane leaving makes room; the refused one attaches when it is
    // asked for again.
    harness
        .service
        .control(TerminalControl::Forget { pane: "p0".into() });
    // The cap includes the retiring connection until its actual cleanup.
    let deadline = Instant::now() + WAIT;
    while harness.service.shared.children.contains("p0") {
        assert!(
            Instant::now() < deadline,
            "the retired connection never ended"
        );
        thread::yield_now();
    }
    harness.attach("over", Some(SIZE));
    assert_eq!(harness.opened().pane, "over");
}

/// A screen naming pane after pane leaves this node at most its cap of
/// panes it never attached: past it those the core does not show are
/// forgotten, and with every one on screen the next is refused and the
/// refusal reported. A key into a pane the node was never told of records
/// no fact; a pane the core then attaches still attaches.
#[test]
fn panes_a_screen_names_and_the_node_never_heard_of_stay_bounded() {
    let harness = harness(RetryPolicy::Automatic);
    for index in 0..10 * MAX_UNATTACHED_PANES {
        harness.service.view(&format!("junk{index}"), SIZE, true);
        assert!(harness.service.shared.lock().panes.len() <= MAX_UNATTACHED_PANES);
    }
    let tracked = harness
        .service
        .shared
        .lock()
        .panes
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(tracked.len(), MAX_UNATTACHED_PANES);
    harness
        .service
        .control(TerminalControl::Shown { panes: tracked });
    harness.service.view("one-more", SIZE, true);
    assert!(!harness.service.shared.lock().panes.contains_key("one-more"));
    harness.report_where(|report| {
        matches!(report, TerminalReport::Error { kind, .. } if kind == "terminal.pane_limit")
    });
    for index in 0..1_000 {
        harness.key(&format!("ghost{index}"), b"x");
    }
    assert!(harness.service.shared.lock().facts.is_empty());
    let _opened = harness.controlling("w1:p1");
}

/// A screen's resize that was on its way when the core forgot its pane
/// leaves a pane nothing owns; however many pile up, a pane on screen later
/// is still drawn.
#[test]
fn views_that_land_after_their_pane_was_forgotten_do_not_keep_a_new_pane_from_being_drawn() {
    let harness = harness(RetryPolicy::Automatic);
    for index in 0..MAX_UNATTACHED_PANES {
        let pane = format!("closed{index}");
        harness
            .service
            .control(TerminalControl::Forget { pane: pane.clone() });
        harness.service.view(&pane, SIZE, false);
    }
    harness.service.view("w1:p9", SIZE, true);
    harness.attach("w1:p9", Some(SIZE));
    harness.opened();
    let mut refused = false;
    harness.report_where(|report| {
        refused |=
            matches!(report, TerminalReport::Error { kind, .. } if kind == "terminal.pane_limit");
        matches!(report, TerminalReport::State { pane, state } if pane == "w1:p9" && state.state == "controlling")
    });
    assert!(!refused, "the view of a pane on screen was refused");
}

/// What the core said of a pane it has not attached is kept at the cap: a
/// sleeping agent's pane still drops the keys typed into it once it attaches.
#[test]
fn a_pane_the_core_put_to_sleep_is_not_forgotten_at_the_cap() {
    let harness = harness(RetryPolicy::Automatic);
    harness.service.control(TerminalControl::Asleep {
        pane: "w1:p9".into(),
        asleep: true,
    });
    for index in 1..MAX_UNATTACHED_PANES {
        let pane = format!("closed{index}");
        harness
            .service
            .control(TerminalControl::Forget { pane: pane.clone() });
        harness.service.view(&pane, SIZE, false);
    }
    assert_eq!(
        harness.service.shared.lock().panes.len(),
        MAX_UNATTACHED_PANES
    );
    harness.service.view("w1:p10", SIZE, true);
    let opened = harness.controlling("w1:p9");
    harness.key("w1:p9", b"lost");
    harness.service.control(TerminalControl::Asleep {
        pane: "w1:p9".into(),
        asleep: false,
    });
    harness.key("w1:p9", b"kept");
    assert_eq!(opened.next_input(), b"kept");
}

/// Controls about panes no view or attach named leave nothing counted
/// against the cap: a pane the core woke without ever putting to sleep, or
/// whose frame it watched and never attached (a hidden tab, a refused or
/// failed attach), does not keep a new pane's view from being drawn.
#[test]
fn panes_only_watched_or_woken_do_not_keep_a_new_pane_from_being_drawn() {
    let harness = harness(RetryPolicy::Automatic);
    for index in 0..=MAX_UNATTACHED_PANES {
        let pane = format!("ghost{index}");
        harness.service.control(TerminalControl::Asleep {
            pane: pane.clone(),
            asleep: false,
        });
        harness
            .service
            .control(TerminalControl::WatchFrame { pane: pane.clone() });
        harness.service.control(TerminalControl::Release {
            pane,
            message: "hidden".into(),
        });
    }
    for index in 0..=MAX_UNATTACHED_PANES {
        harness.service.control(TerminalControl::WatchFrame {
            pane: format!("watched{index}"),
        });
    }
    harness.service.view("w1:p9", SIZE, true);
    harness.attach("w1:p9", Some(SIZE));
    harness.opened();
    let mut refused = false;
    harness.report_where(|report| {
        refused |=
            matches!(report, TerminalReport::Error { kind, .. } if kind == "terminal.pane_limit");
        matches!(report, TerminalReport::State { pane, state } if pane == "w1:p9" && state.state == "controlling")
    });
    assert!(!refused, "the view of a new pane was refused");
}

/// The core watches a pane Herdr just created before it attaches it; the
/// pane's first frame still reports when it reached the screen.
#[test]
fn a_frame_watched_before_its_pane_attaches_is_reported_when_shown() {
    let harness = harness(RetryPolicy::Automatic);
    harness.service.control(TerminalControl::WatchFrame {
        pane: "w1:p5".into(),
    });
    let opened = harness.controlling("w1:p5");
    opened.frame(SIZE, true, b"prompt");
    harness.report_where(
        |report| matches!(report, TerminalReport::FrameShown { pane, .. } if pane == "w1:p5"),
    );
}

/// A new view at the grid its session already runs at asks Herdr for the
/// whole screen at the node, since the core sends no resize for a size that
/// did not change; the next frame is that whole screen.
#[test]
fn a_new_view_at_the_sessions_grid_asks_for_a_whole_frame() {
    let harness = harness(RetryPolicy::Automatic);
    let opened = harness.controlling("w1:p1");
    // What the attach itself wrote is not the answer.
    while opened
        .input
        .recv_timeout(Duration::from_millis(200))
        .is_ok()
    {}
    harness.service.view("w1:p1", SIZE, true);
    let line = loop {
        let line = opened
            .input
            .recv_timeout(WAIT)
            .expect("the node asked Herdr for the whole screen");
        if line["type"] == "terminal.resize" {
            break line;
        }
    };
    assert_eq!(line["rows"], SIZE.rows);
    assert_eq!(line["cols"], SIZE.cols);
}

#[test]
fn an_idle_service_has_nothing_due() {
    let harness = harness(RetryPolicy::Automatic);
    let _opened = harness.controlling("w1:p1");
    assert_eq!(harness.service.shared.lock().next_deadline(), None);
}

/// B5 on the failure path: a thousand keys into a pane this node has never
/// heard of are each refused, and the core hears it once a window.
#[test]
fn a_key_flood_into_an_unknown_pane_is_reported_once_a_window() {
    let harness = harness(RetryPolicy::Automatic);
    let started = Instant::now();
    for _ in 0..1_000 {
        harness.key("w9:p9", b"k");
    }
    let refused = harness
        .reports
        .try_iter()
        .filter(|report| matches!(report, TerminalReport::Error { .. }))
        .count() as u64;
    let windows = started.elapsed().as_secs() + 1;
    assert!(
        (1..=windows).contains(&refused),
        "{refused} refusals in {windows} windows"
    );
}

/// A frame reaches the screens' sink with the service's lock released: a
/// control sent while the sink still encodes a frame goes through at once.
#[test]
fn a_frame_on_its_way_to_the_screens_holds_up_no_control() {
    /// Holds the first output it is given until the test lets it go.
    struct HeldOutput {
        first: Mutex<bool>,
        entered: Mutex<Sender<()>>,
        go: Mutex<Receiver<()>>,
    }
    impl OutputSink for HeldOutput {
        fn output(&self, _: &str, _: &[u8], _: bool) {
            if std::mem::replace(&mut *self.first.lock().unwrap(), false) {
                self.entered.lock().unwrap().send(()).unwrap();
                let _ = self.go.lock().unwrap().recv_timeout(WAIT);
            }
        }
        fn forget(&self, _: &str) {}
    }
    let (entered_tx, entered) = channel();
    let (go_tx, go) = channel();
    let (opened_tx, opened) = channel();
    let (reports_tx, _reports) = channel();
    let service = Arc::new(
        Service::start(
            Box::new(FakeAttacher {
                opened: Mutex::new(opened_tx),
                refuse: Mutex::new(None),
            }),
            Arc::new(HeldOutput {
                first: Mutex::new(true),
                entered: Mutex::new(entered_tx),
                go: Mutex::new(go),
            }),
            Arc::new(Reports(Mutex::new(reports_tx))),
            RetryPolicy::Automatic,
        )
        .unwrap(),
    );
    service.control(TerminalControl::Attach {
        pane: "w1:p1".into(),
        size: Some(SIZE),
        manual: false,
    });
    let first: Opened = opened.recv_timeout(WAIT).unwrap();
    first.frame(SIZE, true, b"whole screen");
    entered
        .recv_timeout(WAIT)
        .expect("the frame reached the sink");
    let (done_tx, done) = channel();
    let control = {
        let service = Arc::clone(&service);
        thread::spawn(move || {
            service.control(TerminalControl::Attach {
                pane: "w1:p2".into(),
                size: Some(SIZE),
                manual: false,
            });
            done_tx.send(()).unwrap();
        })
    };
    let went_through = done.recv_timeout(Duration::from_secs(2)).is_ok();
    go_tx.send(()).unwrap();
    control.join().unwrap();
    assert!(went_through, "a control waited for the screens' sink");
}

/// Two threads' reports reach the core in the order their work ran in the
/// service, so a pane's older state never lands after a newer one.
#[test]
fn reports_reach_the_core_in_the_order_the_service_made_them() {
    /// Holds the first report it is given until the test lets it go.
    struct HeldSink {
        first: Mutex<bool>,
        entered: Mutex<Sender<()>>,
        go: Mutex<Receiver<()>>,
        heard: Mutex<Vec<String>>,
    }
    impl ReportSink for HeldSink {
        fn report(&self, report: TerminalReport) {
            let first = std::mem::replace(&mut *self.first.lock().unwrap(), false);
            if first {
                self.entered.lock().unwrap().send(()).unwrap();
                let _ = self.go.lock().unwrap().recv_timeout(WAIT);
            }
            if let TerminalReport::State { pane, .. } = report {
                self.heard.lock().unwrap().push(pane);
            }
        }
    }
    let (entered_tx, entered) = channel();
    let (go_tx, go) = channel();
    let sink = Arc::new(HeldSink {
        first: Mutex::new(true),
        entered: Mutex::new(entered_tx),
        go: Mutex::new(go),
        heard: Mutex::default(),
    });
    let (opened_tx, _opened) = channel();
    let service = Arc::new(
        Service::start(
            Box::new(FakeAttacher {
                opened: Mutex::new(opened_tx),
                refuse: Mutex::new(None),
            }),
            Arc::new(Outputs::default()),
            Arc::clone(&sink) as Arc<dyn ReportSink>,
            RetryPolicy::Automatic,
        )
        .unwrap(),
    );
    let attach = |pane: &str| TerminalControl::Attach {
        pane: pane.into(),
        size: None,
        manual: false,
    };
    let first = {
        let service = Arc::clone(&service);
        let control = attach("w1:p1");
        thread::spawn(move || service.control(control))
    };
    entered.recv_timeout(WAIT).unwrap();
    let (done_tx, done) = channel();
    let second = {
        let service = Arc::clone(&service);
        let control = attach("w1:p2");
        thread::spawn(move || {
            service.control(control);
            done_tx.send(()).unwrap();
        })
    };
    // The second control has every chance to report before the first's
    // report is let go.
    let _ = done.recv_timeout(Duration::from_millis(200));
    go_tx.send(()).unwrap();
    first.join().unwrap();
    second.join().unwrap();
    assert_eq!(sink.heard.lock().unwrap().as_slice(), ["w1:p1", "w1:p2"]);
}

/// A closed service lets every pane it held go, while something else still
/// holds the service, and attaches none after: a core that stops for a move
/// leaves this machine's panes to the node its process becomes, which would
/// otherwise find each one held and draw it read-only.
#[test]
fn a_closed_service_lets_its_panes_go_and_attaches_none_again() {
    let harness = harness(RetryPolicy::Automatic);
    let opened = harness.controlling("w1:p1");
    harness.service.close();
    while opened.next_line()["type"] != "terminal.release" {}
    harness.attach("w1:p2", Some(SIZE));
    assert_eq!(harness.service.attached(), 0);
}

/// A close returns only once each attach child it ended has exited: the
/// role that follows in the same process attaches only after it, and a
/// child still on its way out would meet that attach as another client
/// holding the pane.
#[test]
fn a_close_returns_only_once_its_children_have_exited() {
    let (exiting_tx, exiting) = channel();
    let (exit_tx, exit) = channel::<()>();
    let exiting_tx = Mutex::new(exiting_tx);
    let exit = Arc::new(Mutex::new(exit));
    let harness = harness_with(RetryPolicy::Automatic, move || {
        let exiting = exiting_tx.lock().unwrap().clone();
        let exit = Arc::clone(&exit);
        Cleanup::Other(Box::new(move || {
            exiting.send(()).unwrap();
            let _ = exit.lock().unwrap().recv();
        }))
    });
    harness.controlling("w1:p1");
    let (closed_tx, closed) = channel();
    thread::scope(|scope| {
        scope.spawn(|| {
            harness.service.close();
            closed_tx.send(()).unwrap();
        });
        exiting.recv_timeout(WAIT).expect("the child began to exit");
        // A close that does not wait returns within microseconds of letting
        // the pane go; this one waits for the child as long as it takes.
        assert!(
            closed.recv_timeout(WAIT / 10).is_err(),
            "the close returned while its child was still exiting"
        );
        exit_tx.send(()).unwrap();
        closed
            .recv_timeout(WAIT)
            .expect("the close returned once its child exited");
    });
}

#[test]
fn a_close_that_runs_before_a_spawn_s_worker_waits_and_nothing_opens_after_it() {
    // Tells of any open that comes after its service's close returned.
    struct Late {
        returned: Arc<std::sync::atomic::AtomicBool>,
        late: Mutex<Sender<()>>,
    }
    impl Attacher for Late {
        fn open_transport(&self, _: &str, _: Option<GridSize>) -> Result<Transport, OpenFailure> {
            if self.returned.load(std::sync::atomic::Ordering::SeqCst) {
                let _ = self.late.lock().unwrap().send(());
            }
            Err(OpenFailure::retried("refused".into()))
        }
    }
    let (late_tx, late) = channel();
    // Every service lives to the end, so a worker that runs late still can.
    let mut services = Vec::new();
    for _ in 0..50 {
        let returned = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (reports_tx, reports) = channel();
        let service = Service::start(
            Box::new(Late {
                returned: Arc::clone(&returned),
                late: Mutex::new(late_tx.clone()),
            }),
            Arc::new(Outputs::default()),
            Arc::new(Reports(Mutex::new(reports_tx))),
            RetryPolicy::Manual,
        )
        .unwrap();
        // The attach queues a spawn whose worker may not have run yet.
        service.control(TerminalControl::Attach {
            pane: "w1:p1".into(),
            size: Some(SIZE),
            manual: false,
        });
        service.close();
        returned.store(true, std::sync::atomic::Ordering::SeqCst);
        services.push((service, reports));
    }
    assert!(
        late.recv_timeout(WAIT / 10).is_err(),
        "a session opened after its service's close returned"
    );
}

fn side_input(
    service: &Service,
    pane: &str,
    bytes: usize,
) -> Receiver<Result<(), hide_node_link::terminal::InputPermitError>> {
    let (reply, result) = channel();
    service.input_permit(
        pane,
        bytes,
        Box::new(move |outcome| {
            let _ = reply.send(outcome);
        }),
    );
    result
}

#[test]
fn a_phone_permission_follows_earlier_desktop_keys_through_the_attach_queue() {
    let harness = harness(RetryPolicy::Automatic);
    harness.service.control(TerminalControl::RequestOpen {
        request: "fork".into(),
        fork: true,
    });
    harness.service.control(TerminalControl::RequestHold {
        request: "fork".into(),
        pane: "w1:p2".into(),
    });
    harness.key("w1:p2", b"ab");
    let permission = side_input(&harness.service, "w1:p2", 3);
    harness.service.control(TerminalControl::RequestResolve {
        request: "fork".into(),
        pane: "w1:p2".into(),
    });
    assert!(matches!(
        permission.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    harness.attach("w1:p2", Some(SIZE));
    let opened = harness.opened();
    assert_eq!(permission.recv_timeout(WAIT).unwrap(), Ok(()));
    // Once the phone is permitted, the older desktop bytes have already
    // reached the controller pipe. Reservation bytes are never written.
    let mut written = Vec::new();
    while let Ok(line) = opened.input.try_recv() {
        if line["type"] == "terminal.input" {
            written.extend(protocol::decode_base64(line["bytes"].as_str().unwrap()).unwrap());
        }
    }
    assert_eq!(written, b"ab");
}

#[test]
fn a_non_fork_permission_expires_and_releases_its_reserved_bytes() {
    let harness = harness(RetryPolicy::Automatic);
    harness.service.control(TerminalControl::RequestOpen {
        request: "new".into(),
        fork: false,
    });
    harness.service.control(TerminalControl::RequestHold {
        request: "new".into(),
        pane: "w1:p2".into(),
    });
    let permission = side_input(
        &harness.service,
        "w1:p2",
        hide_node_link::terminal::INPUT_HOLD_LIMIT_BYTES,
    );
    let deadline = harness
        .service
        .shared
        .lock()
        .next_deadline()
        .expect("permission owns a deadline");
    harness.service.shared.run(|inner, shared| {
        inner.tick(shared, deadline);
    });
    assert_eq!(
        permission.recv_timeout(WAIT).unwrap(),
        Err(hide_node_link::terminal::InputPermitError::Discarded)
    );
    let next = side_input(
        &harness.service,
        "w1:p2",
        hide_node_link::terminal::INPUT_HOLD_LIMIT_BYTES,
    );
    harness.service.control(TerminalControl::RequestResolve {
        request: "new".into(),
        pane: "w1:p2".into(),
    });
    assert_eq!(
        next.recv_timeout(WAIT).unwrap(),
        Ok(()),
        "expired reservation no longer consumes the cap"
    );
}

#[test]
fn side_channel_input_waits_for_the_fork_without_a_control_attach() {
    let harness = harness(RetryPolicy::Automatic);
    harness.service.control(TerminalControl::RequestOpen {
        request: "fork".into(),
        fork: true,
    });
    harness.service.control(TerminalControl::RequestHold {
        request: "fork".into(),
        pane: "w1:p2".into(),
    });
    let held = side_input(&harness.service, "w1:p2", 20);
    assert!(matches!(
        held.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    assert_eq!(
        side_input(&harness.service, "w1:p3", 20)
            .recv_timeout(WAIT)
            .unwrap(),
        Ok(())
    );
    harness.service.control(TerminalControl::RequestResolve {
        request: "fork".into(),
        pane: "w1:p2".into(),
    });
    assert_eq!(held.recv_timeout(WAIT).unwrap(), Ok(()));
    assert!(matches!(
        harness.opened.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    assert_eq!(harness.service.attached(), 0);
}

#[test]
fn side_channel_input_shares_the_fork_caps_and_does_not_acknowledge_discarded_input() {
    use hide_node_link::terminal::InputPermitError;
    for timeout in [false, true] {
        let harness = harness(RetryPolicy::Automatic);
        harness.service.control(TerminalControl::RequestOpen {
            request: "fork".into(),
            fork: true,
        });
        harness.service.control(TerminalControl::RequestHold {
            request: "fork".into(),
            pane: "w1:p2".into(),
        });
        let held = side_input(
            &harness.service,
            "w1:p2",
            hide_node_link::terminal::INPUT_HOLD_LIMIT_BYTES,
        );
        if timeout {
            harness
                .service
                .shared
                .run(|inner, shared| inner.tick(shared, Instant::now() + input::FORK_HOLD_MAX_AGE));
        } else {
            // A desktop key crosses the shared cap, including the phone's reservation.
            harness.key("w1:p2", b"x");
        }
        assert_eq!(
            held.recv_timeout(WAIT).unwrap(),
            Err(InputPermitError::Discarded)
        );
        assert_eq!(
            side_input(&harness.service, "w1:p2", 1)
                .recv_timeout(WAIT)
                .unwrap(),
            Err(InputPermitError::Discarded)
        );
        harness.service.control(TerminalControl::RequestResolve {
            request: "fork".into(),
            pane: "w1:p2".into(),
        });
        assert_eq!(
            side_input(&harness.service, "w1:p2", 1)
                .recv_timeout(WAIT)
                .unwrap(),
            Ok(())
        );
    }
}

#[test]
fn side_channel_waiters_are_bounded_and_node_close_refuses_them() {
    use hide_node_link::terminal::InputPermitError;
    let harness = harness(RetryPolicy::Automatic);
    harness.service.control(TerminalControl::RequestOpen {
        request: "fork".into(),
        fork: true,
    });
    harness.service.control(TerminalControl::RequestHold {
        request: "fork".into(),
        pane: "w1:p2".into(),
    });
    let waiting = (0..64)
        .map(|_| side_input(&harness.service, "w1:p2", 1))
        .collect::<Vec<_>>();
    assert_eq!(
        side_input(&harness.service, "w1:p2", 1)
            .recv_timeout(WAIT)
            .unwrap(),
        Err(InputPermitError::Limit)
    );
    harness.service.close();
    for result in waiting {
        assert_eq!(
            result.recv_timeout(WAIT).unwrap(),
            Err(InputPermitError::Discarded)
        );
    }
}

/// Reads the session's input until `count` bytes came, in order.
fn input_bytes(opened: &Opened, count: usize) -> Vec<u8> {
    let mut written = Vec::new();
    while written.len() < count {
        written.extend(opened.next_input());
    }
    written
}

/// PRD engine-bundle-3-core-owns D-13, B31: keys the operator types while
/// the core's own engine types a letter wait until its submit was handed
/// over, then follow it in the order typed; none lands inside the letter.
#[test]
fn keys_typed_during_a_letter_follow_its_submit_in_order() {
    let harness = harness(RetryPolicy::Automatic);
    let opened = harness.controlling("w1:p1");
    thread::scope(|scope| {
        let letter = scope.spawn(|| {
            harness
                .service
                .type_letter(
                    "w1:p1",
                    vec![Typed::Bytes(b"letter".to_vec())],
                    Typed::Bytes(b"\r".to_vec()),
                    Duration::from_millis(300),
                )
                .unwrap();
        });
        assert_eq!(input_bytes(&opened, b"letter".len()), b"letter");
        harness.key("w1:p1", b"x");
        harness.key("w1:p1", b"y\r");
        letter.join().unwrap();
    });
    assert_eq!(input_bytes(&opened, b"\rxy\r".len()), b"\rxy\r");
    // The held keys are the operator's, reported as typed once written.
    let typed = harness.report_where(|report| matches!(report, TerminalReport::Input { .. }));
    assert!(matches!(typed, TerminalReport::Input { .. }), "{typed:?}");
}

/// A letter begun while another is typed into the same pane shares its
/// hold: a key typed during the first is kept and written once the last
/// letter has been submitted.
#[test]
fn keys_held_for_a_letter_are_kept_when_another_letter_begins_and_follow_the_last_submit() {
    let harness = harness(RetryPolicy::Automatic);
    let opened = harness.controlling("w1:p1");
    let letter = |body: &'static [u8], gap: u64| {
        harness
            .service
            .type_letter(
                "w1:p1",
                vec![Typed::Bytes(body.to_vec())],
                Typed::Bytes(b"\r".to_vec()),
                Duration::from_millis(gap),
            )
            .unwrap();
    };
    thread::scope(|scope| {
        let first = scope.spawn(|| letter(b"a", 300));
        assert_eq!(input_bytes(&opened, 1), b"a");
        harness.key("w1:p1", b"x");
        let second = scope.spawn(|| letter(b"b", 600));
        // Each letter is one submission: the second waits behind the
        // first's submit, as Herdr's `agent.prompt` holds what follows it.
        assert_eq!(input_bytes(&opened, b"\rb\r".len()), b"\rb\r");
        first.join().unwrap();
        second.join().unwrap();
    });
    assert_eq!(input_bytes(&opened, 1), b"x");
}

/// A fork's new pane holds the operator's keys until the fork's agent runs,
/// but the core's own engine types that agent's start line there at once, and
/// the held keys follow it once the request resolves: holding the start line
/// too left the fork's agent never started.
#[test]
fn the_engines_start_line_is_not_held_behind_a_forks_request() {
    let harness = harness(RetryPolicy::Automatic);
    let opened = harness.controlling("w1:p1");
    harness.service.control(TerminalControl::RequestOpen {
        request: "fork".into(),
        fork: true,
    });
    harness.service.control(TerminalControl::RequestHold {
        request: "fork".into(),
        pane: "w1:p1".into(),
    });
    harness.key("w1:p1", b"typed early");
    harness
        .service
        .type_input("w1:p1", Typed::Bytes(b"claude --resume s\r".to_vec()))
        .unwrap();
    assert_eq!(input_bytes(&opened, 18), b"claude --resume s\r");
    harness.service.control(TerminalControl::RequestResolve {
        request: "fork".into(),
        pane: "w1:p1".into(),
    });
    assert_eq!(input_bytes(&opened, 11), b"typed early");
}

/// A sleeping agent's pane takes none of the operator's keys, but the core's
/// own engine types the agent's quit key and its resume line there while the
/// sleep record holds the pane asleep, and they are written.
#[test]
fn the_engines_typing_reaches_a_sleeping_pane_and_the_operators_keys_do_not() {
    let harness = harness(RetryPolicy::Automatic);
    let opened = harness.controlling("w1:p1");
    harness.service.control(TerminalControl::Asleep {
        pane: "w1:p1".into(),
        asleep: true,
    });
    harness.key("w1:p1", b"lost");
    harness
        .service
        .type_input("w1:p1", Typed::Bytes(b"\x03".to_vec()))
        .unwrap();
    harness
        .service
        .type_input("w1:p1", Typed::Bytes(b"claude --resume s\r".to_vec()))
        .unwrap();
    let typed = b"\x03claude --resume s\r";
    assert_eq!(input_bytes(&opened, typed.len()), typed);
    harness.service.control(TerminalControl::Asleep {
        pane: "w1:p1".into(),
        asleep: false,
    });
    harness.key("w1:p1", b"kept");
    assert_eq!(opened.next_input(), b"kept");
}

/// The core's own engine typing a command is not the operator typing: no
/// key of it is reported, so the first report is the operator's own key.
#[test]
fn the_engines_typing_is_not_reported_as_the_operators() {
    let harness = harness(RetryPolicy::Automatic);
    let opened = harness.controlling("w1:p1");
    harness
        .service
        .type_input("w1:p1", Typed::Bytes(b"ls\r".to_vec()))
        .unwrap();
    assert_eq!(opened.next_input(), b"ls\r");
    harness.key("w1:p1", b"x");
    assert_eq!(opened.next_input(), b"x");
    let first = harness.report_where(|report| matches!(report, TerminalReport::Input { .. }));
    assert!(
        matches!(
            first,
            TerminalReport::Input {
                submitted: false,
                ..
            }
        ),
        "{first:?}"
    );
}

/// A pane whose tab left the last shown has no session; the engine's input
/// attaches it again and is written once the session opens.
#[test]
fn the_engines_typing_into_a_released_pane_attaches_it_and_is_written() {
    let harness = harness(RetryPolicy::Automatic);
    let _first = harness.controlling("w1:p1");
    harness.service.control(TerminalControl::Release {
        pane: "w1:p1".into(),
        message: "detached".into(),
    });
    harness.state("w1:p1", "released");
    harness
        .service
        .type_input("w1:p1", Typed::Bytes(b"resume\r".to_vec()))
        .unwrap();
    let again = harness.opened();
    assert_eq!(input_bytes(&again, b"resume\r".len()), b"resume\r");
}

/// Named keys go only to a transport that encodes them; another refuses
/// them with why, sends nothing, and keeps its session.
#[test]
fn named_keys_for_a_session_that_does_not_read_them_are_refused_with_why() {
    let harness = harness(RetryPolicy::Automatic);
    let opened = harness.controlling("w1:p1");
    harness
        .service
        .type_input("w1:p1", Typed::Keys(vec![hide_holder_protocol::Key::Enter]))
        .unwrap_err();
    let refused = harness.report_where(|report| matches!(report, TerminalReport::Error { .. }));
    assert!(
        matches!(&refused, TerminalReport::Error { kind, .. } if kind == "terminal.unsupported_holder"),
        "{refused:?}"
    );
    harness.key("w1:p1", b"x");
    assert_eq!(opened.next_input(), b"x");
}

/// The grid a holder in [`HolderPanes`] runs at until it is told another.
const HOLDER_SIZE: GridSize = GridSize { rows: 24, cols: 80 };
/// The grid the screen draws the pane at, and the core saved for it.
const VIEW: GridSize = GridSize {
    rows: 44,
    cols: 109,
};

/// Panes as a node with holders and no Herdr opens them: a pane whose holder
/// was started keeps its own size, attached at [`HOLDER_SIZE`] unless a size
/// is named, and every other pane is refused, as a daemon on the core's own
/// engine refuses a pane whose holder it has not started yet.
struct HolderPanes {
    fake: Arc<FakeAttacher>,
    holders: Mutex<HashSet<String>>,
    /// Holds the next attempt, once it says it began, until the test lets
    /// it answer.
    gate: Mutex<Option<(Sender<()>, Receiver<()>)>>,
}

impl HolderPanes {
    /// Holds the next attempt: the first end hears it begin, the second
    /// lets it answer.
    fn hold_next(&self) -> (Receiver<()>, Sender<()>) {
        let (began, beginning) = channel();
        let (answer, answered) = channel();
        *self.gate.lock().unwrap() = Some((began, answered));
        (beginning, answer)
    }
}

impl Attacher for HolderPanes {
    fn open_transport(&self, pane: &str, size: Option<GridSize>) -> Result<Transport, OpenFailure> {
        // Whether the holder was started when the attempt began.
        let held = self.keeps_own_size(pane);
        let gate = self.gate.lock().unwrap().take();
        if let Some((began, answered)) = gate {
            began.send(()).unwrap();
            answered
                .recv_timeout(WAIT)
                .expect("the test let the attempt answer");
        }
        if !held {
            return Err(OpenFailure::retried(format!(
                "Pane {pane} has no holder on this node"
            )));
        }
        let size = size.unwrap_or(HOLDER_SIZE);
        let mut transport = holder_like(self.fake.open_transport(pane, Some(size))?);
        transport.size = Some(size);
        Ok(transport)
    }

    fn keeps_own_size(&self, pane: &str) -> bool {
        self.holders.lock().unwrap().contains(pane)
    }
}

/// A harness over [`HolderPanes`] that never retries on its own, so an
/// attach that opens is one a control asked for.
fn holder_harness() -> (Harness, Arc<HolderPanes>) {
    let (opened_tx, opened) = channel();
    let (reports_tx, reports) = channel();
    let fake = Arc::new(FakeAttacher {
        opened: Mutex::new(opened_tx),
        refuse: Mutex::new(None),
    });
    let panes = Arc::new(HolderPanes {
        fake: Arc::clone(&fake),
        holders: Mutex::new(HashSet::new()),
        gate: Mutex::new(None),
    });
    struct Shared(Arc<HolderPanes>);
    impl Attacher for Shared {
        fn open_transport(
            &self,
            pane: &str,
            size: Option<GridSize>,
        ) -> Result<Transport, OpenFailure> {
            self.0.open_transport(pane, size)
        }
        fn keeps_own_size(&self, pane: &str) -> bool {
            self.0.keeps_own_size(pane)
        }
    }
    let outputs = Arc::new(Outputs::default());
    let service = Service::start(
        Box::new(Shared(Arc::clone(&panes))),
        Arc::clone(&outputs) as Arc<dyn OutputSink>,
        Arc::new(Reports(Mutex::new(reports_tx))),
        RetryPolicy::Manual,
    )
    .unwrap();
    let harness = Harness {
        service,
        opened,
        reports,
        outputs,
        refuse: fake,
    };
    (harness, panes)
}

#[test]
fn a_holder_pane_its_node_attached_before_a_view_drew_it_takes_the_view_s_grid() {
    // A daemon that starts again after every process ended: the restore
    // starts the pane's shell, its node attaches it at the holder's size,
    // and only then does the page draw it, at the grid the core saved and
    // so never sends as a resize.
    let (harness, panes) = holder_harness();
    panes.holders.lock().unwrap().insert("w1:p1".into());
    harness.attach("w1:p1", None);
    let opened = harness.opened();
    assert_eq!(opened.size, HOLDER_SIZE);
    harness.state("w1:p1", "controlling");
    harness.service.view("w1:p1", VIEW, true);
    let resize = opened.next_line();
    assert_eq!(
        (&resize["type"], &resize["rows"], &resize["cols"]),
        (&json!("terminal.resize"), &json!(44), &json!(109))
    );
    opened.frame(VIEW, true, b"fixture % ");
    let written = harness.wait_written(1);
    assert_eq!(written[0].0, "w1:p1");
    assert!(written[0].1.ends_with(b"fixture % "), "{written:?}");
}

#[test]
fn a_holder_started_after_its_attach_was_refused_attaches_at_once_at_the_core_s_size() {
    // The core asks for the pane from the saved layout before the restore
    // has started its shell: that attempt is refused. The holder's start
    // then attaches the pane now, at the size the core named, not at the
    // holder's.
    let (harness, panes) = holder_harness();
    harness.attach("w1:p1", Some(VIEW));
    harness.state("w1:p1", "unavailable");
    panes.holders.lock().unwrap().insert("w1:p1".into());
    harness.attach("w1:p1", None);
    let opened = harness.opened();
    assert_eq!(opened.size, VIEW);
    harness.state("w1:p1", "controlling");
}

#[test]
fn a_pane_with_no_holder_or_whose_holder_refused_is_not_attached_again_by_a_later_attach() {
    let (harness, panes) = holder_harness();
    // Closed while no daemon ran: its holder is never started.
    harness.attach("w1:p1", Some(VIEW));
    harness.state("w1:p1", "unavailable");
    // Its holder is known but the attach to it fails: the pane is over.
    panes.holders.lock().unwrap().insert("w1:p2".into());
    *harness.refuse.refuse.lock().unwrap() = Some("the holder ended".into());
    harness.attach("w1:p2", None);
    harness.state("w1:p2", "unavailable");
    *harness.refuse.refuse.lock().unwrap() = None;
    harness.attach("w1:p1", None);
    harness.attach("w1:p1", Some(VIEW));
    harness.attach("w1:p2", None);
    harness.attach("w1:p2", Some(VIEW));
    // A pane whose holder was started, attached after them: the service
    // takes controls and hands their reports over in order, so any session
    // the attaches above started is reported before this one starts.
    panes.holders.lock().unwrap().insert("w1:p3".into());
    harness.attach("w1:p3", None);
    loop {
        match harness.report_where(|report| matches!(report, TerminalReport::State { .. })) {
            TerminalReport::State { pane, state } if pane == "w1:p3" => {
                assert_eq!(state.state, "starting");
                break;
            }
            TerminalReport::State { pane, state } => assert!(
                !matches!(state.state.as_str(), "starting" | "controlling"),
                "{pane} with no holder to reach was attached again: {state:?}"
            ),
            _ => unreachable!(),
        }
    }
    assert_eq!(harness.opened().pane, "w1:p3");
}

#[test]
fn a_view_that_arrives_while_a_holder_pane_opens_is_the_grid_it_takes() {
    // The page draws the pane after its node asked for the holder and
    // before the holder answered: the view's grid is the one the pane
    // takes, as an attach after the view would have.
    let (harness, panes) = holder_harness();
    panes.holders.lock().unwrap().insert("w1:p1".into());
    let (began, answer) = panes.hold_next();
    harness.attach("w1:p1", None);
    began.recv_timeout(WAIT).expect("the holder was asked for");
    harness.service.view("w1:p1", VIEW, true);
    answer.send(()).unwrap();
    let opened = harness.opened();
    assert_eq!(opened.size, HOLDER_SIZE);
    let resize = opened.next_line();
    assert_eq!(
        (&resize["type"], &resize["rows"], &resize["cols"]),
        (&json!("terminal.resize"), &json!(44), &json!(109))
    );
}

#[test]
fn a_holder_started_during_an_old_open_attaches_once_that_open_ends_without_retry_delay() {
    // The core's attempt, made before the holder was started, has not been
    // refused yet when the holder's start asks for the pane: the pane
    // queues the successor now. Its actual open waits for the old open's
    // completion, but never waits for the failed attempt's retry clock.
    let (harness, panes) = holder_harness();
    let (began, answer) = panes.hold_next();
    harness.attach("w1:p1", Some(VIEW));
    began.recv_timeout(WAIT).expect("the attempt began");
    harness.state("w1:p1", "starting");
    panes.holders.lock().unwrap().insert("w1:p1".into());
    harness.attach("w1:p1", None);
    assert!(harness.opened.try_recv().is_err(), "opens overlapped");
    answer.send(()).unwrap();
    let opened = harness.opened();
    assert_eq!(opened.size, VIEW);
    harness.state("w1:p1", "controlling");
}
