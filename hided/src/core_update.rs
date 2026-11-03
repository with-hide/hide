//! `hided core-update`: the core on this machine is updated to this build
//! (PRD core-host-node-move B10, D-10). A node of a newer build runs it here
//! over SSH, once it put this build in a version folder beside the one the
//! core runs (`hide_node::ssh::upstream::Upstream::install_build`), so the
//! update needs nothing from the older core. It prints one JSON line and
//! exits 0 once it has an answer, a refusal included; a usage error exits
//! non-zero.
//!
//! The core is replaced by this build's through its starter (the account's
//! login item, whose replacement ends the old job), and has 30 s to take
//! links; one that does not is replaced by the previous build's again,
//! unless this machine's Herdr session already moved into the new build's
//! engine (PRD engine-bundle-4-herdr-removal D-27), which only goes forward. Both
//! ends leave `current` at the build that runs and keep the other build of
//! the update, so a later connection reuses its files and no more than two
//! builds are kept. A move holds this machine's handover record while it
//! runs, and the update holds that record's lock for its whole length, so
//! the two never overlap: an update finds a move's record and is refused as
//! `move_running`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::core_move::handover;
use crate::core_move::starter::CoreStarter;

const USAGE: &str = "usage: hided core-update [--state-dir <dir>] --previous <hided> --intent <id>";

/// How long an update may take where it runs: the old core's stop, the new
/// one's 30 s to take links, and the same again for a way back.
pub const WITHIN: std::time::Duration = std::time::Duration::from_secs(150);
/// The most an update's run may print that its caller reads.
pub const OUTPUT_CAP: usize = 64 * 1024;

/// A new update's intent id.
pub fn new_intent() -> String {
    crate::core_move::new_intent().replacen("move-", "update-", 1)
}

/// What an update's run prints, its one line on standard output. The
/// node that runs it uploaded this build, so both ends read one shape.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Outcome {
    /// The core runs this build.
    Updated {
        pid: u32,
        program: PathBuf,
    },
    /// This build did not start, and the previous build's core runs again.
    RolledBack {
        pid: u32,
        reason: String,
    },
    Refused {
        reason: String,
    },
}

/// What an update's run said: `Ok` once the core runs the new build, the
/// reason otherwise. A line that is not an outcome is named by the run's
/// exit, never read as one.
pub fn outcome(stdout: &str, exit: &str, stderr: &str) -> Result<(), String> {
    let Some(outcome) = stdout
        .lines()
        .last()
        .and_then(|line| serde_json::from_str::<Outcome>(line).ok())
    else {
        return Err(format!(
            "core-update exited {exit}: {}",
            stderr.trim().chars().take(512).collect::<String>()
        ));
    };
    match outcome {
        Outcome::Updated { .. } => Ok(()),
        Outcome::RolledBack { reason, .. } => Err(format!("rolled_back: {reason}")),
        Outcome::Refused { reason } => Err(reason),
    }
}

struct Args {
    state_dir: PathBuf,
    previous: PathBuf,
    intent: String,
}

fn parse(args: &[OsString]) -> Result<Args, String> {
    let flags =
        crate::core_move::flags::parse(args, &["--state-dir", "--previous", "--intent"], USAGE)?;
    Ok(Args {
        state_dir: flags.state_dir,
        previous: flags.previous.ok_or(USAGE)?,
        intent: flags.intent.ok_or(USAGE)?,
    })
}

pub fn run(args: &[OsString]) -> Result<(), String> {
    let args = parse(args)?;
    let outcome = update(&args).unwrap_or_else(|reason| Outcome::Refused { reason });
    herdr_core::diagnostic!(json!({
        "component": "core_update",
        "kind": "update.answered",
        "intent": args.intent,
        "answer": outcome,
    }));
    println!(
        "{}",
        serde_json::to_string(&outcome).map_err(|error| error.to_string())?
    );
    Ok(())
}

/// A build's place: `<root>/<version>/hided`, the layout the kit installs
/// and a core's login item runs.
#[derive(Debug, Eq, PartialEq)]
struct Build {
    root: PathBuf,
    version: String,
}

/// `program` on this machine as a build's place (`hide_kit::build_of`).
fn build_of(program: &Path) -> Result<Build, String> {
    let unspelled = |error| format!("{}: {error}", program.display());
    let place = hide_kit::build_of(&hide_platform::path::to_wire(program).map_err(unspelled)?)?;
    Ok(Build {
        root: hide_platform::path::from_wire(&place.root).map_err(unspelled)?,
        version: place.version,
    })
}

