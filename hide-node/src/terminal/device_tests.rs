use std::sync::mpsc::{Receiver, Sender, channel};

use hide_host::serve::Terminals;
use hide_node_link::terminal::GridSize;

use super::super::tests::{FakeAttacher, Opened, WAIT};
use super::*;

const SIZE: GridSize = GridSize { rows: 24, cols: 80 };

fn up(line: &[u8]) -> TerminalUp {
    serde_json::from_slice::<TerminalLine<TerminalUp>>(line)
        .unwrap()
        .terminal
}

fn down(control: TerminalControl) -> Vec<u8> {
    line_of(TerminalDown::Control { control }).unwrap()
}

fn key_line(pane: &str, bytes: &[u8]) -> Vec<u8> {
    line_of(TerminalDown::Key {
        target: KeyTarget::Pane(pane.to_owned()),
        data: encode_base64(bytes),
        typed_at_unix_ms: 1,
    })
    .unwrap()
}

#[test]
fn reports_go_up_first_and_panes_take_turns() {
    let uplink = Uplink::default();
    for index in 0..50 {
        uplink.push_output("w1:p1", format!("flood {index}").as_bytes(), false);
    }
    uplink.push_output("w1:p2", b"quiet", false);
    uplink.push_report(TerminalReport::FirstFrame {
        pane: "w1:p2".into(),
        generation: 1,
    });
    let (first, _) = uplink.next().unwrap();
    assert!(matches!(up(&first.unwrap()), TerminalUp::Report { .. }));
    let mut panes = Vec::new();
    for _ in 0..3 {
        let (line, _) = uplink.next().unwrap();
        let TerminalUp::Output(output) = up(&line.unwrap()) else {
            panic!("output");
        };
        panes.push(output.pane);
    }
    assert_eq!(panes, ["w1:p1", "w1:p2", "w1:p1"]);
}

/// A link that stopped taking lines past the device's unsent reports
/// loses no pane's state: a later report takes the place of the waiting one
/// about its pane, so the core still hears each pane's last word.
#[test]
fn a_device_past_its_unsent_reports_keeps_each_panes_last_state() {
    let uplink = Arc::new(Uplink::default());
    let reports = UplinkReports(Arc::clone(&uplink));
    for at in 0..2 * MAX_UNSENT_REPORT_LINES as u64 {
        reports.report(TerminalReport::Input {
            pane: "w1:p1".into(),
            at_unix_ms: at,
            submitted: false,
            focus: false,
        });
    }
    let state = PaneTerminalState {
        state: "controlling".into(),
        mode: Some("control".into()),
        generation: 3,
        attempt: 1,
        message: None,
        exit_category: None,
        retry_decision: "none".into(),
        last_attempt_at_unix_ms: None,
    };
    reports.report(TerminalReport::State {
        pane: "w1:p2".into(),
        state: state.clone(),
    });
    let mut last_key = None;
    let mut states = Vec::new();
    while let (Some(line), _) = uplink.next().unwrap() {
        match up(&line) {
            TerminalUp::Report {
                report: TerminalReport::Input { at_unix_ms, .. },
            } => last_key = Some(at_unix_ms),
            TerminalUp::Report {
                report: TerminalReport::State { pane, state },
            } => states.push((pane, state)),
            _ => {}
        }
        if lock(&uplink.state).reports.is_empty() {
            break;
        }
    }
    assert_eq!(states, [("w1:p2".to_owned(), state)]);
    assert_eq!(last_key, Some(2 * MAX_UNSENT_REPORT_LINES as u64 - 1));
}

/// B4: while the core has no screen of its own drawing a node's panes, the
/// node's pane output never goes up the link; once one does, each pane that
/// had output is drawn again whole, and nothing of what was withheld follows.
#[test]
fn a_withheld_panes_output_stays_off_the_link_until_the_core_draws_it_whole() {
    let uplink = Uplink::default();
    uplink.withhold(true);
    uplink.push_output("w1:p1", b"typed on this machine", false);
    uplink.push_output("w1:p1", b"\x1bcwhole while withheld", true);
    uplink.withhold(false);
    let (line, redraws) = uplink.next().unwrap();
    assert!(line.is_none(), "withheld output went up the link");
    assert_eq!(redraws, ["w1:p1"]);
    // Only the full frame the redraw brings goes up, never the withheld tail.
    uplink.push_output("w1:p1", b"late", false);
    uplink.push_output("w1:p1", b"\x1bcredrawn", true);
    let (line, _) = uplink.next().unwrap();
    let TerminalUp::Output(output) = up(&line.unwrap()) else {
        panic!("output");
    };
    assert!(output.full);
    assert_eq!(decode_base64(&output.data).unwrap(), b"\x1bcredrawn");
}

