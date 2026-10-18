//! A command-line program whose standard output was closed by its reader.

use std::io::{self, ErrorKind};
use std::panic::{self, PanicHookInfo};

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
