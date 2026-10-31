//! A pane's holder. See `Cargo.toml` and docs/ARCHITECTURE.md, "The pane
//! holder".
//!
//! ```text
//! hide-holder --socket <path> --rows <n> --cols <n> --cwd <dir> [--log <file>]
//!             [--id <pane>] [--record <file> --record-lock <file> --incarnation <n>]
//!             [--theme <theme>] -- <program> [<argument>...]
//! ```
//!
//! With `--record` it keeps its pane's record (`record.rs`): it writes
//! itself in before it starts the program and fails the start when the
//! record no longer reserves it.
//!
//! `--theme` is the node's theme in `Theme::to_arg`'s form, so the program's
//! first color question is answered with it; without it the parser's own
//! defaults answer until a theme message comes.
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

mod answers;
mod ask;
mod encode;
mod holder;
mod log;
mod pane;
mod record;
mod screen;

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
    pub record: Option<RecordArgs>,
    pub program: Vec<std::ffi::OsString>,
    pub theme: Option<hide_holder_protocol::Theme>,
}

/// The pane's record, its lock and the start this holder is.
pub struct RecordArgs {
    pub record: PathBuf,
    pub lock: PathBuf,
    pub incarnation: u64,
}

fn parse(mut arguments: impl Iterator<Item = std::ffi::OsString>) -> Result<Config, String> {
    let mut socket = None;
    let mut rows = None;
    let mut cols = None;
    let mut cwd = None;
    let mut log = None;
    let mut id = String::new();
    let mut record = None;
    let mut record_lock = None;
    let mut incarnation = None;
    let mut theme = None;
    let value = |name: &str, arguments: &mut dyn Iterator<Item = std::ffi::OsString>| {
        arguments
            .next()
            .ok_or_else(|| format!("{name} needs a value"))
    };
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--socket") => {
                socket = Some(absolute("--socket", value("--socket", &mut arguments)?)?)
            }
            Some("--rows") => rows = Some(number(value("--rows", &mut arguments)?)?),
            Some("--cols") => cols = Some(number(value("--cols", &mut arguments)?)?),
            Some("--cwd") => cwd = Some(absolute("--cwd", value("--cwd", &mut arguments)?)?),
            Some("--log") => log = Some(absolute("--log", value("--log", &mut arguments)?)?),
            Some("--id") => {
                id = value("--id", &mut arguments)?
                    .into_string()
                    .map_err(|_| "--id is not text".to_owned())?;
            }
            Some("--record") => {
                record = Some(absolute("--record", value("--record", &mut arguments)?)?)
            }
            Some("--record-lock") => {
                record_lock = Some(absolute(
                    "--record-lock",
                    value("--record-lock", &mut arguments)?,
                )?)
            }
            Some("--incarnation") => {
                incarnation = Some(
                    value("--incarnation", &mut arguments)?
                        .to_str()
                        .and_then(|text| text.parse::<u64>().ok())
                        .ok_or("--incarnation is not a number")?,
                )
            }
            Some("--theme") => {
                theme = Some(
                    value("--theme", &mut arguments)?
                        .to_str()
                        .ok_or_else(|| "not text".to_owned())
                        .and_then(hide_holder_protocol::Theme::from_arg)
                        .map_err(|error| format!("--theme: {error}"))?,
                )
            }
            Some("--") => break,
            _ => return Err(format!("unknown argument {argument:?}")),
        }
    }
    let program: Vec<_> = arguments.collect();
    if program.is_empty() {
        return Err("no program to hold after --".to_owned());
    }
    let record = match (record, record_lock, incarnation) {
        (Some(record), Some(lock), Some(incarnation)) => Some(RecordArgs {
            record,
            lock,
            incarnation,
        }),
        (None, None, None) => None,
        _ => return Err("--record, --record-lock and --incarnation go together".to_owned()),
    };
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
        record,
        program,
        theme,
    })
}

