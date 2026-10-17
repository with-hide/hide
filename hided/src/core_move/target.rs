//! `hided core-move <step>`: the steps of a move that run on the other
//! machine, each one SSH exec from the driver (PRD core-host-node-move B4,
//! B5). Each prints one JSON line and exits 0 once it has an answer, a
//! refusal included; a usage error exits non-zero.
//!
//! On the machine taking the core: `inspect`, `verify`, `place`, `start`,
//! `status`, `abort`, `finish` and `abandon`, and `check`, which `inspect` runs in the
//! account's GUI session (`preflight`). On the machine giving the core back to
//! the node that dialed it (`back`): `release`, `resume` and `retire`.
//! Every step that changes the state folder runs under the handover
//! record's lock (`handover`), and each can be run again with the same
//! intent and reach the same state.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use herdr_core::node_migration::copy;
use serde_json::json;

use super::answer::{Inspection, StepAnswer, StepLine};
use super::handover::{self, Handover, HandoverState};
use super::starter::CoreStarter;

struct Args {
    step: String,
    state_dir: PathBuf,
    intent: Option<String>,
    source: Option<String>,
    target: Option<String>,
    /// For `inspect` and `check`: the agents Hide AI asks, as
    /// `[[provider, model]]`.
    ai: Option<String>,
    /// For `check`: the file it answers in.
    answer: Option<PathBuf>,
}

const USAGE: &str = "usage: hided core-move <inspect|check|verify|place|start|status|abort|finish|abandon|release|resume|retire> [--state-dir <dir>] [--intent <id>] [--source <node>] [--target <node>] [--ai <json>] [--answer <file>]";

/// How long a released core may take to end once it answered.
const RELEASED_WITHIN: std::time::Duration = std::time::Duration::from_secs(30);

fn parse(args: &[OsString]) -> Result<Args, String> {
    let (step, rest) = args.split_first().ok_or(USAGE)?;
    let step = step.to_str().ok_or(USAGE)?.to_owned();
    let flags = super::flags::parse(
        rest,
        &[
            "--state-dir",
            "--intent",
            "--source",
            "--target",
            "--ai",
            "--answer",
        ],
        USAGE,
    )?;
    Ok(Args {
        step,
        state_dir: flags.state_dir,
        intent: flags.intent,
        source: flags.source,
        target: flags.target,
        ai: flags.ai,
        answer: flags.answer,
    })
}

pub fn run(args: &[OsString]) -> Result<(), String> {
    let args = parse(args)?;
    let answered = match args.step.as_str() {
        "inspect" => inspect(&args.state_dir, args.ai.as_deref()),
        "check" => check(args.ai.as_deref(), args.answer.as_deref().ok_or(USAGE)?),
        step => {
            let intent = args.intent.as_deref().ok_or(USAGE)?;
            match step {
                "verify" => verify(
                    &args.state_dir,
                    intent,
                    args.target.as_deref().ok_or(USAGE)?,
                ),
                "place" => place(
                    &args.state_dir,
                    intent,
                    args.source.as_deref().ok_or(USAGE)?,
                    args.target.as_deref().ok_or(USAGE)?,
                ),
                "start" => start(&args.state_dir, intent),
                "status" => status(&args.state_dir, intent),
                "abort" => abort(&args.state_dir, intent, account_starter),
                "finish" => finish(&args.state_dir, intent),
                "abandon" => abandon(&args.state_dir, intent, account_starter),
                "release" => release(
                    &args.state_dir,
                    intent,
                    args.target.as_deref().ok_or(USAGE)?,
                ),
                "resume" => resume(&args.state_dir, intent),
                "retire" => ai_settings().and_then(|settings| {
                    retire(&args.state_dir, intent, account_starter, &settings)
                }),
                _ => return Err(USAGE.to_owned()),
            }
        }
    };
    let line = StepLine {
        build: crate::build_id::of_current_exe()?,
        answer: answered.unwrap_or_else(|refused| refused),
    };
    herdr_core::diagnostic!(json!({
        "component": "core_move",
        "kind": "target.step",
        "step": args.step,
        "intent": args.intent,
        "answer": line.answer,
    }));
    println!(
        "{}",
        serde_json::to_string(&line).map_err(|error| error.to_string())?
    );
    Ok(())
}