#[test]
fn a_pane_past_its_unsent_output_loses_it_and_is_drawn_again_from_a_full_frame() {
    let uplink = Uplink::default();
    uplink.push_output("w1:p2", b"neighbour", false);
    let chunk = vec![b'x'; 64 * 1024];
    for _ in 0..20 {
        uplink.push_output("w1:p1", &chunk, false);
    }
    // The flooding pane's queue is gone and it is asked to draw again; its
    // neighbour's output is untouched.
    let (line, redraws) = uplink.next().unwrap();
    assert_eq!(redraws, ["w1:p1"]);
    let TerminalUp::Output(output) = up(&line.unwrap()) else {
        panic!("output");
    };
    assert_eq!(output.pane, "w1:p2");
    // Until the full frame, nothing of the dropped pane goes up.
    uplink.push_output("w1:p1", b"tail", false);
    uplink.push_output("w1:p1", b"\x1bcwhole", true);
    let (line, redraws) = uplink.next().unwrap();
    assert!(redraws.is_empty());
    let TerminalUp::Output(output) = up(&line.unwrap()) else {
        panic!("output");
    };
    assert!(output.full);
    assert_eq!(decode_base64(&output.data).unwrap(), b"\x1bcwhole");
    let state = lock(&uplink.state);
    assert!(
        state
            .panes
            .values()
            .all(|pane| pane.bytes <= MAX_UNSENT_OUTPUT_BYTES)
    );
}

/// A device's diagnostic reaches this machine's log with its plain fields
/// only, cut to size, and the device it came from.
#[test]
fn a_devices_diagnostic_keeps_its_plain_fields_cut_to_size() {
    let mut record = serde_json::Map::new();
    record.insert("kind".into(), json!("terminal.output_overflow"));
    record.insert("bytes".into(), json!(42));
    record.insert("message".into(), json!("m".repeat(10_000)));
    record.insert("nested".into(), json!({"a": [1, 2, 3]}));
    record.insert("n".repeat(100), json!(1));
    record.insert("device".into(), json!("someone-else"));
    for index in 0..100 {
        record.insert(format!("field{index:03}"), json!(index));
    }
    let kept = device_record("mini", serde_json::Value::Object(record));
    let kept = kept.as_object().unwrap();
    assert_eq!(kept["kind"], "terminal.output_overflow");
    assert_eq!(kept["bytes"], 42);
    assert_eq!(kept["device"], "mini");
    assert_eq!(kept["message"].as_str().unwrap().len(), DEVICE_RECORD_TEXT);
    assert!(!kept.contains_key("nested"));
    assert!(kept.len() <= DEVICE_RECORD_FIELDS + 4);
    assert!(kept["fields_left_out"].as_u64().unwrap() > 0);
    assert_eq!(
        device_record("mini", json!("not a record")),
        json!({"fields_left_out": 1, "device": "mini"})
    );
}

fn node(socket_seen: Sender<String>) -> (NodeTerminals, Receiver<Opened>) {
    let (opened_tx, opened) = channel();
    let attacher = Arc::new(FakeAttacher {
        opened: Mutex::new(opened_tx),
        refuse: Mutex::new(None),
    });
    let socket_seen = Mutex::new(socket_seen);
    let node = NodeTerminals::with_attacher(Box::new(move |socket: &str| {
        socket_seen.lock().unwrap().send(socket.to_owned()).unwrap();
        struct Shared(Arc<FakeAttacher>);
        impl super::super::Attacher for Shared {
            fn open(
                &self,
                pane: &str,
                mode: super::super::Mode,
                rows: u16,
                cols: u16,
            ) -> Result<super::super::SessionParts, String> {
                self.0.open(pane, mode, rows, cols)
            }
        }
        Box::new(Shared(Arc::clone(&attacher)))
    }));
    (node, opened)
}

fn attach(node: &NodeTerminals, opened: &Receiver<Opened>, pane: &str) -> Opened {
    node.line(
        down(TerminalControl::Attach {
            pane: pane.into(),
            size: Some(SIZE),
            manual: false,
        })
        .trim_ascii_end(),
    );
    opened.recv_timeout(WAIT).expect("the pane attached")
}

/// D-18, B18: one pane flooding its output holds neither another pane's
/// keys nor its frames.
#[test]
fn a_flooding_pane_holds_back_neither_another_panes_keys_nor_its_frames() {
    let (socket_tx, socket) = channel();
    let (node, opened) = node(socket_tx);
    node.start("/tmp/herdr.sock").unwrap();
    assert_eq!(socket.recv().unwrap(), "/tmp/herdr.sock");
    let flooding = attach(&node, &opened, "w1:p1");
    let quiet = attach(&node, &opened, "w1:p2");
    flooding.frame(SIZE, true, b"start");
    quiet.frame(SIZE, true, b"start");
    // Nobody reads the link: the flooding pane's output piles up past its
    // cap while the other pane's keys still reach its writer.
    for _ in 0..40 {
        flooding.frame(SIZE, false, &[b'y'; 48 * 1024]);
    }
    node.line(key_line("w1:p2", b"typed").trim_ascii_end());
    assert_eq!(quiet.next_input(), b"typed");
    quiet.frame(SIZE, false, b"echo");
    // The quiet pane's echo goes up within a few lines of the reader
    // coming back, not behind the flood.
    let mut seen = Vec::new();
    let deadline = std::time::Instant::now() + WAIT;
    while std::time::Instant::now() < deadline {
        let line = node.next_up().unwrap();
        if let TerminalUp::Output(output) = up(&line) {
            let echo =
                output.pane == "w1:p2" && decode_base64(&output.data).unwrap().ends_with(b"echo");
            seen.push(output.pane);
            if echo {
                break;
            }
        }
    }
    let flood_lines_before_echo = seen.iter().filter(|pane| *pane == "w1:p1").count();
    assert!(
        seen.last().is_some_and(|pane| pane == "w1:p2"),
        "the quiet pane's echo went up: {seen:?}"
    );
    assert!(
        flood_lines_before_echo <= 3,
        "the echo waited behind {flood_lines_before_echo} flood lines"
    );
    node.stop();
    assert!(node.next_up().is_none());
}