fn update(args: &Args) -> Result<Outcome, String> {
    let program =
        std::env::current_exe().map_err(|error| format!("this hided has no path: {error}"))?;
    let new = build_of(&program)?;
    let previous = build_of(&args.previous)?;
    if new.root != previous.root {
        return Err(format!(
            "{} and {} are not builds of one install",
            program.display(),
            args.previous.display()
        ));
    }
    let held = handover::hold(&args.state_dir)?;
    if let Some(record) = held.read()? {
        log(
            &args.intent,
            "update.refused",
            json!({"move": record.intent}),
        );
        return Err("move_running".to_owned());
    }
    let home = hide_platform::host::home_dir().map_err(|error| error.to_string())?;
    let starter = CoreStarter::for_account(&home)?;
    log(
        &args.intent,
        "update.started",
        json!({"previous": previous.version, "build": new.version}),
    );
    match starter.replace(&args.state_dir, &program) {
        Ok(pid) => {
            adopt(&new, &previous.version);
            log(&args.intent, "update.done", json!({"pid": pid}));
            Ok(Outcome::Updated { pid, program })
        }
        Err(reason) => {
            log(&args.intent, "update.failed", json!({"reason": reason}));
            // Once this machine's Herdr session moved into the new build's
            // engine, no path starts the previous build on this state folder
            // again: its Herdr would restore what moved (PRD
            // engine-bundle-4-herdr-removal D-13, D-27).
            let engine_dir = hide_kit::layout::engine_dir(&args.state_dir);
            if herdr_core::herdr_import::committed(&engine_dir) {
                log(&args.intent, "rollback.refused", json!({"reason": "herdr_moved"}));
                return Err(format!(
                    "the update failed ({reason}); this machine's Herdr session moved into the new build, so the previous build is not started again"
                ));
            }
            let pid = starter
                .replace(&args.state_dir, &args.previous)
                .map_err(|again| {
                    log(&args.intent, "rollback.failed", json!({"reason": again}));
                    format!("the update failed ({reason}) and the previous build did not start again: {again}")
                })?;
            adopt(&previous, &new.version);
            log(&args.intent, "rollback.done", json!({"pid": pid}));
            Ok(Outcome::RolledBack { pid, reason })
        }
    }
}

/// `current` leads to `running`, and the other build of the update stays.
fn adopt(running: &Build, other: &str) {
    if let Err(error) = hide_host::kit::adopt_build(&running.root, &running.version, other) {
        // The core runs either way; the kit's next pass points it again.
        herdr_core::diagnostic!(json!({
            "component": "core_update",
            "kind": "current.failed",
            "reason": error.to_string(),
        }));
    }
}

fn log(intent: &str, kind: &str, fields: Value) {
    let mut record = json!({"component": "core_update", "kind": kind, "intent": intent});
    if let (Some(record), Value::Object(fields)) = (record.as_object_mut(), fields) {
        record.extend(fields);
    }
    herdr_core::diagnostic!(record);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An update run with no state folder works on the process's default
    /// one, as a node whose placement names none runs it (B10).
    #[test]
    fn an_update_with_no_state_folder_runs_on_the_default_one() {
        let args: Vec<OsString> = ["--previous", "/x/hided", "--intent", "update-1"]
            .into_iter()
            .map(OsString::from)
            .collect();
        let args = parse(&args).unwrap();
        let home = hide_platform::host::home_dir().unwrap();
        assert_eq!(
            args.state_dir,
            hide_kit::layout::state_dir_from_process(&home)
        );
        assert_eq!(args.previous, PathBuf::from("/x/hided"));
    }

    /// The node reads only an outcome: a line of any other shape is named
    /// by the run's exit, not read as an update.
    #[test]
    fn an_update_is_read_only_from_its_outcome() {
        let line = |outcome: &Outcome| serde_json::to_string(outcome).unwrap();
        let updated = Outcome::Updated {
            pid: 7,
            program: PathBuf::from("/h/b/hided"),
        };
        assert_eq!(outcome(&line(&updated), "0", ""), Ok(()));
        assert_eq!(
            outcome(
                &line(&Outcome::RolledBack {
                    pid: 8,
                    reason: "no links".to_owned()
                }),
                "0",
                ""
            ),
            Err("rolled_back: no links".to_owned())
        );
        assert_eq!(
            outcome(r#"{"updated":{"pid":7}}"#, "0", "old"),
            Err("core-update exited 0: old".to_owned())
        );
    }
}
