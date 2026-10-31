//! A pane's holder. See `Cargo.toml` and docs/ARCHITECTURE.md, "The pane
//! holder".
//!
//! ```text
//! hide-holder --socket <path> --rows <n> --cols <n> [--cwd <dir>] [--log <file>]
//!             [--id <pane>] -- <program> [<argument>...]
//! ```
//!
//! It is started as a resident child (`hide_platform::process::start_resident`)
//! lent one pipe, on which it says `ready holder=<pid> program=<pid>` once
//! the program runs and the socket listens, or `failed <reason>` and exits.

mod holder;
mod log;

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use hide_holder_protocol::Size;

/// What the holder was asked to hold.
pub struct Config {
    pub socket: PathBuf,
    pub size: Size,
    pub cwd: Option<PathBuf>,
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
        cwd,
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

fn main() -> ExitCode {
    // The pipe the starter reads this holder's start from; a holder started
    // by hand has none and says it on its standard error instead.
    let mut report: Box<dyn Write> = match hide_platform::process::take_lent() {
        Ok(mut lent) if !lent.is_empty() => Box::new(std::fs::File::from(lent.remove(0))),
        _ => Box::new(std::io::stderr()),
    };
    let config = match parse(std::env::args_os().skip(1)) {
        Ok(config) => config,
        Err(reason) => {
            let _ = writeln!(report, "failed {reason}");
            return ExitCode::from(2);
        }
    };
    let log = log::Log::open(config.log.as_deref(), &config.id);
    match holder::Holder::start(&config, log.clone()) {
        Ok(holder) => {
            let _ = writeln!(
                report,
                "ready holder={} program={}",
                std::process::id(),
                holder.program()
            );
            drop(report);
            holder.run()
        }
        Err(reason) => {
            log.event("holder.start_failed", &[("reason", reason.as_str().into())]);
            let _ = writeln!(report, "failed {}", reason.replace('\n', " "));
            ExitCode::FAILURE
        }
    }
}