/// A link that takes lines when the test lets it.
#[derive(Default)]
struct GatedLink {
    open: Mutex<bool>,
    opened: Condvar,
    written: Mutex<Vec<Vec<u8>>>,
    fail: Mutex<Option<LineRefused>>,
    /// Lines answered busy before the link takes one.
    busy: Mutex<usize>,
    /// Why the writer ended the link, once it has.
    ended: Mutex<Option<String>>,
}

impl GatedLink {
    fn release(&self) {
        *self.open.lock().unwrap() = true;
        self.opened.notify_all();
    }

    fn lines(&self) -> Vec<TerminalDown> {
        self.written
            .lock()
            .unwrap()
            .iter()
            .map(|line| {
                serde_json::from_slice::<TerminalLine<TerminalDown>>(line)
                    .unwrap()
                    .terminal
            })
            .collect()
    }
}

impl LineLink for GatedLink {
    fn send_line(&self, line: &[u8]) -> Result<(), LineRefused> {
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.opened.wait(open).unwrap();
        }
        if let Some(refused) = self.fail.lock().unwrap().clone() {
            return Err(refused);
        }
        let mut busy = self.busy.lock().unwrap();
        if *busy > 0 {
            *busy -= 1;
            return Err(LineRefused::Busy);
        }
        self.written.lock().unwrap().push(line.to_vec());
        Ok(())
    }

    fn end(&self, reason: &str) {
        *self.ended.lock().unwrap() = Some(reason.to_owned());
    }
}

/// A device, pane, the bytes, and whether they draw the whole screen.
type HeardOutput = (String, String, Vec<u8>, bool);

#[derive(Default)]
struct Heard {
    output: Mutex<Vec<HeardOutput>>,
    reports: Mutex<Vec<(String, TerminalReport)>>,
}

impl DeviceSink for Heard {
    fn output(&self, device: &str, pane: &str, bytes: &[u8], full: bool) {
        self.output
            .lock()
            .unwrap()
            .push((device.into(), pane.into(), bytes.to_vec(), full));
    }

    fn report(&self, device: &str, report: TerminalReport) {
        self.reports.lock().unwrap().push((device.into(), report));
    }
}

fn proxy() -> (DeviceTerminals, Arc<GatedLink>, Arc<Heard>) {
    let link = Arc::new(GatedLink::default());
    let heard = Arc::new(Heard::default());
    let proxy = DeviceTerminals::start(
        "mini",
        Arc::clone(&link) as Arc<dyn LineLink>,
        Arc::clone(&heard) as Arc<dyn DeviceSink>,
    )
    .unwrap();
    (proxy, link, heard)
}

fn wait_for(mut done: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + WAIT;
    while !done() {
        assert!(std::time::Instant::now() < deadline, "timed out");
        std::thread::yield_now();
    }
}

fn controlling() -> PaneTerminalState {
    PaneTerminalState {
        state: "controlling".into(),
        mode: Some("control".into()),
        generation: 4,
        attempt: 1,
        message: None,
        exit_category: None,
        retry_decision: "none".into(),
        last_attempt_at_unix_ms: None,
    }
}

#[test]
fn a_pane_past_its_unsent_keys_reads_ended_while_other_panes_keys_go_on() {
    let (proxy, link, heard) = proxy();
    let inbound = proxy.inbound();
    proxy.control(TerminalControl::Attach {
        pane: "w1:p1".into(),
        size: Some(SIZE),
        manual: false,
    });
    inbound(
        &line_of(TerminalUp::Report {
            report: TerminalReport::State {
                pane: "w1:p1".into(),
                state: controlling(),
            },
        })
        .unwrap(),
    );
    // The link takes nothing: 256 KiB of one pane's keys fill its flow.
    let chunk = vec![b'k'; 16 * 1024];
    let mut taken = 0;
    while heard
        .reports
        .lock()
        .unwrap()
        .iter()
        .all(|(_, report)| !matches!(report, TerminalReport::Error { .. }))
    {
        proxy.key(KeyTarget::Pane("w1:p1".into()), chunk.clone(), 1);
        taken += 1;
        assert!(taken < 64, "the cap was never reached");
    }
    let reports = heard.reports.lock().unwrap().clone();
    let ended = reports
        .iter()
        .rev()
        .find_map(|(_, report)| match report {
            TerminalReport::State { pane, state } if pane == "w1:p1" => Some(state.clone()),
            _ => None,
        })
        .expect("the pane reads ended");
    assert_eq!(ended.state, "ended");
    assert_eq!(ended.generation, 4);
    // Refused once; later keys add nothing and send nothing.
    proxy.key(KeyTarget::Pane("w1:p1".into()), b"late".to_vec(), 1);
    assert_eq!(heard.reports.lock().unwrap().len(), reports.len());
    // The device's own word on the pane waits until it is attached again.
    inbound(
        &line_of(TerminalUp::Report {
            report: TerminalReport::State {
                pane: "w1:p1".into(),
                state: controlling(),
            },
        })
        .unwrap(),
    );
    assert_eq!(heard.reports.lock().unwrap().len(), reports.len());
    // Another pane's keys still go.
    proxy.key(KeyTarget::Pane("w1:p2".into()), b"other".to_vec(), 1);
    link.release();
    wait_for(|| {
        link.lines().iter().any(|line| {
            matches!(line, TerminalDown::Key { target: KeyTarget::Pane(pane), .. } if pane == "w1:p2")
        })
    });
    // The keys it took before the cap were written, in order, before it.
    let lines = link.lines();
    let first_keys = lines
        .iter()
        .filter(|line| matches!(line, TerminalDown::Key { .. }))
        .take_while(|line| {
            matches!(line, TerminalDown::Key { target: KeyTarget::Pane(pane), .. } if pane == "w1:p1")
        })
        .count();
    assert_eq!(first_keys, taken - 1);
    // Attaching again lets keys through.
    proxy.control(TerminalControl::Attach {
        pane: "w1:p1".into(),
        size: None,
        manual: true,
    });
    proxy.key(KeyTarget::Pane("w1:p1".into()), b"again".to_vec(), 1);
    wait_for(|| {
        link.lines().iter().any(|line| {
            matches!(line, TerminalDown::Key { data, .. } if decode_base64(data).unwrap() == b"again")
        })
    });
}

