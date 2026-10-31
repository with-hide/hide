//! A holder that speaks protocol version 1, the oldest a node speaks to
//! (`OLDEST_HOLDER_VERSION`), written out byte for byte here rather than
//! through `hide-holder-protocol`, so the version this build's node must
//! keep speaking stays fixed however the protocol grows (PRD
//! engine-bundle-2-pane-holder B10). It is this test binary in the
//! `holder_v1` role: it listens at the socket a record names, draws a fixed
//! screen, logs every message a node sends it, answers a read and a process
//! list, and on a terminate writes its pane's end into the record and exits.
//! A message of a kind version 1 does not have is logged `unknown` and the
//! connection closed `protocol_error`, which the test reads as a node that
//! said something its holder does not know.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use hide_holder_protocol::record::{self, End, EndReason, Phase};
use hide_platform::ipc::{LocalListener, LocalStream};

/// The variables the role reads: where it listens, where it logs, and its
/// pane's record and the records' lock.
pub const SOCKET: &str = "HIDE_HOLDER_TEST_FIXTURE_SOCKET";
pub const LOG: &str = "HIDE_HOLDER_TEST_FIXTURE_LOG";
pub const RECORD: &str = "HIDE_HOLDER_TEST_FIXTURE_RECORD";
pub const LOCK: &str = "HIDE_HOLDER_TEST_FIXTURE_LOCK";

/// The screen it draws, and what it answers a read with.
pub const SCREEN_TEXT: &str = "fixture-v1";
pub const READ_TEXT: &str = "fixture-read";

/// How long the role lives with nobody ending it.
const LIFE: Duration = Duration::from_secs(120);

// Version 1's kinds, as the version was frozen.
const HELLO: u8 = 0x01;
const INPUT: u8 = 0x02;
const RESIZE: u8 = 0x03;
const RELEASE: u8 = 0x04;
const TERMINATE: u8 = 0x05;
const REDRAW: u8 = 0x10;
const PASTE: u8 = 0x11;
const MOUSE: u8 = 0x12;
const SCROLL: u8 = 0x13;
const THEME: u8 = 0x14;
const ASK: u8 = 0x18;
const WELCOME: u8 = 0x81;
const CLOSED: u8 = 0x83;
const FRAGMENT: u8 = 0x90;
const ANSWER: u8 = 0x91;

// Version 1's close codes.
const RELEASED: u16 = 1;
const TERMINATED: u16 = 2;
const TAKEN_OVER: u16 = 4;
const HELLO_INVALID: u16 = 5;
const PROTOCOL_ERROR: u16 = 9;

const MAGIC: &[u8; 8] = b"HIDEHOLD";

struct State {
    size: (u16, u16),
    epoch: u64,
    generation: u64,
    /// The connection that holds the pane.
    holding: Option<LocalStream>,
}

pub fn holder_v1() {
    let socket = PathBuf::from(std::env::var_os(SOCKET).unwrap());
    let listener = LocalListener::bind(&socket).unwrap();
    log("listening");
    let state = Arc::new(Mutex::new(State {
        size: (24, 80),
        epoch: 0,
        generation: 0,
        holding: None,
    }));
    std::thread::spawn(|| {
        #[allow(clippy::disallowed_methods)] // the role's own life
        std::thread::sleep(LIFE);
        std::process::exit(0);
    });
    loop {
        let stream = listener.accept().unwrap();
        let state = Arc::clone(&state);
        std::thread::spawn(move || serve(stream, &state));
    }
}

fn log(line: &str) {
    let path = std::env::var_os(LOG).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .unwrap();
    file.write_all(format!("{line}\n").as_bytes()).unwrap();
}

fn read_frame(stream: &mut LocalStream) -> Option<(u8, Vec<u8>)> {
    let mut length = [0u8; 4];
    stream.read_exact(&mut length).ok()?;
    let length = u32::from_le_bytes(length) as usize;
    if length == 0 || length > 1024 * 1024 {
        return None;
    }
    let mut frame = vec![0u8; length];
    stream.read_exact(&mut frame).ok()?;
    let kind = frame.remove(0);
    Some((kind, frame))
}

fn write_frame(stream: &mut LocalStream, kind: u8, payload: &[u8]) {
    let mut frame = ((payload.len() + 1) as u32).to_le_bytes().to_vec();
    frame.push(kind);
    frame.extend_from_slice(payload);
    let _ = stream.write_all(&frame).and_then(|()| stream.flush());
}

fn closed(stream: &mut LocalStream, code: u16) {
    let mut payload = code.to_le_bytes().to_vec();
    payload.extend_from_slice(&1u16.to_le_bytes()); // the holder's version
    payload.push(0); // no exit code
    payload.extend_from_slice(&0u16.to_le_bytes()); // no pids left
    payload.extend_from_slice(&0u16.to_le_bytes()); // no detail
    write_frame(stream, CLOSED, &payload);
}