/// A step's answer, or the refusal or busy answer it stopped with.
type Answered = Result<StepAnswer, StepAnswer>;

pub(crate) fn incoming(state_dir: &Path, intent: &str) -> PathBuf {
    hide_kit::layout::move_incoming(state_dir).join(intent)
}

pub(super) fn manifest_path(state_dir: &Path, intent: &str) -> PathBuf {
    hide_kit::layout::move_incoming(state_dir).join(format!("{intent}.manifest.json"))
}

/// Where this account keeps Hide AI's settings, which move with the core.
fn ai_settings() -> Result<PathBuf, StepAnswer> {
    hide_platform::host::home_dir()
        .map(|home| herdr_core::hide_ai_settings_path(&home))
        .map_err(|error| plain(error.to_string()))
}

fn refusal(error: herdr_core::node_migration::Refusal) -> StepAnswer {
    StepAnswer::Refused {
        file: Some(error.file.display().to_string()),
        reason: error.reason,
    }
}

fn plain(reason: impl Into<String>) -> StepAnswer {
    StepAnswer::refused(reason)
}

fn busy(reason: impl Into<String>) -> StepAnswer {
    StepAnswer::Busy {
        reason: reason.into(),
    }
}

/// The handover record's lock; one another change holds is busy.
fn hold(state_dir: &Path) -> Result<handover::Held, StepAnswer> {
    handover::hold(state_dir).map_err(|error| match error {
        handover::HoldError::Busy(reason) => busy(reason),
        handover::HoldError::Failed(reason) => plain(reason),
    })
}

/// This machine's node, state folder, what brain state the folder holds,
/// the move touching it, its Hide AI settings, and each of the move's
/// checks that fails here (`preflight`), Hide AI's agents in `ai` among
/// them.
fn inspect(state_dir: &Path, ai: Option<&str>) -> Answered {
    let node = herdr_core::node::NodeId::of_this_machine().map_err(plain)?;
    let handover = handover::read(state_dir).map_err(plain)?;
    // The Herdr this machine's core would own, as its daemon resolves it.
    let env = crate::env::load()
        .map_err(|errors| plain(format!("{} environment errors", errors.len())))?;
    let failed = super::preflight::failing(
        &env.home,
        state_dir,
        &super::preflight::Herdr {
            bin: env.herdr_bin_path.as_deref(),
            socket: env.herdr_socket_path.as_deref(),
        },
        ai,
        &super::preflight::Programs::for_this_machine().map_err(plain)?,
    );
    Ok(StepAnswer::Inspected(Box::new(Inspection {
        node: node.as_str().to_owned(),
        state_dir: hide_platform::path::to_wire(state_dir)
            .map_err(|error| plain(error.to_string()))?,
        brain: copy::brain_present(state_dir),
        handover,
        herdr_socket: env.herdr_socket_path,
        ai: herdr_core::stored_hide_ai_settings(&env.home),
        failed,
        path_refused: path_refused(state_dir),
    })))
}