/// Every key line the link wrote, as (pane, bytes, typed at).
fn key_lines(link: &GatedLink) -> Vec<(String, Vec<u8>, u64)> {
    link.lines()
        .into_iter()
        .filter_map(|line| match line {
            TerminalDown::Key {
                target: KeyTarget::Pane(pane),
                data,
                typed_at_unix_ms,
            } => Some((pane, decode_base64(&data).unwrap(), typed_at_unix_ms)),
            _ => None,
        })
        .collect()
}

/// Holds the writer on a first control line, so everything after it waits.
fn stall(proxy: &DeviceTerminals) {
    proxy.control(TerminalControl::Release {
        pane: "w9:p9".into(),
        message: "stall".into(),
    });
}

/// The cap counts keys, not the frames they travel in (D-18, B18): with the
/// link stalled a pane takes 256 KiB of one-byte keys, the next byte
/// refuses only that pane, another pane's keys still go, and once the link
/// drains every key taken arrives, in order.
#[test]
fn a_stalled_link_takes_256_kib_of_one_byte_keys_and_refuses_only_the_next() {
    let (proxy, link, heard) = proxy();
    stall(&proxy);
    let typed: Vec<u8> = (0..MAX_UNSENT_KEY_BYTES)
        .map(|index| b'a' + (index % 26) as u8)
        .collect();
    for (index, key) in typed.iter().enumerate() {
        proxy.key(KeyTarget::Pane("w1:p1".into()), vec![*key], index as u64);
    }
    assert!(
        heard.reports.lock().unwrap().is_empty(),
        "no key was refused"
    );
    proxy.key(KeyTarget::Pane("w1:p1".into()), b"z".to_vec(), u64::MAX);
    let refused: Vec<String> = heard
        .reports
        .lock()
        .unwrap()
        .iter()
        .filter_map(|(_, report)| match report {
            TerminalReport::Error { pane, kind, .. } => Some(format!("{pane} {kind}")),
            _ => None,
        })
        .collect();
    assert_eq!(refused, ["w1:p1 terminal.device_input_overflow"]);
    proxy.key(KeyTarget::Pane("w1:p2".into()), b"other".to_vec(), 1);
    link.release();
    let arrived = |pane: &str| -> Vec<u8> {
        key_lines(&link)
            .into_iter()
            .filter(|(owner, _, _)| owner == pane)
            .flat_map(|(_, bytes, _)| bytes)
            .collect()
    };
    wait_for(|| arrived("w1:p2") == b"other");
    assert_eq!(arrived("w1:p1"), typed);
    // Runs are bounded: 256 KiB went down in 16 KiB lines.
    let lines = key_lines(&link);
    assert!(lines.iter().all(|(_, bytes, _)| bytes.len() <= 16 * 1024));
    assert_eq!(lines.len(), MAX_UNSENT_KEY_BYTES / (16 * 1024) + 1);
}

