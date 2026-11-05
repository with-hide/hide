//! Whether a Windows pseudoconsole edits lines: the Windows form of
//! [`super::master_line_input`].
//!
//! A pseudoconsole keeps its program's input mode (`ENABLE_LINE_INPUT`) in
//! the console host, and the only way to ask it is from a process attached
//! to that console. A process attached to a console receives that console's
//! control events, and a close event ends it whatever its handler answers,
//! so the process that asks is never the pane's holder: it is a helper the
//! holder starts as an [`OwnedChild`] ([`ConsoleInput::start`]), which
//! attaches to the console the pane's shell runs in and answers one read per
//! request ([`serve`]). At worst the console's end takes the helper with it,
//! and the holder reads that as not knowing.
//!
//! The wire between the two is the helper's standard streams: once attached
//! the helper says [`READY`] (or [`FAILED`] and ends), then the owner writes
//! one byte to ask and the helper answers one byte, [`LINES`] or [`KEYS`],
//! or [`FAILED`] and ends. The helper ends when its standard input does,
//! which is when its owner lets it go or dies.
//!
//! The pane holder's watch tests run the real helper against a pane's console
//! on Windows (`hide-holder/tests/it/watch.rs`).

use std::io::{self, Read, Write};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, sync_channel};
use std::time::Duration;

use super::OwnedChild;

/// What the helper says once it is attached to the console.
pub const READY: u8 = b'r';
/// The answer for a console that edits lines (`ENABLE_LINE_INPUT` set).
pub const LINES: u8 = b'l';
/// The answer for a console that hands each key over as it arrives.
pub const KEYS: u8 = b'k';
/// What a helper says when it could not attach or read; it ends after it.
pub const FAILED: u8 = b'e';

/// How long one answer may take. The helper answers from the console host's
/// memory in well under a millisecond; one slower than this is stuck, and
/// a caller on a facts path must not wait on it.
pub const ANSWER_LIMIT: Duration = Duration::from_millis(500);

/// How long the helper may take to start and attach, a process start on a
/// loaded machine; past it the start is `TimedOut` and the helper is ended.
pub const READY_LIMIT: Duration = Duration::from_secs(2);

/// A helper attached to one pseudoconsole, which its owner asks whether the
/// console edits lines. Dropping it ends the helper.
pub struct ConsoleInput {
    /// Declared first, so it closes first: the helper reads its end and
    /// exits on its own before the child below is ended.
    ask: ChildStdin,
    answers: Receiver<io::Result<u8>>,
    helper: OwnedChild,
}

impl ConsoleInput {
    /// Starts `helper`, a command the caller built that runs [`serve`] for
    /// the console it means, with its standard streams given to this owner,
    /// and waits for it to attach, within [`READY_LIMIT`].
    pub fn start(helper: &mut Command) -> io::Result<Self> {
        helper
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = OwnedChild::spawn(helper)?;
        let (Some(ask), Some(mut said)) = (child.take_stdin(), child.take_stdout()) else {
            return Err(io::Error::other(
                "the helper's standard streams were not piped",
            ));
        };
        // One answer is in flight at a time, so a slot of one never blocks
        // the reader for long; it ends when the helper's output does.
        let (send, answers) = sync_channel(1);
        std::thread::Builder::new()
            .name("console-input".to_owned())
            .spawn(move || {
                let mut byte = [0u8; 1];
                loop {
                    let read = match said.read(&mut byte) {
                        Ok(0) => Err(io::ErrorKind::UnexpectedEof.into()),
                        Ok(_) => Ok(byte[0]),
                        Err(error) => Err(error),
                    };
                    let ended = read.is_err();
                    if send.send(read).is_err() || ended {
                        break;
                    }
                }
            })?;
        let started = Self {
            ask,
            answers,
            helper: child,
        };
        match started.answer(READY_LIMIT)? {
            READY => Ok(started),
            FAILED => Err(io::Error::other(
                "the helper could not attach to the console",
            )),
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("the helper said {other:#04x} for ready"),
            )),
        }
    }

    /// Whether the console edits lines now. An answer that does not come
    /// within [`ANSWER_LIMIT`] is `TimedOut`; a helper that could not read the
    /// console, or ended, is an error too, and a caller starts another to
    /// ask again.
    pub fn edits_lines(&mut self) -> io::Result<bool> {
        // An answer left from an ask that timed out is not this one's.
        while self.answers.try_recv().is_ok() {}
        self.ask.write_all(b"?")?;
        self.ask.flush()?;
        match self.answer(ANSWER_LIMIT)? {
            LINES => Ok(true),
            KEYS => Ok(false),
            FAILED => Err(io::Error::other("the helper could not read the console")),
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("the helper answered {other:#04x}"),
            )),
        }
    }

    /// The helper's process id.
    pub fn id(&self) -> u32 {
        self.helper.id()
    }

    /// The helper's next byte within `limit`.
    fn answer(&self, limit: Duration) -> io::Result<u8> {
        match self.answers.recv_timeout(limit) {
            Ok(read) => read,
            Err(RecvTimeoutError::Timeout) => Err(io::ErrorKind::TimedOut.into()),
            Err(RecvTimeoutError::Disconnected) => Err(io::ErrorKind::UnexpectedEof.into()),
        }
    }
}

