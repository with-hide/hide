use std::io::{Read, Write};
use std::sync::mpsc::{Receiver, Sender, channel};

use super::*;

pub(super) const WAIT: Duration = Duration::from_secs(5);

/// A session's stdout as the test writes it.
pub(super) struct LineReader {
    lines: Receiver<String>,
    pending: Vec<u8>,
}

impl Read for LineReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.pending.is_empty() {
            match self.lines.recv() {
                Ok(line) => {
                    self.pending = line.into_bytes();
                    self.pending.push(b'\n');
                }
                Err(_) => return Ok(0),
            }
        }
        let count = buffer.len().min(self.pending.len());
        buffer[..count].copy_from_slice(&self.pending[..count]);
        self.pending.drain(..count);
        Ok(count)
    }
}

/// A session's stdin as the test reads it, one line per write.
pub(super) struct LineWriter {
    lines: Sender<serde_json::Value>,
    pending: Vec<u8>,
}

impl Write for LineWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line = self.pending.drain(..=end).collect::<Vec<_>>();
            let value = serde_json::from_slice(&line[..line.len() - 1]).unwrap();
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
    pub(super) mode: Mode,
    pub(super) size: GridSize,
    pub(super) output: Sender<String>,
    pub(super) input: Receiver<serde_json::Value>,
}

impl Opened {
    pub(super) fn frame(&self, size: GridSize, full: bool, bytes: &[u8]) {
        self.output
            .send(
                json!({
                    "type": "terminal.frame", "seq": 1, "encoding": "ansi",
                    "width": size.cols, "height": size.rows, "full": full,
                    "bytes": protocol::encode_base64(bytes),
                })
                .to_string(),
            )
            .unwrap();
    }

    fn close(&self, reason: Option<&str>) {
        self.output
            .send(json!({"type": "terminal.closed", "reason": reason}).to_string())
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
}

pub(super) struct FakeAttacher {
    pub(super) opened: Mutex<Sender<Opened>>,
    pub(super) refuse: Mutex<Option<String>>,
}

impl Attacher for FakeAttacher {
    fn open(&self, pane: &str, mode: Mode, rows: u16, cols: u16) -> Result<SessionParts, String> {
        if let Some(reason) = self.refuse.lock().unwrap().clone() {
            return Err(reason);
        }
        let (output, lines) = channel();
        let (written, input) = channel();
        self.opened
            .lock()
            .unwrap()
            .send(Opened {
                pane: pane.to_owned(),
                mode,
                size: GridSize { rows, cols },
                output,
                input,
            })
            .unwrap();
        Ok(SessionParts {
            reader: Box::new(LineReader {
                lines,
                pending: Vec::new(),
            }),
            writer: Some(Box::new(LineWriter {
                lines: written,
                pending: Vec::new(),
            })),
            cleanup: Cleanup::None,
        })
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
    let (opened_tx, opened) = channel();
    let (reports_tx, reports) = channel();
    let attacher = Arc::new(FakeAttacher {
        opened: Mutex::new(opened_tx),
        refuse: Mutex::new(None),
    });
    let outputs = Arc::new(Outputs::default());
    struct Shared<C>(Arc<FakeAttacher>, C);
    impl<C: Fn() -> Cleanup + Send + Sync> Attacher for Shared<C> {
        fn open(
            &self,
            pane: &str,
            mode: Mode,
            rows: u16,
            cols: u16,
        ) -> Result<SessionParts, String> {
            let mut parts = self.0.open(pane, mode, rows, cols)?;
            parts.cleanup = (self.1)();
            Ok(parts)
        }
    }
    let service = Service::start(
        Box::new(Shared(Arc::clone(&attacher), cleanup)),
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
    assert_eq!(opened.mode, Mode::Control);
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

/// B2. Herdr draws an observer at the grid it attached with and an observer
/// cannot resize, so a view that changed size after the attach held every
/// frame and the pane froze. The observer attaches again at the view's grid.
#[test]
fn an_observed_frame_at_an_old_grid_reattaches_the_observer_at_the_views_grid() {
    let harness = harness(RetryPolicy::Automatic);
    let opened = harness.controlling("w1:p1");
    opened.close(Some("terminal attach taken over"));
    let observer = harness.opened();
    harness.state("w1:p1", "observing");
    let view = GridSize {
        rows: 33,
        cols: 143,
    };
    harness.service.view("w1:p1", view, false);
    observer.frame(SIZE, false, b"old");
    let again = harness.opened();
    assert_eq!((again.mode, again.size), (Mode::Observe, view));
    again.frame(view, true, b"new");
    assert_eq!(harness.wait_written(1)[0].1, b"\x1bcnew".to_vec());
}

/// B2, B8. An idle observed pane sends no frame after its size changes, so
/// once the size has held for a moment the observer attaches again at it;
/// a drag's intermediate sizes and the old-grid frames meanwhile attach
/// nothing.
#[test]
fn an_observed_view_that_changes_size_reattaches_once_the_size_settles() {
    let harness = harness(RetryPolicy::Automatic);
    let opened = harness.controlling("w1:p1");
    opened.close(Some("terminal attach taken over"));
    let observer = harness.opened();
    harness.state("w1:p1", "observing");
    for cols in [160, 180, 200] {
        harness.service.control(TerminalControl::Resize {
            pane: "w1:p1".into(),
            size: GridSize { rows: 33, cols },
        });
    }
    observer.frame(SIZE, false, b"old");
    let again = harness.opened();
    assert_eq!(
        (again.mode, again.size),
        (
            Mode::Observe,
            GridSize {
                rows: 33,
                cols: 200
            }
        )
    );
    assert!(
        harness
            .opened
            .recv_timeout(OBSERVER_RESIZE_QUIET * 3)
            .is_err(),
        "one new observer for the whole drag"
    );
    assert!(harness.written().is_empty(), "the old-grid frame was held");
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

#[test]
fn a_pane_another_client_controls_is_observed_once() {
    let harness = harness(RetryPolicy::Automatic);
    let opened = harness.controlling("w1:p1");
    opened.close(Some("terminal attach taken over"));
    let observer = harness.opened();
    assert_eq!(observer.mode, Mode::Observe);
    let state = harness.state("w1:p1", "observing");
    assert_eq!(state.mode.as_deref(), Some("observe"));
    harness.key("w1:p1", b"x");
    harness.report_where(|report| {
        matches!(report, TerminalReport::Error { kind, .. } if kind == "terminal.read_only")
    });
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
    first.close(Some("pane exited"));
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
    opened.close(Some("pane exited"));
    let state = harness.state("w1:p1", "ended");
    assert_eq!(state.exit_category.as_deref(), Some("terminal_closed"));
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
    second.close(None);
    harness.state("w1:p2", "closing");
    assert_eq!(
        harness.written().len(),
        1,
        "nothing is drawn over a closing pane"
    );
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
        fn open(&self, _: &str, _: Mode, _: u16, _: u16) -> Result<SessionParts, String> {
            if self.returned.load(std::sync::atomic::Ordering::SeqCst) {
                let _ = self.late.lock().unwrap().send(());
            }
            Err("refused".into())
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