/// A run of waiting keys never crosses a pane, a control line, an Enter or
/// an escape, so the node's submit rule reads the chunks it would have
/// read; a merged run carries its last key's time.
#[test]
fn a_waiting_run_of_keys_stops_at_a_pane_a_control_an_enter_and_an_escape() {
    let (proxy, link, _heard) = proxy();
    stall(&proxy);
    let key = |pane: &str, bytes: &[u8], at: u64| {
        proxy.key(KeyTarget::Pane(pane.into()), bytes.to_vec(), at)
    };
    key("w1:p1", b"a", 1);
    key("w1:p1", b"b", 2);
    key("w1:p1", b"\r", 3);
    key("w1:p1", b"c", 4);
    key("w1:p2", b"x", 5);
    key("w1:p1", b"d", 6);
    proxy.control(TerminalControl::Attach {
        pane: "w1:p3".into(),
        size: None,
        manual: false,
    });
    key("w1:p1", b"e", 7);
    key("w1:p1", b"\x1b", 8);
    key("w1:p1", b"\r", 9);
    key("w1:p1", b"f", 10);
    key("w1:p1", b"g", 11);
    link.release();
    wait_for(|| key_lines(&link).len() == 9);
    let lines: Vec<(String, Vec<u8>, u64)> = key_lines(&link);
    let expected: Vec<(String, Vec<u8>, u64)> = [
        ("w1:p1", &b"ab"[..], 2),
        ("w1:p1", b"\r", 3),
        ("w1:p1", b"c", 4),
        ("w1:p2", b"x", 5),
        ("w1:p1", b"d", 6),
        ("w1:p1", b"e", 7),
        ("w1:p1", b"\x1b", 8),
        ("w1:p1", b"\r", 9),
        ("w1:p1", b"fg", 11),
    ]
    .into_iter()
    .map(|(pane, bytes, at)| (pane.to_owned(), bytes.to_vec(), at))
    .collect();
    assert_eq!(lines, expected);
    // The control went down between `d` and `e`.
    let all = link.lines();
    let control = all
        .iter()
        .position(|line| {
            matches!(
                line,
                TerminalDown::Control {
                    control: TerminalControl::Attach { .. }
                }
            )
        })
        .unwrap();
    let d = all
        .iter()
        .position(|line| matches!(line, TerminalDown::Key { data, .. } if decode_base64(data).unwrap() == b"d"))
        .unwrap();
    assert_eq!(control, d + 1);
}

/// The controls, views and redraws waiting for a device's link are capped
/// (principle 15): with the link stalled, views past the count stop the
/// queue growing, drop what waited, refuse later keys with a reason, and
/// end the link once its writer is free, as a link that ends does (D-19).
#[test]
fn a_stalled_link_past_what_may_wait_is_ended_rather_than_grown() {
    let (views, link, heard) = proxy();
    stall(&views);
    // The writer holds the stalling line before the flood starts, so the
    // line on its way is that one.
    wait_for(|| lock(&views.shared.state).lines.is_empty());
    let mut most = 0;
    for _ in 0..MAX_WAITING_LINES + 100 {
        views.view("w1:p1", SIZE, false);
        // The line already on its way counts until the link takes it.
        most = most.max(lock(&views.shared.state).waiting_lines);
    }
    assert_eq!(most, MAX_WAITING_LINES);
    assert!(
        lock(&views.shared.state).lines.is_empty(),
        "what waited was dropped"
    );
    views.key(KeyTarget::Pane("w1:p1".into()), b"late".to_vec(), 1);
    let refused = |heard: &Heard| {
        heard
            .reports
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, report)| {
                matches!(report, TerminalReport::Error { kind, .. } if kind == "terminal.device_disconnected")
            })
            .count()
    };
    assert_eq!(refused(&heard), 1);
    link.release();
    wait_for(|| link.ended.lock().unwrap().is_some());
    assert_eq!(
        link.lines().len(),
        1,
        "only the line already on its way was written"
    );
}

/// Keys bind to their pane's caps first (B18): with the link stalled, 64
/// panes each just under their unsent bytes never end the link, a 65th
/// pane and a pane past its waiting runs are refused alone, and every key
/// taken arrives once the link drains.
#[test]
fn every_panes_keys_at_their_caps_end_no_link_and_refuse_only_their_pane() {
    let (proxy, link, heard) = proxy();
    stall(&proxy);
    let keys = vec![b'k'; MAX_UNSENT_KEY_BYTES - MAX_PANE_KEY_LINES];
    for pane in 0..MAX_KEY_PANES {
        proxy.key(KeyTarget::Pane(format!("w1:p{pane}")), keys.clone(), 1);
    }
    let runs = lock(&proxy.shared.state).keys["w1:p0"].lines;
    let enters = MAX_PANE_KEY_LINES - runs;
    for _ in 0..enters {
        proxy.key(KeyTarget::Pane("w1:p0".into()), b"\r".to_vec(), 1);
    }
    assert!(
        heard.reports.lock().unwrap().is_empty(),
        "no key was refused"
    );
    proxy.key(KeyTarget::Pane("w1:p0".into()), b"\r".to_vec(), 1);
    proxy.key(KeyTarget::Pane("w9:p9".into()), b"x".to_vec(), 1);
    proxy.key(KeyTarget::Pane("w1:p1".into()), b"\r".to_vec(), 1);
    let refused: Vec<String> = heard
        .reports
        .lock()
        .unwrap()
        .iter()
        .filter_map(|(_, report)| match report {
            TerminalReport::Error { pane, kind, .. } => Some(format!("{pane} {kind}")),
            _ => None,
        })
        .collect();
    assert_eq!(
        refused,
        [
            "w1:p0 terminal.device_input_overflow",
            "w9:p9 terminal.device_input_overflow"
        ]
    );
    {
        let state = lock(&proxy.shared.state);
        assert!(
            state.failed.is_none() && !state.end_link,
            "the link goes on"
        );
    }
    link.release();
    let arrived = |pane: &str| -> usize {
        key_lines(&link)
            .into_iter()
            .filter(|(owner, _, _)| owner == pane)
            .map(|(_, bytes, _)| bytes.len())
            .sum()
    };
    wait_for(|| arrived("w1:p63") == keys.len());
    assert_eq!(arrived("w1:p0"), keys.len() + enters);
    assert_eq!(arrived("w1:p1"), keys.len() + 1);
    for pane in 2..MAX_KEY_PANES {
        assert_eq!(arrived(&format!("w1:p{pane}")), keys.len(), "w1:p{pane}");
    }
    assert!(link.ended.lock().unwrap().is_none());
}