/// The helper's side: attaches this process to the console `console` runs
/// in, says so, and answers, for each byte read from `asks`, whether that
/// console edits lines, until `asks` ends. A Ctrl+C or Ctrl+Break typed into
/// the pane is ignored, from before the attach; the console's close ends
/// this process, which is what the owner reads as the helper ending.
pub fn serve(console: u32, mut asks: impl Read, mut answers: impl Write) -> io::Result<()> {
    let mut say = |byte: u8| {
        answers.write_all(&[byte])?;
        answers.flush()
    };
    let input = match sys::attach(console) {
        Ok(input) => input,
        Err(error) => {
            say(FAILED)?;
            return Err(error);
        }
    };
    say(READY)?;
    let mut byte = [0u8; 1];
    loop {
        if asks.read(&mut byte)? == 0 {
            return Ok(());
        }
        match sys::edits_lines(&input) {
            Ok(edits) => say(if edits { LINES } else { KEYS })?,
            Err(error) => {
                say(FAILED)?;
                return Err(error);
            }
        }
    }
}

mod sys {
    use std::io;

    use windows_sys::Win32::Foundation::{
        CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Console::{
        AttachConsole, CTRL_BREAK_EVENT, CTRL_C_EVENT, ENABLE_LINE_INPUT, FreeConsole,
        GetConsoleMode, SetConsoleCtrlHandler,
    };
    use windows_sys::core::BOOL;

    /// The console's input buffer, open for this process.
    pub(super) struct Input(HANDLE);

    impl Drop for Input {
        fn drop(&mut self) {
            // SAFETY: the handle was opened by `attach` and is closed once.
            unsafe { CloseHandle(self.0) };
        }
    }

    /// Handles the pane's interrupts as done, so they do not end the helper;
    /// every other event (the console's close) takes the default, which ends
    /// it.
    unsafe extern "system" fn ignore_interrupts(event: u32) -> BOOL {
        BOOL::from(event == CTRL_C_EVENT || event == CTRL_BREAK_EVENT)
    }

    /// Leaves the console this process was given, attaches to the one
    /// `console` runs in and opens its input buffer.
    pub(super) fn attach(console: u32) -> io::Result<Input> {
        // SAFETY: plain calls with no pointers but the terminated name and a
        // handler that lives as long as the program. A helper started with
        // no console of its own fails `FreeConsole` harmlessly. The handler
        // is in place before the attach, so no interrupt finds the helper
        // without it.
        unsafe {
            if SetConsoleCtrlHandler(Some(ignore_interrupts), 1) == 0 {
                return Err(io::Error::last_os_error());
            }
            FreeConsole();
            if AttachConsole(console) == 0 {
                return Err(io::Error::last_os_error());
            }
            let name: Vec<u16> = "CONIN$".encode_utf16().chain(Some(0)).collect();
            let handle = CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            );
            if handle == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            Ok(Input(handle))
        }
    }

    pub(super) fn edits_lines(input: &Input) -> io::Result<bool> {
        let mut mode = 0u32;
        // SAFETY: `mode` is a valid place for the answer.
        if unsafe { GetConsoleMode(input.0, &mut mode) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(mode & ENABLE_LINE_INPUT != 0)
    }
}
