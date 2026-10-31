//! A pane's holder. See `Cargo.toml` and docs/ARCHITECTURE.md, "The pane
//! holder".
//!
//! ```text
//! hide-holder --socket <path> --rows <n> --cols <n> --cwd <dir> [--log <file>]
//!             [--id <pane>] -- <program> [<argument>...]
//! ```
//!
//! It is started as a resident child (`hide_platform::process::start_resident`)
//! lent one pipe, on which it says `ready holder=<pid> program=<pid>
//! job=<limits|none>` once the program runs in `--cwd` and the socket
//! listens, or `failed <reason>` and exits. `job` is the Windows job that
//! holds the holder, which ends it when closed (`none` on Unix). The holder
//! itself works in the folder its program file is in, its version folder, so
//! it never keeps the folder it was started from.
//!
//! A panic on any thread ends the holder at once (`abort`) after one log
//! line, and a `failed` line when it had not said `ready`: the pane's
//! program then loses its terminal (Unix) or its job (Windows) and ends,
//! as when the holder is killed, rather than running on with part of the
//! holder gone.

mod holder;
mod log;

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Mutex, OnceLock, PoisonError, TryLockError};

use hide_holder_protocol::Size;

/// What the holder was asked to hold.
pub struct Config {
    pub socket: PathBuf,
    pub size: Size,
    pub cwd: PathBuf,
    pub log: Option<PathBuf>,
    pub id: String,
    pub program: Vec<std::ffi::OsString>,
}

fn parse(mut arguments: impl Iterator<Item = std::ffi::OsString>) -> Result<Config, String> {
    let mut socket = None;
    let mut rows = None;
    let mut cols = None;
    let mut cwd = None;
    let mut log = None;
    let mut id = String::new();
    let value = |name: &str, arguments: &mut dyn Iterator<Item = std::ffi::OsString>| {
        arguments
            .next()
            .ok_or_else(|| format!("{name} needs a value"))
    };
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--socket") => socket = Some(PathBuf::from(value("--socket", &mut arguments)?)),
            Some("--rows") => rows = Some(number(value("--rows", &mut arguments)?)?),
            Some("--cols") => cols = Some(number(value("--cols", &mut arguments)?)?),
            Some("--cwd") => cwd = Some(PathBuf::from(value("--cwd", &mut arguments)?)),
            Some("--log") => log = Some(PathBuf::from(value("--log", &mut arguments)?)),
            Some("--id") => {
                id = value("--id", &mut arguments)?
                    .into_string()
                    .map_err(|_| "--id is not text".to_owned())?;
            }
            Some("--") => break,
            _ => return Err(format!("unknown argument {argument:?}")),
        }
    }
    let program: Vec<_> = arguments.collect();
    if program.is_empty() {
        return Err("no program to hold after --".to_owned());
    }
    let size = Size::checked(
        rows.ok_or("--rows is missing")?,
        cols.ok_or("--cols is missing")?,
    )?;
    Ok(Config {
        socket: socket.ok_or("--socket is missing")?,
        size,
        cwd: cwd.ok_or("--cwd is missing")?,
        log,
        id,
        program,
    })
}

fn number(value: std::ffi::OsString) -> Result<u16, String> {
    value
        .to_str()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| format!("{value:?} is not a size"))
}

/// Where the holder says how its start went, until it has: the starter's
/// pipe, or standard error for a holder started by hand.
static REPORT: Mutex<Option<Box<dyn Write + Send>>> = Mutex::new(None);

/// The log, once it is open, for the panic hook.
static LOG: OnceLock<log::Log> = OnceLock::new();

/// Says `line` on the report and lets it go: the starter reads one line.
fn report(line: &str) {
    let taken = REPORT.lock().unwrap_or_else(PoisonError::into_inner).take();
    if let Some(mut report) = taken {
        let _ = writeln!(report, "{}", line.replace('\n', " "));
    }
}

fn on_panic(info: &std::panic::PanicHookInfo<'_>) {
    let message = info
        .payload()
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_default();
    let at = info
        .location()
        .map(|location| format!("{}:{}", location.file(), location.line()))
        .unwrap_or_default();
    let thread = std::thread::current().name().unwrap_or("").to_owned();
    if let Some(log) = LOG.get() {
        log.event_now(
            "holder.panicked",
            &[
                ("thread", thread.as_str().into()),
                ("message", message.as_str().into()),
                ("at", at.as_str().into()),
            ],
        );
    }
    // Never waits: the panicking thread may hold the report.
    let mut held = match REPORT.try_lock() {
        Ok(held) => held,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => std::process::abort(),
    };
    if let Some(report) = held.as_mut() {
        let _ = writeln!(
            report,
            "failed the holder panicked on {thread:?}: {message}"
        );
    }
    std::process::abort();
}

fn main() -> ExitCode {
    std::panic::set_hook(Box::new(on_panic));
    // The pipe the starter reads this holder's start from; a holder started
    // by hand has none and says it on its standard error instead.
    let lent = hide_platform::process::take_lent();
    let pipe: Box<dyn Write + Send> = match lent {
        Ok(mut lent) if !lent.is_empty() => Box::new(std::fs::File::from(lent.remove(0))),
        Ok(_) => Box::new(std::io::stderr()),
        Err(error) => {
            let _ = writeln!(
                std::io::stderr(),
                "failed the report pipe it was lent is not there: {error}"
            );
            return ExitCode::FAILURE;
        }
    };
    *REPORT.lock().unwrap_or_else(PoisonError::into_inner) = Some(pipe);
    let config = match parse(std::env::args_os().skip(1)) {
        Ok(config) => config,
        Err(reason) => {
            report(&format!("failed {reason}"));
            return ExitCode::from(2);
        }
    };
    let log = match log::Log::open(config.log.as_deref(), &config.id) {
        Ok(log) => log,
        Err(reason) => {
            report(&format!("failed {reason}"));
            return ExitCode::FAILURE;
        }
    };
    let _ = LOG.set(log.clone());
    if let Err(reason) = leave_start_folder() {
        log.event("holder.start_failed", &[("reason", reason.as_str().into())]);
        report(&format!("failed {reason}"));
        return ExitCode::FAILURE;
    }
    let job = match hide_platform::process::own_job() {
        Ok(job) => job.map_or_else(|| "none".to_owned(), |limits| limits.to_string()),
        Err(error) => {
            let reason = format!("the job holding the holder could not be read: {error}");
            log.event("holder.start_failed", &[("reason", reason.as_str().into())]);
            report(&format!("failed {reason}"));
            return ExitCode::FAILURE;
        }
    };
    match holder::Holder::start(&config, log.clone(), &job) {
        Ok(holder) => {
            report(&format!(
                "ready holder={} program={} job={job}",
                std::process::id(),
                holder.program()
            ));
            holder.run()
        }
        Err(reason) => {
            log.event("holder.start_failed", &[("reason", reason.as_str().into())]);
            report(&format!("failed {reason}"));
            ExitCode::FAILURE
        }
    }
}

/// Moves the holder into the folder its program file is in, so the folder
/// it was started from (an app's, on Windows kept open by a working folder)
/// is not held for as long as the pane lives.
fn leave_start_folder() -> Result<(), String> {
    let program = std::env::current_exe()
        .map_err(|error| format!("the holder's own file could not be found: {error}"))?;
    let folder = program
        .parent()
        .ok_or_else(|| format!("{} is in no folder", program.display()))?;
    std::env::set_current_dir(folder)
        .map_err(|error| format!("{} could not be worked in: {error}", folder.display()))
}