#[test]
fn a_failed_link_refuses_keys_once_per_pane_and_never_keeps_them() {
    let (proxy, link, heard) = proxy();
    *link.fail.lock().unwrap() = Some(LineRefused::Ended("the link closed".into()));
    proxy.key(KeyTarget::Pane("w1:p1".into()), b"lost".to_vec(), 1);
    proxy.key(KeyTarget::Pane("w1:p1".into()), b"queued".to_vec(), 1);
    link.release();
    wait_for(|| lock(&proxy.shared.state).failed.is_some());
    assert!(lock(&proxy.shared.state).lines.is_empty());
    proxy.key(KeyTarget::Pane("w1:p1".into()), b"after".to_vec(), 1);
    proxy.key(KeyTarget::Pane("w1:p1".into()), b"after".to_vec(), 1);
    let errors = heard
        .reports
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, report)| {
            matches!(report, TerminalReport::Error { kind, .. } if kind == "terminal.device_disconnected")
        })
        .count();
    assert_eq!(errors, 1);
    assert!(link.lines().is_empty());
}

/// Keys to more panes than a report-once set remembers are all refused,
/// the panes past it are told once between them, and the set stays bounded
/// whatever pane ids the keys name; a forgotten pane leaves it.
#[test]
fn a_failed_link_tells_a_bounded_set_of_panes() {
    let (proxy, link, heard) = proxy();
    *link.fail.lock().unwrap() = Some(LineRefused::Ended("the link closed".into()));
    proxy.key(KeyTarget::Pane("w1:p1".into()), b"lost".to_vec(), 1);
    link.release();
    wait_for(|| lock(&proxy.shared.state).failed.is_some());
    for pane in 0..REMEMBERED_PANES + 100 {
        proxy.key(KeyTarget::Pane(format!("w9:p{pane}")), b"k".to_vec(), 1);
    }
    let errors = heard
        .reports
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, report)| {
            matches!(report, TerminalReport::Error { kind, .. } if kind == "terminal.device_disconnected")
        })
        .count();
    assert_eq!(errors, REMEMBERED_PANES + 1);
    assert_eq!(
        lock(&proxy.shared.state).told_failed.len(),
        REMEMBERED_PANES + 1
    );
    proxy.control(TerminalControl::Forget {
        pane: "w9:p0".into(),
    });
    assert_eq!(
        lock(&proxy.shared.state).told_failed.len(),
        REMEMBERED_PANES
    );
    assert!(link.lines().is_empty());
}

/// A device's error about its own pane is its word, not the operator's
/// refusal: it reaches the core as a note, never as the last error.
#[test]
fn a_devices_error_reaches_the_core_as_a_note() {
    let (proxy, _link, heard) = proxy();
    let inbound = proxy.inbound();
    inbound(
        &line_of(TerminalUp::Report {
            report: TerminalReport::Error {
                pane: "w1:p1".into(),
                kind: "remote.control.close_status_unknown".into(),
                message: "chosen by the device".into(),
            },
        })
        .unwrap(),
    );
    let reports = heard.reports.lock().unwrap();
    assert!(matches!(
        reports.as_slice(),
        [(device, TerminalReport::Note { pane, kind, .. })]
            if device == "mini" && pane == "w1:p1" && kind == "remote.control.close_status_unknown"
    ));
}

/// A proxy with no writer, whose record window stays open for the test, so
/// the records its lines make are emitted on the test's thread and no
/// second can pass mid-flood.
fn held_proxy() -> (ProxyShared, Arc<Heard>) {
    let heard = Arc::new(Heard::default());
    let shared = ProxyShared {
        device: "mini".into(),
        link: 7,
        permits: Arc::default(),
        state: Mutex::default(),
        ready: Condvar::new(),
        sink: Arc::clone(&heard) as Arc<dyn DeviceSink>,
        records: Mutex::new(RecordWindow::opened_at(
            Instant::now() + Duration::from_secs(3600),
        )),
    };
    (shared, heard)
}

fn kinds(records: &[serde_json::Value], kind: &str) -> Vec<serde_json::Value> {
    records
        .iter()
        .filter(|record| record["kind"] == kind)
        .cloned()
        .collect()
}

/// What a device's lines make this machine log is capped per second, so a
/// flood of unreadable lines writes at most the cap, and the rest are
/// counted in one record when the link's proxy ends.
#[test]
fn a_devices_unreadable_lines_write_at_most_the_record_cap() {
    let ((), records) = crate::diagnostics::capture(|| {
        let (shared, _heard) = held_proxy();
        for _ in 0..DEVICE_RECORDS_PER_WINDOW + 36 {
            shared.inbound(b"not a terminal line");
        }
        drop(shared);
    });
    assert_eq!(
        kinds(&records, "terminal.up_unreadable").len(),
        DEVICE_RECORDS_PER_WINDOW
    );
    let counted = kinds(&records, "terminal.device_records_unwritten");
    assert_eq!(counted.len(), 1);
    assert_eq!(counted[0]["count"], 36);
    assert_eq!(counted[0]["device"], "mini");
}

