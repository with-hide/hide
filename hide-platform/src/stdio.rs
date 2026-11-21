//! A command-line program's standard output: when its reader closed it, and
//! the window it draws in when it is a terminal.

use std::io::{self, ErrorKind};
use std::panic::{self, PanicHookInfo};

/// The size of the terminal window standard output draws in, in character
/// cells: what a command that draws a picture (`hide phone pair`'s QR) checks
/// before it draws.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Window {
    pub columns: u16,
    pub rows: u16,
}

/// The window of the terminal standard output is, or an error when it is
/// not one (a pipe, a file) or the terminal does not say.
pub fn stdout_window() -> io::Result<Window> {
    let window = window::of_stdout()?;
    if window.columns == 0 || window.rows == 0 {
        return Err(io::Error::new(
            ErrorKind::Unsupported,
            "the terminal reports no window size",
        ));
    }
    Ok(window)
}

/// Lets standard output's terminal read colour escape sequences: Windows'
/// console reads them only once asked, every other system's terminal always
/// does. An error when standard output is not a console.
pub fn stdout_escapes() -> io::Result<()> {
    window::escapes()
}

#[cfg(unix)]
mod window {
    use super::Window;
    use std::io;

    pub(super) fn of_stdout() -> io::Result<Window> {
        // SAFETY: an all-zero `winsize` is a valid value of the plain struct.
        let mut size: libc::winsize = unsafe { std::mem::zeroed() };
        // SAFETY: TIOCGWINSZ writes one `winsize` into the place it is given.
        if unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ as _, &mut size) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Window {
            columns: size.ws_col,
            rows: size.ws_row,
        })
    }

    pub(super) fn escapes() -> io::Result<()> {
        // SAFETY: a plain call on this process's standard output.
        if unsafe { libc::isatty(libc::STDOUT_FILENO) } == 1 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

#[cfg(windows)]
mod window {
    use super::Window;
    use std::io;
    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Console::{
        CONSOLE_SCREEN_BUFFER_INFO, ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode,
        GetConsoleScreenBufferInfo, GetStdHandle, STD_OUTPUT_HANDLE, SetConsoleMode,
    };

    fn stdout() -> io::Result<HANDLE> {
        // SAFETY: a plain call; null or INVALID_HANDLE_VALUE means none.
        let handle = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "this process has no standard output",
            ));
        }
        Ok(handle)
    }

    pub(super) fn of_stdout() -> io::Result<Window> {
        let handle = stdout()?;
        // SAFETY: an all-zero CONSOLE_SCREEN_BUFFER_INFO is a valid value.
        let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
        // SAFETY: `info` is a valid place for the answer about our own handle.
        if unsafe { GetConsoleScreenBufferInfo(handle, &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // The visible window, not the buffer behind its scroll bar.
        let span = |low: i16, high: i16| u16::try_from(i32::from(high) - i32::from(low) + 1);
        Ok(Window {
            columns: span(info.srWindow.Left, info.srWindow.Right).unwrap_or(0),
            rows: span(info.srWindow.Top, info.srWindow.Bottom).unwrap_or(0),
        })
    }

    pub(super) fn escapes() -> io::Result<()> {
        let handle = stdout()?;
        let mut mode = 0u32;
        // SAFETY: `mode` is a valid place for the answer about our own handle.
        if unsafe { GetConsoleMode(handle, &mut mode) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0 {
            return Ok(());
        }
        // SAFETY: as above; the mode only adds escape reading.
        if unsafe { SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

/// The exit status of a program that ended because its reader left: 128 plus
/// `SIGPIPE`, what a shell reports for a command a closed pipe killed. Windows
/// has no signal, and uses the same number so a script reads one answer on
/// every system.
pub const CLOSED_STDOUT_EXIT: i32 = 141;

/// Ends the process quietly, with [`CLOSED_STDOUT_EXIT`], when a `print!` or
/// `println!` anywhere in it (any crate, any thread) fails because the reader
/// of standard output is gone, as `hide agent list | head` does. Every other
/// panic, including a print that fails for another reason such as a full
/// disk, goes to the hook that was installed before this one.
///
/// The standard library turns a failed print into a panic whose message is
/// `failed printing to stdout: <error> (os error <code>)`, and offers no
/// typed form of the error, so the code is read back from the message and
/// classified by the standard library itself: `BrokenPipe` is `EPIPE` on
/// Unix and `ERROR_BROKEN_PIPE` or `ERROR_NO_DATA` on Windows, and no table of
/// codes is kept here. Call it once, first thing in `main`, in a program that
/// prints to a pipe.
pub fn exit_quietly_when_stdout_closes() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        if closed_stdout(info) {
            std::process::exit(CLOSED_STDOUT_EXIT);
        }
        previous(info);
    }));
}

fn closed_stdout(info: &PanicHookInfo<'_>) -> bool {
    info.payload_as_str().is_some_and(is_closed_stdout_message)
}

fn is_closed_stdout_message(message: &str) -> bool {
    let Some(error) = message.strip_prefix("failed printing to stdout: ") else {
        return false;
    };
    let Some((_, code)) = error.rsplit_once("(os error ") else {
        return false;
    };
    code.strip_suffix(')')
        .and_then(|code| code.parse::<i32>().ok())
        .is_some_and(|code| io::Error::from_raw_os_error(code).kind() == ErrorKind::BrokenPipe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_broken_pipe_on_stdout_counts() {
        let broken = io::Error::from(ErrorKind::BrokenPipe);
        assert!(!is_closed_stdout_message(&format!(
            "failed printing to stdout: {broken}"
        )));
        #[cfg(unix)]
        {
            assert!(is_closed_stdout_message(
                "failed printing to stdout: Broken pipe (os error 32)"
            ));
            // A full disk is a failure the operator can act on.
            assert!(!is_closed_stdout_message(
                "failed printing to stdout: No space left on device (os error 28)"
            ));
        }
        #[cfg(windows)]
        {
            assert!(is_closed_stdout_message(
                "failed printing to stdout: The pipe is being closed. (os error 232)"
            ));
            assert!(!is_closed_stdout_message(
                "failed printing to stdout: There is not enough space on the disk. (os error 112)"
            ));
        }
        assert!(!is_closed_stdout_message(
            "failed printing to stderr: Broken pipe (os error 32)"
        ));
        assert!(!is_closed_stdout_message("something else (os error 32)"));
    }
}