fn full_screen(stream: &mut LocalStream, state: &mut State) {
    state.epoch += 1;
    let (rows, cols) = state.size;
    let mut payload = state.epoch.to_le_bytes().to_vec();
    payload.extend_from_slice(&0u64.to_le_bytes()); // seq
    payload.push(1); // full
    payload.extend_from_slice(&rows.to_le_bytes());
    payload.extend_from_slice(&cols.to_le_bytes());
    payload.extend_from_slice(b"\x1b[H\x1b[2J");
    payload.extend_from_slice(SCREEN_TEXT.as_bytes());
    write_frame(stream, FRAGMENT, &payload);
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn lock<'a>(state: &'a Mutex<State>) -> std::sync::MutexGuard<'a, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

fn serve(mut stream: LocalStream, state: &Mutex<State>) {
    let Some((kind, payload)) = read_frame(&mut stream) else {
        return;
    };
    if payload.len() < MAGIC.len() || &payload[..MAGIC.len()] != MAGIC {
        log("opening refused");
        closed(&mut stream, HELLO_INVALID);
        return;
    }
    let rest = &payload[MAGIC.len()..];
    match kind {
        HELLO => {
            let oldest = u16::from_le_bytes([rest[0], rest[1]]);
            let newest = u16::from_le_bytes([rest[2], rest[3]]);
            log(&format!("hello {oldest} {newest}"));
            if !(oldest..=newest).contains(&1) {
                closed(&mut stream, 7);
                return;
            }
            hold(stream, state);
        }
        ASK => {
            let version = u16::from_le_bytes([rest[0], rest[1]]);
            log(&format!("ask {version} {}", rest[2]));
            if version != 1 {
                closed(&mut stream, PROTOCOL_ERROR);
                return;
            }
            let mut answer = Vec::new();
            match rest[2] {
                1 => {
                    answer.push(1);
                    answer.push(0); // not truncated
                    answer.extend_from_slice(&(READ_TEXT.len() as u32).to_le_bytes());
                    answer.extend_from_slice(READ_TEXT.as_bytes());
                }
                _ => {
                    answer.push(2);
                    answer.extend_from_slice(&std::process::id().to_le_bytes());
                    answer.push(3); // foreground unreadable
                }
            }
            write_frame(&mut stream, ANSWER, &answer);
        }
        other => {
            log(&format!("unknown {other:#04x}"));
            closed(&mut stream, HELLO_INVALID);
        }
    }
}

fn hold(mut stream: LocalStream, state: &Mutex<State>) {
    let generation = {
        let mut state = lock(state);
        if let Some(mut previous) = state.holding.take() {
            closed(&mut previous, TAKEN_OVER);
            previous.shutdown_handle().shutdown();
        }
        state.generation += 1;
        let mut welcome = 1u16.to_le_bytes().to_vec();
        welcome.extend_from_slice(&state.generation.to_le_bytes());
        welcome.extend_from_slice(&std::process::id().to_le_bytes());
        welcome.extend_from_slice(&state.size.0.to_le_bytes());
        welcome.extend_from_slice(&state.size.1.to_le_bytes());
        welcome.push(1);
        welcome.extend_from_slice(&6u16.to_le_bytes());
        welcome.extend_from_slice(b"screen");
        write_frame(&mut stream, WELCOME, &welcome);
        full_screen(&mut stream, &mut state);
        state.holding = Some(stream.duplicate());
        state.generation
    };
    while let Some((kind, payload)) = read_frame(&mut stream) {
        let mut state = lock(state);
        if state.generation != generation {
            log(&format!("dropped {kind:#04x} of generation {generation}"));
            return;
        }
        match kind {
            INPUT => log(&format!("input {}", hex(&payload))),
            RESIZE => {
                let rows = u16::from_le_bytes([payload[0], payload[1]]);
                let cols = u16::from_le_bytes([payload[2], payload[3]]);
                log(&format!("resize {rows} {cols}"));
                state.size = (rows, cols);
                full_screen(&mut stream, &mut state);
            }
            REDRAW => {
                log("redraw");
                full_screen(&mut stream, &mut state);
            }
            SCROLL => log(&format!("scroll {}", hex(&payload))),
            PASTE => log(&format!("paste {}", hex(&payload))),
            MOUSE => log(&format!("mouse {}", hex(&payload))),
            THEME => log(&format!("theme {}", hex(&payload))),
            RELEASE => {
                log("release");
                closed(&mut stream, RELEASED);
                state.holding = None;
                return;
            }
            TERMINATE => {
                log("terminate");
                end_record();
                closed(&mut stream, TERMINATED);
                std::process::exit(0);
            }
            other => {
                log(&format!("unknown {other:#04x}"));
                closed(&mut stream, PROTOCOL_ERROR);
                state.holding = None;
                return;
            }
        }
    }
}

/// Writes the pane's end into its record, as a holder does before it exits.
fn end_record() {
    let path = PathBuf::from(std::env::var_os(RECORD).unwrap());
    let lock = PathBuf::from(std::env::var_os(LOCK).unwrap());
    let _lock = record::lock(&lock, record::LOCK_WAIT).unwrap().unwrap();
    let mut current = record::read(&path).unwrap().unwrap();
    current.phase = Phase::Ended;
    current.end = Some(End::new(
        EndReason::Terminated,
        "the fixture was asked to end",
    ));
    record::write(&path, &current).unwrap();
}