/// The cap is on bytes too: a device's largest diagnostics, every field cut
/// to its longest, write no more than the byte budget in one second.
#[test]
fn a_devices_largest_diagnostics_write_at_most_the_byte_budget() {
    let mut fields = serde_json::Map::new();
    for index in 0..DEVICE_RECORD_FIELDS {
        fields.insert(
            format!("field_{index}"),
            "x".repeat(DEVICE_RECORD_TEXT * 2).into(),
        );
    }
    let line = line_of(TerminalUp::Diagnostic {
        record: serde_json::Value::Object(fields),
    })
    .unwrap();
    let ((), records) = crate::diagnostics::capture(|| {
        let (shared, _heard) = held_proxy();
        for _ in 0..DEVICE_RECORDS_PER_WINDOW {
            shared.inbound(&line);
        }
        drop(shared);
    });
    let written: Vec<usize> = records
        .iter()
        .filter(|record| record["field_0"].is_string())
        .map(|record| record.to_string().len())
        .collect();
    assert_eq!(written.len(), 1);
    assert!(written.iter().sum::<usize>() <= DEVICE_RECORD_BYTES_PER_WINDOW);
    let counted = kinds(&records, "terminal.device_records_unwritten");
    assert_eq!(counted.len(), 1);
    assert_eq!(counted[0]["count"], DEVICE_RECORDS_PER_WINDOW - 1);
}

/// A device's notes and errors are log records too, so they count against
/// the same cap, while a pane's state still reaches the core past it.
#[test]
fn a_devices_notes_count_against_the_record_cap_and_its_states_do_not() {
    let (shared, heard) = held_proxy();
    let inbound = |line: &[u8]| shared.inbound(line);
    let error = line_of(TerminalUp::Report {
        report: TerminalReport::Error {
            pane: "w1:p1".into(),
            kind: "remote.control.close_status_unknown".into(),
            message: "chosen by the device".into(),
        },
    })
    .unwrap();
    for _ in 0..DEVICE_RECORDS_PER_WINDOW + 36 {
        inbound(&error);
    }
    inbound(
        &line_of(TerminalUp::Report {
            report: TerminalReport::State {
                pane: "w1:p1".into(),
                state: PaneTerminalState {
                    state: "live".into(),
                    mode: None,
                    generation: 1,
                    attempt: 1,
                    message: None,
                    exit_category: None,
                    retry_decision: "none".into(),
                    last_attempt_at_unix_ms: None,
                },
            },
        })
        .unwrap(),
    );
    let reports = heard.reports.lock().unwrap();
    let notes = reports
        .iter()
        .filter(|(_, report)| matches!(report, TerminalReport::Note { .. }))
        .count();
    assert_eq!(notes, DEVICE_RECORDS_PER_WINDOW);
    assert!(matches!(
        reports.last(),
        Some((_, TerminalReport::State { pane, .. })) if pane == "w1:p1"
    ));
}

/// A note is charged its line's size: one longer than the whole byte budget
/// never reaches the core and is counted, while a short one after it does.
#[test]
fn a_devices_note_past_the_byte_budget_is_counted_not_passed() {
    let note = |message: String| {
        line_of(TerminalUp::Report {
            report: TerminalReport::Note {
                pane: "w1:p1".into(),
                kind: "remote.control.close_status_unknown".into(),
                message,
            },
        })
        .unwrap()
    };
    let ((), records) = crate::diagnostics::capture(|| {
        let (shared, heard) = held_proxy();
        shared.inbound(&note("x".repeat(DEVICE_RECORD_BYTES_PER_WINDOW)));
        shared.inbound(&note("short".into()));
        let reports = heard.reports.lock().unwrap();
        assert!(matches!(
            reports.as_slice(),
            [(_, TerminalReport::Note { message, .. })] if message == "short"
        ));
        drop(reports);
        drop(shared);
    });
    let counted = kinds(&records, "terminal.device_records_unwritten");
    assert_eq!(counted.len(), 1);
    assert_eq!(counted[0]["count"], 1);
}

/// A window writes the cap, and what it did not write is counted once:
/// when the next window opens, or when the link's last holder lets go.
#[test]
fn a_record_window_counts_what_it_did_not_write_once() {
    let opened = Instant::now();
    let mut window = RecordWindow::opened_at(opened);
    let admitted = (0..DEVICE_RECORDS_PER_WINDOW + 36)
        .filter(|_| window.admit(opened, 100).0)
        .count();
    assert_eq!(admitted, DEVICE_RECORDS_PER_WINDOW);
    assert_eq!(
        window.admit(opened + DEVICE_RECORD_WINDOW, 100),
        (true, Some(36))
    );
    for _ in 0..DEVICE_RECORDS_PER_WINDOW + 5 {
        window.admit(opened + DEVICE_RECORD_WINDOW, 100);
    }
    assert_eq!(window.close(), Some(6));
    assert_eq!(window.close(), None);

    // Past the byte budget a record is counted even under the count cap,
    // and a smaller one that still fits is written.
    let later = opened + DEVICE_RECORD_WINDOW * 2;
    assert_eq!(
        window.admit(later, DEVICE_RECORD_BYTES_PER_WINDOW - 100),
        (true, None)
    );
    assert_eq!(window.admit(later, 101), (false, None));
    assert_eq!(window.admit(later, 100), (true, None));
    assert_eq!(window.close(), Some(1));
}