/// A path the holder is given, which must not depend on the folder it works
/// in: it leaves the one it was started from for its version folder.
fn absolute(name: &str, value: std::ffi::OsString) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(format!("{name} {} is not an absolute path", path.display()))
    }
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
    let claim = match &config.record {
        Some(args) => {
            match record::Claim::take(args.record.clone(), args.lock.clone(), args.incarnation) {
                Ok(claim) => Some(claim),
                Err(reason) => {
                    let reason = format!("the pane's record: {reason}");
                    log.event("holder.start_failed", &[("reason", reason.as_str().into())]);
                    report(&format!("failed {reason}"));
                    return ExitCode::FAILURE;
                }
            }
        }
        None => None,
    };
    // The start's own failure goes into the record too, so the node reads
    // why rather than a holder that ended without a word.
    let failure = claim.clone();
    match holder::Holder::start(&config, log.clone(), &job, claim) {
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
            if let Some(claim) = failure {
                let end = hide_holder_protocol::record::End::new(
                    hide_holder_protocol::record::EndReason::StartFailed,
                    reason.as_str(),
                );
                if let Err(error) = claim.end(end) {
                    log.event(
                        hide_holder_protocol::record::UNWRITTEN_EVENT,
                        &[
                            (
                                "reason",
                                hide_holder_protocol::record::EndReason::StartFailed
                                    .name()
                                    .into(),
                            ),
                            ("incarnation", claim.incarnation().into()),
                            ("error", error.as_str().into()),
                        ],
                    );
                }
            }
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

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::parse;

    fn arguments(socket: &str, cwd: &str, log: &str) -> Vec<OsString> {
        [
            "--socket", socket, "--rows", "24", "--cols", "80", "--cwd", cwd, "--log", log, "--",
            "sh",
        ]
        .into_iter()
        .map(OsString::from)
        .collect()
    }

    #[test]
    fn a_record_is_named_with_its_lock_and_its_start_or_not_at_all() {
        let root = if cfg!(windows) { "C:\\h" } else { "/h" };
        let at = |name: &str| format!("{root}/{name}");
        let with = |extra: &[&str]| {
            let mut words = arguments(&at("p.s"), &at("w"), &at("p.log"));
            let program = words.split_off(words.len() - 2);
            words.extend(extra.iter().map(OsString::from));
            words.extend(program);
            parse(words.into_iter())
        };
        let record = at("r/p.json");
        let lock = at("r/.lock");
        let parsed = with(&[
            "--record",
            &record,
            "--record-lock",
            &lock,
            "--incarnation",
            "9",
        ])
        .unwrap()
        .record
        .unwrap();
        assert_eq!(parsed.incarnation, 9);
        for partial in [
            vec!["--record", record.as_str()],
            vec!["--record", record.as_str(), "--record-lock", lock.as_str()],
            vec!["--incarnation", "9"],
        ] {
            assert!(with(&partial).is_err(), "{partial:?} was taken");
        }
        assert!(
            with(&[
                "--record",
                "r.json",
                "--record-lock",
                &lock,
                "--incarnation",
                "9"
            ])
            .is_err()
        );
    }

    #[test]
    fn a_relative_socket_cwd_or_log_is_refused() {
        let root = if cfg!(windows) { "C:\\h" } else { "/h" };
        let at = |name: &str| format!("{root}/{name}");
        assert!(parse(arguments(&at("p.s"), &at("w"), &at("p.log")).into_iter()).is_ok());
        for (socket, cwd, log, named) in [
            ("p.s".to_owned(), at("w"), at("p.log"), "--socket"),
            (at("p.s"), "w".to_owned(), at("p.log"), "--cwd"),
            (at("p.s"), at("w"), "p.log".to_owned(), "--log"),
        ] {
            let refused = parse(arguments(&socket, &cwd, &log).into_iter())
                .err()
                .unwrap_or_else(|| panic!("a relative {named} was taken"));
            assert!(refused.starts_with(named), "{refused}");
        }
    }
}