/// Why the move's copy cannot be sent to `state_dir` by its path: the copy
/// goes over SFTP, which makes each folder on the way that is missing and
/// refuses one that is a link or not a folder, so such a folder would fail
/// the copy only after the source's core stopped. Names the first, from the
/// root down; a folder that is missing below it is made by the copy. A state
/// folder another account owns is named too: the moved core could not make
/// it private, and would not start on it.
fn path_refused(state_dir: &Path) -> Option<String> {
    let mut folders: Vec<&Path> = state_dir.ancestors().collect();
    folders.reverse();
    for folder in folders {
        match std::fs::symlink_metadata(folder) {
            Ok(found) if found.file_type().is_symlink() => {
                return Some(format!(
                    "{} is a link, and the move's copy is sent by a path without one",
                    folder.display()
                ));
            }
            Ok(found) if !found.is_dir() => {
                return Some(format!("{} is not a folder", folder.display()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(error) => return Some(format!("{} could not be read: {error}", folder.display())),
        }
    }
    match hide_platform::fs::private::owned_by_current_user(state_dir) {
        Ok(true) => None,
        Ok(false) => Some(format!(
            "{} belongs to another account, so the moved core could not keep it private",
            state_dir.display()
        )),
        Err(error) => Some(format!(
            "{} could not be read: {error}",
            state_dir.display()
        )),
    }
}

/// The checks of the account's logins, run as a one-shot job in its GUI
/// session (`preflight`): answered in `answer` as well as on standard
/// output, since a job's output goes to its log.
fn check(ai: Option<&str>, answer: &Path) -> Answered {
    let env = crate::env::load()
        .map_err(|errors| plain(format!("{} environment errors", errors.len())))?;
    let failed = super::preflight::logins_failing(
        &env.home,
        ai,
        &super::preflight::Programs::for_this_machine().map_err(plain)?,
    );
    let checked = StepAnswer::Checked { failed };
    let line = StepLine {
        build: crate::build_id::of_current_exe().map_err(plain)?,
        answer: checked.clone(),
    };
    let bytes = serde_json::to_vec(&line).map_err(|error| plain(error.to_string()))?;
    hide_platform::fs::atomic::write_file(answer, &bytes, hide_platform::fs::Access::Private)
        .map_err(|error| plain(format!("{}: {error}", answer.display())))?;
    Ok(checked)
}

/// Refuses unless this machine is `target`, the node the move's copy was
/// made for.
fn this_node_is(target: &str) -> Result<(), StepAnswer> {
    let node = herdr_core::node::NodeId::of_this_machine().map_err(plain)?;
    if node.as_str() != target {
        return Err(plain(format!(
            "the move is for node {target}, and this machine is node {node}"
        )));
    }
    Ok(())
}

/// Compares the received copy with the manifest sent beside it; a copy
/// that matches, made for this machine, is loaded with this build's
/// readers.
fn verify(state_dir: &Path, intent: &str, target: &str) -> Answered {
    this_node_is(target)?;
    let path = manifest_path(state_dir, intent);
    let bytes =
        std::fs::read(&path).map_err(|error| plain(format!("{}: {error}", path.display())))?;
    let manifest: copy::Manifest = serde_json::from_slice(&bytes)
        .map_err(|error| plain(format!("{}: {error}", path.display())))?;
    let dir = incoming(state_dir, intent);
    let received = if dir.exists() {
        copy::digest(&dir).map_err(refusal)?
    } else {
        copy::Manifest::default()
    };
    let differs = manifest.differs(&received);
    let extra: Vec<String> = received
        .files
        .into_keys()
        .filter(|path| !manifest.files.contains_key(path))
        .collect();
    if !differs.is_empty() || !extra.is_empty() {
        return Ok(StepAnswer::Differs { differs, extra });
    }
    copy::owned_by(&dir, target).map_err(refusal)?;
    copy::check_loadable(&dir).map_err(refusal)?;
    Ok(StepAnswer::Loadable)
}

/// Places the received copy, under a pending handover for `intent`.
fn place(state_dir: &Path, intent: &str, source: &str, target: &str) -> Answered {
    this_node_is(target)?;
    copy::owned_by(&incoming(state_dir, intent), target).map_err(refusal)?;
    let held = hold(state_dir)?;
    match held.read().map_err(plain)? {
        // A core this machine retired for a move back holds nothing.
        Some(record) if record.state == HandoverState::Retired => {}
        Some(record) if record.intent != intent => {
            return Err(plain(format!(
                "another move ({}) holds this machine",
                record.intent
            )));
        }
        Some(record) => {
            // Placed already, by an earlier run of this step.
            return Ok(StepAnswer::Placed {
                placed: Vec::new(),
                handover: record,
            });
        }
        None => {}
    }
    let record = Handover::new(intent, source, target, HandoverState::pending_from_now());
    held.write(&record).map_err(plain)?;
    let placed = copy::place(&incoming(state_dir, intent), state_dir, &ai_settings()?);
    match placed {
        Ok(placed) => Ok(StepAnswer::Placed {
            placed,
            handover: record,
        }),
        Err(not_placed) => {
            // What the folder holds now is its own, unless part of the copy
            // could not be taken back: that stays pending, for the driver's
            // abort to take back.
            if not_placed.left.is_empty() {
                held.remove().map_err(plain)?;
            }
            Err(refusal(not_placed.refusal))
        }
    }
}

/// Starts the pending core on the placed copy; its lease runs from the
/// place, which the driver sends just before.
fn start(state_dir: &Path, intent: &str) -> Answered {
    let lease_until_unix_ms = match handover::read(state_dir).map_err(plain)? {
        Some(Handover {
            intent: placed,
            state: HandoverState::Pending {
                lease_until_unix_ms,
            },
            ..
        }) if placed == intent => lease_until_unix_ms,
        _ => return Err(plain("no copy of this move is placed here")),
    };
    let Some(within) = time_to_start(lease_until_unix_ms, handover::now_unix_ms()) else {
        return Err(plain("the move's lease leaves its core no time to start"));
    };
    let home = hide_platform::host::home_dir().map_err(|error| plain(error.to_string()))?;
    let starter = CoreStarter::for_account(&home).map_err(plain)?;
    let program = super::starter::this_program().map_err(plain)?;
    let pid = starter
        .start_within(state_dir, &program, within)
        .map_err(plain)?;
    Ok(StepAnswer::Started {
        pid,
        lease_left_ms: lease_until_unix_ms.saturating_sub(handover::now_unix_ms()),
    })
}

/// How long a start may take so that the core it started still has the
/// full pending lease for its link; none when the lease leaves no time.
fn time_to_start(lease_until_unix_ms: u64, now_unix_ms: u64) -> Option<std::time::Duration> {
    let within = std::time::Duration::from_millis(lease_until_unix_ms.saturating_sub(now_unix_ms))
        .saturating_sub(handover::PENDING_LEASE);
    (!within.is_zero()).then_some(within)
}

fn status(state_dir: &Path, intent: &str) -> Answered {
    let record = handover::read(state_dir).map_err(plain)?;
    Ok(match record {
        Some(record) if record.intent == intent => StepAnswer::Status {
            handover: Some(record),
        },
        Some(record) => StepAnswer::OtherMove {
            intent: record.intent,
        },
        None => StepAnswer::Status { handover: None },
    })
}

/// This account's starter of a core (`CoreStarter::for_account`).
fn account_starter() -> Result<CoreStarter, String> {
    let home = hide_platform::host::home_dir().map_err(|error| error.to_string())?;
    CoreStarter::for_account(&home)
}

/// Stops a pending core and returns its copy to `move-incoming`; an active
/// core is the move committed and is left running. A core that cannot be
/// confirmed stopped is busy: nothing of it is taken back while it may run.
/// With no record left (the pending core gave its copy back at its lease's
/// end) the starter is removed all the same, so no login starts a core on
/// the folder the move left; a core running there is no core of the move
/// and is left alone.
fn abort(
    state_dir: &Path,
    intent: &str,
    starter: impl FnOnce() -> Result<CoreStarter, String>,
) -> Answered {
    let held = hold(state_dir)?;
    match held.read().map_err(plain)? {
        None => {
            starter()
                .and_then(|starter| starter.remove(state_dir))
                .map_err(busy)?;
            return Ok(StepAnswer::Aborted);
        }
        Some(record) if record.intent != intent => {
            return Ok(StepAnswer::OtherMove {
                intent: record.intent,
            });
        }
        Some(record) if record.state == HandoverState::Active => {
            return Ok(StepAnswer::Active);
        }
        Some(_) => {}
    }
    starter()
        .and_then(|starter| starter.stop(state_dir))
        .map_err(busy)?;
    // A core still starting holds no record yet, but it holds the instance
    // lock; holding it here keeps one from starting on the copy while it is
    // taken back.
    let _instance = acquire_instance(state_dir)?;
    if let Err(refused) = copy::unplace(state_dir, &incoming(state_dir, intent), &ai_settings()?) {
        return Ok(StepAnswer::StoppedNotReturned {
            reason: refused.to_string(),
        });
    }
    held.remove().map_err(plain)?;
    Ok(StepAnswer::Aborted)
}

/// Ends the move's records once it committed: the handover and the copy it
/// was placed from.
fn finish(state_dir: &Path, intent: &str) -> Answered {
    let held = hold(state_dir)?;
    match held.read().map_err(plain)? {
        Some(record) if record.intent == intent && record.state == HandoverState::Active => {
            held.remove().map_err(plain)?;
        }
        Some(record) if record.intent == intent => {
            return Err(plain("the move has not committed here"));
        }
        _ => {}
    }
    remove_copy(state_dir, intent)?;
    Ok(StepAnswer::Finished)
}

/// Ends what a move its driver gave up on left here (PRD core-host-node-move
/// amendment 4): a pending or active core of `intent` is stopped, what it
/// ran on is set aside under `intent`, and its starter, handover and copy
/// go, so this machine is back in the role it had before the move and no
/// link carrying `intent` is taken again. A core is stopped only by its
/// proven pid (`CoreStarter::stop`); one that cannot be confirmed stopped
/// is busy, and nothing of it is set aside while it may run. Another
/// move's record is left alone.
fn abandon(
    state_dir: &Path,
    intent: &str,
    starter: impl FnOnce() -> Result<CoreStarter, String>,
) -> Answered {
    let held = hold(state_dir)?;
    match held.read().map_err(plain)? {
        Some(record) if record.intent != intent => {
            return Ok(StepAnswer::OtherMove {
                intent: record.intent,
            });
        }
        Some(record)
            if matches!(
                record.state,
                HandoverState::Pending { .. } | HandoverState::Active
            ) =>
        {
            starter()
                .and_then(|starter| starter.stop(state_dir))
                .map_err(busy)?;
            // No core starts on the folder while what it ran on is set
            // aside.
            let _instance = acquire_instance(state_dir)?;
            copy::set_aside(state_dir, intent, &ai_settings()?).map_err(refusal)?;
            held.remove().map_err(plain)?;
        }
        Some(_) => return Err(plain("this machine gave its core back in this move")),
        // The pending core gave its copy back at its lease's end, or none
        // was placed: no core of the move runs, and no login starts one.
        None => {
            starter()
                .and_then(|starter| starter.remove(state_dir))
                .map_err(busy)?;
        }
    }
    remove_copy(state_dir, intent)?;
    Ok(StepAnswer::Abandoned)
}

/// Removes the copy `intent` was placed from and its manifest.
fn remove_copy(state_dir: &Path, intent: &str) -> Result<(), StepAnswer> {
    for path in [
        incoming(state_dir, intent),
        manifest_path(state_dir, intent),
    ] {
        let removed = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        match removed {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(plain(format!("{}: {error}", path.display()))),
        }
    }
    Ok(())
}

/// Has this machine's core stop for the move back `intent` to `target` and
/// stages its copy: the core checks the move, records the stop and its
/// export, answers, and ends; the copy is made once it has ended, under the
/// instance lock so no core starts on the folder meanwhile. The record's
/// lock is held for the decision and from the recorded stop to the staged
/// copy, so no other change of the record (an abandon, an update) runs
/// between them; it is let go only while the core records its stop, which
/// takes the same lock.
fn release(state_dir: &Path, intent: &str, target: &str) -> Answered {
    let held = hold(state_dir)?;
    // Held to the staged copy.
    let _held = match held.read().map_err(plain)? {
        Some(record) if record.intent == intent && record.state == HandoverState::Retired => {
            return Ok(StepAnswer::Retired { leftover: None });
        }
        // Stopped already, by an earlier run of this step.
        Some(record) if record.intent == intent && record.state == HandoverState::StoppedFor => {
            held
        }
        Some(record) if record.state != HandoverState::Retired => {
            return Err(plain(format!(
                "another move ({}) holds this machine",
                record.intent
            )));
        }
        _ => {
            let running = crate::state_file::read_state(state_dir)
                .map_err(|error| plain(error.to_string()))?
                .ok_or_else(|| plain("no core runs here"))?;
            drop(held);
            if let Err(reason) = crate::attach::release(state_dir, intent, target) {
                // The answer can be lost as the core ends; its record says
                // whether it stopped.
                let stopped = handover::read(state_dir)
                    .ok()
                    .flatten()
                    .is_some_and(|record| {
                        record.intent == intent && record.state == HandoverState::StoppedFor
                    });
                if !stopped {
                    return Err(plain(reason));
                }
            }
            wait_until_gone(&running)?;
            let held = hold(state_dir)?;
            match held.read().map_err(plain)? {
                Some(record)
                    if record.intent == intent && record.state == HandoverState::StoppedFor =>
                {
                    held
                }
                _ => {
                    return Err(plain(
                        "the core's stop for this move is no longer recorded here",
                    ));
                }
            }
        }
    };
    let _instance = acquire_instance(state_dir)?;
    let export = super::back::read_export(state_dir, intent).map_err(plain)?;
    let staging = herdr_core::node_migration::staging_dir(state_dir, intent);
    copy::stage(state_dir, &staging, &ai_settings()?).map_err(refusal)?;
    super::driver::carry_labels(&staging, target, &export.labels).map_err(plain)?;
    let manifest = copy::digest(&staging).map_err(refusal)?;
    let bytes = serde_json::to_vec(&manifest).map_err(|error| plain(error.to_string()))?;
    let path = super::back::manifest_path(state_dir, intent);
    hide_platform::fs::atomic::write_file_durable(
        &path,
        &bytes,
        hide_platform::fs::Access::Private,
    )
    .map_err(|error| plain(format!("{}: {error}", path.display())))?;
    Ok(StepAnswer::Staged {
        files: manifest.files.len(),
    })
}

/// Undoes a release: the staged copy goes and this machine's core starts on
/// its folder again. A core retired for the move stays retired.
fn resume(state_dir: &Path, intent: &str) -> Answered {
    let held = hold(state_dir)?;
    let mut leftover = None;
    match held.read().map_err(plain)? {
        Some(record) if record.intent == intent && record.state == HandoverState::Retired => {
            return Ok(StepAnswer::Retired { leftover: None });
        }
        Some(record) if record.intent == intent && record.state == HandoverState::StoppedFor => {
            // The core the record says stopped may still be ending: it exits
            // successfully, which nothing starts again, so it is waited for
            // rather than read as running.
            if let Some(stopping) = crate::state_file::read_state(state_dir)
                .map_err(|error| plain(error.to_string()))?
            {
                wait_until_gone(&stopping)?;
            }
            // A staged copy that cannot be removed keeps no core from
            // starting: the next staging removes it first.
            leftover = super::back::remove_staging(state_dir, intent).err();
            held.remove().map_err(plain)?;
        }
        Some(record) if record.state != HandoverState::Retired => {
            return Err(plain(format!(
                "another move ({}) holds this machine",
                record.intent
            )));
        }
        // The release never stopped the core, or an earlier resume ran.
        _ => {}
    }
    drop(held);
    if let Ok(Some(running)) = crate::state_file::read_state(state_dir)
        && running.is_proven_running()
    {
        return Ok(StepAnswer::Running {
            pid: running.pid,
            leftover,
        });
    }
    let home = hide_platform::host::home_dir().map_err(|error| plain(error.to_string()))?;
    let pid = CoreStarter::for_account(&home)
        .and_then(|starter| starter.start(state_dir, &super::starter::this_program()?))
        .map_err(plain)?;
    Ok(StepAnswer::Running { pid, leftover })
}

/// The commit of a move back: this machine's starter is removed, so no
/// core of its starts again, and its brain state is set aside. Once the
/// record says retired the move has committed, so what of the folder cannot
/// be set aside or removed is answered as left over, never as a refusal a
/// rollback would undo.
fn retire(
    state_dir: &Path,
    intent: &str,
    starter: impl FnOnce() -> Result<CoreStarter, String>,
    ai_settings: &Path,
) -> Answered {
    let held = hold(state_dir)?;
    match held.read().map_err(plain)? {
        Some(record) if record.intent == intent && record.state == HandoverState::Retired => {}
        Some(mut record)
            if record.intent == intent && record.state == HandoverState::StoppedFor =>
        {
            starter()
                .and_then(|starter| starter.stop(state_dir))
                .map_err(busy)?;
            let _instance = acquire_instance(state_dir)?;
            record.state = HandoverState::Retired;
            held.write(&record).map_err(plain)?;
        }
        _ => return Err(plain("this machine's core is not stopped for this move")),
    }
    let set_aside = copy::set_aside(state_dir, intent, ai_settings)
        .map_err(|refusal| format!("{}: {}", refusal.file.display(), refusal.reason))
        .err();
    let staging = super::back::remove_staging(state_dir, intent).err();
    let leftover = match (set_aside, staging) {
        (None, None) => None,
        (Some(one), None) | (None, Some(one)) => Some(one),
        (Some(set_aside), Some(staging)) => Some(format!("{set_aside}; {staging}")),
    };
    Ok(StepAnswer::Retired { leftover })
}

/// Waits until the core `running` names has ended; one that has not is
/// busy, since it may still end. A pid another process reused is gone.
#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn wait_until_gone(running: &crate::state_file::DaemonState) -> Result<(), StepAnswer> {
    let deadline = std::time::Instant::now() + RELEASED_WITHIN;
    while running.is_proven_running() {
        if std::time::Instant::now() >= deadline {
            return Err(busy(format!(
                "the core {} did not end after its release",
                running.pid
            )));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Ok(())
}

/// The folder's instance lock, which no core holds once this has it; a
/// core that still holds it is busy.
fn acquire_instance(state_dir: &Path) -> Result<hide_platform::fs::lock::Lock, StepAnswer> {
    crate::state_file::acquire_lock(state_dir).map_err(|error| {
        busy(format!(
            "a core of this folder is still starting or running: {error}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state folder reached through a link, as macOS's `/tmp` is, is
    /// named with the link, which the copy would refuse after the core
    /// stopped; one reached by real folders, missing or not, is not.
    #[cfg(unix)]
    #[test]
    fn a_state_folder_reached_through_a_link_is_named_at_the_check() {
        let root = tempfile::tempdir().unwrap();
        let root = hide_platform::fs::identity::canonical(root.path()).unwrap();
        let real = root.join("real");
        std::fs::create_dir_all(real.join("home/s")).unwrap();
        std::os::unix::fs::symlink(&real, root.join("link")).unwrap();
        std::fs::write(root.join("file"), "").unwrap();

        assert_eq!(path_refused(&real.join("home/s")), None);
        assert_eq!(path_refused(&real.join("home/missing/s")), None);
        let through_link = path_refused(&root.join("link/home/s")).unwrap();
        assert!(
            through_link.starts_with(&format!("{} is a link", root.join("link").display())),
            "{through_link}"
        );
        let through_file = path_refused(&root.join("file/s")).unwrap();
        assert_eq!(
            through_file,
            format!("{} is not a folder", root.join("file").display())
        );
    }

    /// A move whose pending core already gave its copy back leaves no
    /// starter: the abort that finds no record removes the folder's login
    /// item all the same.
    #[cfg(unix)]
    #[test]
    fn an_abort_with_no_record_left_removes_the_login_item() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let state = home.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        // `print` answers loaded until a bootout.
        let launchctl = home.path().join("launchctl");
        std::fs::write(
            &launchctl,
            "#!/bin/sh\ncase \"$1\" in\n  print) [ -f \"$HOME/loaded\" ] && exit 0; exit 113 ;;\n  bootout) rm -f \"$HOME/loaded\" ;;\nesac\nexit 0\n",
        )
        .unwrap();
        std::fs::set_permissions(&launchctl, std::fs::Permissions::from_mode(0o755)).unwrap();
        let agents = hide_platform::user_agents::UserAgents::fixture(home.path());
        let label = hide_kit::layout::core_login_item(home.path(), &state);
        agents
            .install(
                &hide_platform::user_agents::LoginAgent {
                    label: &label,
                    program: Path::new("/usr/bin/true"),
                    arguments: &[],
                    environment: &[],
                    log: &home.path().join("core.log"),
                },
                home.path(),
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap();
        std::fs::write(home.path().join("loaded"), "").unwrap();
        let starter = CoreStarter::LoginItem {
            home: home.path().to_path_buf(),
            agents: agents.clone(),
        };
        assert_eq!(abort(&state, "i1", || Ok(starter)), Ok(StepAnswer::Aborted));
        assert!(
            !agents.is_installed(&label, home.path()).unwrap(),
            "the login item is still installed"
        );
        assert!(
            !home.path().join("loaded").exists(),
            "the login item is still loaded"
        );
    }

    /// The retirement is the commit once its record says retired: a folder
    /// it then cannot set aside is answered as left over, never as a refusal
    /// a rollback would resume a retired core on, and the next retirement
    /// sets aside what is left without stopping anything again.
    #[test]
    fn a_retirement_that_cannot_set_the_folder_aside_still_answers_retired() {
        use herdr_core::node_migration::MARKER_FILE;
        let home = tempfile::tempdir().unwrap();
        let state = home.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let settings = home.path().join("account/hide/ai.json");
        std::fs::write(state.join(MARKER_FILE), "{}").unwrap();
        handover::hold(&state)
            .unwrap()
            .write(&handover::Handover::new(
                "i1",
                "mini",
                "mbp",
                HandoverState::StoppedFor,
            ))
            .unwrap();
        // A file where the folder it sets aside into goes.
        let moved_out = hide_kit::layout::moved_out(&state);
        std::fs::write(&moved_out, "").unwrap();

        let answered = retire(&state, "i1", || Ok(CoreStarter::Fixture), &settings);
        let Ok(StepAnswer::Retired {
            leftover: Some(leftover),
        }) = &answered
        else {
            panic!("{answered:?}");
        };
        assert!(leftover.contains("moved-out"), "{leftover}");
        assert_eq!(
            handover::read(&state).unwrap().unwrap().state,
            HandoverState::Retired
        );

        std::fs::remove_file(&moved_out).unwrap();
        let again = retire(
            &state,
            "i1",
            || -> Result<CoreStarter, String> { panic!("a retired core was stopped again") },
            &settings,
        );
        assert_eq!(again, Ok(StepAnswer::Retired { leftover: None }));
        assert!(moved_out.join("i1").join(MARKER_FILE).exists());
        assert!(!state.join(MARKER_FILE).exists());
    }

    /// A start is given only what the lease leaves beyond a full pending
    /// lease, so a core that started always has that lease for its link.
    #[test]
    fn a_start_is_given_only_what_leaves_a_full_lease() {
        let lease = handover::PENDING_LEASE.as_millis() as u64;
        assert_eq!(
            time_to_start(10_000 + lease + 2_500, 10_000),
            Some(std::time::Duration::from_millis(2_500))
        );
        assert_eq!(time_to_start(10_000 + lease, 10_000), None);
        assert_eq!(time_to_start(10_000, 20_000), None);
    }
}