#[test]
fn output_from_the_device_reaches_the_hub_decoded_and_named_by_device() {
    let (proxy, _link, heard) = proxy();
    let inbound = proxy.inbound();
    inbound(
        &line_of(TerminalUp::Output(TerminalOutput {
            pane: "w1:p1".into(),
            data: encode_base64(b"\x1bcscreen"),
            full: true,
        }))
        .unwrap(),
    );
    assert_eq!(
        heard.output.lock().unwrap().as_slice(),
        [(
            "mini".to_owned(),
            "w1:p1".to_owned(),
            b"\x1bcscreen".to_vec(),
            true
        )]
    );
}

/// A link busy with another call past a line's wait sent nothing: the line
/// is written again, the keys behind it follow in order, and the pane is
/// not refused.
#[test]
fn a_busy_link_writes_the_line_again_and_refuses_nothing() {
    let (proxy, link, heard) = proxy();
    *link.busy.lock().unwrap() = 2;
    proxy.key(KeyTarget::Pane("w1:p1".into()), b"a".to_vec(), 1);
    proxy.key(KeyTarget::Pane("w1:p1".into()), b"\r".to_vec(), 2);
    proxy.key(KeyTarget::Pane("w1:p1".into()), b"b".to_vec(), 3);
    link.release();
    wait_for(|| key_lines(&link).len() == 3);
    assert_eq!(
        key_lines(&link),
        [
            ("w1:p1".to_owned(), b"a".to_vec(), 1),
            ("w1:p1".to_owned(), b"\r".to_vec(), 2),
            ("w1:p1".to_owned(), b"b".to_vec(), 3),
        ]
    );
    assert!(heard.reports.lock().unwrap().is_empty(), "nothing refused");
    proxy.key(KeyTarget::Pane("w1:p1".into()), b"c".to_vec(), 4);
    wait_for(|| key_lines(&link).len() == 4);
}

/// A frame larger than the unsent cap goes up whole when nothing of its pane
/// waits, so a pane whose full frame alone passes the cap draws, and no
/// redraw is asked for it again and again.
#[test]
fn a_frame_larger_than_the_cap_goes_up_whole_and_asks_no_redraw() {
    let uplink = Uplink::default();
    let frame = [b"\x1bc".to_vec(), vec![b'w'; 2 * 1024 * 1024]].concat();
    uplink.push_output("w1:p1", &frame, true);
    let (line, redraws) = uplink.next().unwrap();
    assert!(redraws.is_empty());
    let TerminalUp::Output(output) = up(&line.unwrap()) else {
        panic!("output");
    };
    assert_eq!(decode_base64(&output.data).unwrap(), frame);
    // A backlog that the next large chunk would push past the cap is the
    // one thing dropped: one redraw is asked, and its full frame goes up.
    uplink.push_output("w1:p1", b"small", false);
    uplink.push_output("w1:p1", &frame[2..], false);
    assert_eq!(uplink.next().unwrap(), (None, vec!["w1:p1".to_owned()]));
    uplink.push_output("w1:p1", &frame, true);
    let (line, redraws) = uplink.next().unwrap();
    assert!(redraws.is_empty(), "the redraw was asked once");
    let TerminalUp::Output(output) = up(&line.unwrap()) else {
        panic!("output");
    };
    assert!(output.full);
    assert_eq!(decode_base64(&output.data).unwrap(), frame);
}

#[test]
fn side_channel_permission_waits_for_the_device_and_ends_with_its_link() {
    let (proxy, link, heard) = proxy();
    link.release();
    let (reply, result) = channel();
    proxy.input_permit(
        "w1:p1",
        12,
        Box::new(move |outcome| {
            reply.send(outcome).unwrap();
        }),
    );
    wait_for(|| !link.lines().is_empty());
    let TerminalDown::InputPermit {
        ticket,
        pane,
        bytes,
    } = &link.lines()[0]
    else {
        panic!("permission request")
    };
    assert_eq!((pane.as_str(), *bytes), ("w1:p1", 12));
    assert!(matches!(
        result.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    (proxy.inbound())(
        &line_of(TerminalUp::Report {
            report: TerminalReport::InputPermit {
                ticket: *ticket,
                outcome: Ok(()),
            },
        })
        .unwrap(),
    );
    assert_eq!(result.recv_timeout(WAIT).unwrap(), Ok(()));
    assert!(heard.reports.lock().unwrap().is_empty());
    let (reply, result) = channel();
    proxy.input_permit(
        "w1:p1",
        12,
        Box::new(move |outcome| {
            let _ = reply.send(outcome);
        }),
    );
    drop(proxy);
    assert_eq!(
        result.recv_timeout(WAIT).unwrap(),
        Err(hide_node_link::terminal::InputPermitError::Discarded)
    );
}
