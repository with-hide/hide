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
//! engine (PRD engine-bundle-4-herdr-removal D-27), which only goes forward:
//! the update answers why, and the new build stays the one the account
//! starts, so a retry starts it again (B40). All three ends leave `current`
//! at the build that runs or is to run and keep the other build of
//! the update, so a later connection reuses its files and no more than two
//! builds are kept. An update holds the folder's lock
//! (`hide_kit::layout::core_update_lock`) for its whole length, so two never
//! overlap.
//!
//! Run with no `--previous` it raises the core this account already runs
//! (PRD device-connect-iroh B23): a paired machine that started this
//! account's Hide over SSH or `wsl.exe` put this build here and runs it, and
//! what the folder's core is decides the rest, under the same lock, so the
//! raises of two machines happen one after the other and the second finds
//! what the first left. A core of this build or a newer one is left as it
//! is; an older one is replaced as above, with this build put beside the
//! one its starter runs. A core a desktop app opened is left to that app
//! (`hide_kit::app`): the app updates it, and a core raised past the app's
//! build would refuse the app's own connection.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hide_platform::fs::lock::{Lock, Mode, Waited};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::build_order::{Release, Standing};
use crate::core_install::starter::CoreStarter;

const USAGE: &str =
    "usage: hided core-update [--state-dir <dir>] [--previous <hided>] --intent <id>";

/// How long an update may take where it runs: the old core's stop, the new
/// one's 30 s to take links, and the same again for a way back.
pub const WITHIN: std::time::Duration = std::time::Duration::from_secs(150);
/// The most an update's run may print that its caller reads.
pub const OUTPUT_CAP: usize = 64 * 1024;

/// A new update's intent id.
pub fn new_intent() -> String {
    crate::core_install::new_intent("update")
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
    /// A raise found the core running this build already.
    Current,
    /// A raise found the core running a newer build, which is never
    /// lowered.
    Kept,
    /// A raise found that a desktop app opened the core, which updates it
    /// from that day on (B23).
    LeftToApp,
}

/// A raise's refusals a caller tells apart from every other reason.
/// Another update holds the folder now.
pub const BUSY: &str = "busy";
/// No core runs on the folder.
pub const NOT_RUNNING: &str = "core_not_running";
/// Nothing on the account starts the folder's core again, so replacing it
/// could leave none.
pub const NOT_KEPT: &str = "core_not_kept";
/// The core's build and this one are not ordered.
pub const OTHER_BUILD: &str = "other_build";

/// What an update's run said: `Ok` once the core runs the new build, the
/// reason otherwise. A line that is not an outcome is named by the run's
/// exit, never read as one.
pub fn outcome(stdout: &str, exit: &str, stderr: &str) -> Result<(), String> {
    let Some(outcome) = printed(stdout) else {
        return Err(format!(
            "core-update exited {exit}: {}",
            stderr.trim().chars().take(512).collect::<String>()
        ));
    };
    match outcome {
        Outcome::Updated { .. } | Outcome::Current => Ok(()),
        Outcome::RolledBack { reason, .. } => Err(format!("rolled_back: {reason}")),
        Outcome::Refused { reason } => Err(reason),
        Outcome::Kept => Err("core_newer".to_owned()),
        Outcome::LeftToApp => Err("left_to_app".to_owned()),
    }
}

/// The outcome a run printed, when its last line is one.
pub fn printed(stdout: &str) -> Option<Outcome> {
    stdout
        .lines()
        .last()
        .and_then(|line| serde_json::from_str(line).ok())
}

struct Args {
    state_dir: PathBuf,
    /// The build the core runs, named by the node that updates it; none
    /// for a raise, which reads it from the core.
    previous: Option<PathBuf>,
    intent: String,
}

fn parse(args: &[OsString]) -> Result<Args, String> {
    let flags =
        crate::core_install::flags::parse(args, &["--state-dir", "--previous", "--intent"], USAGE)?;
    Ok(Args {
        state_dir: flags.state_dir,
        previous: flags.previous,
        intent: flags.intent.ok_or(USAGE)?,
    })
}

pub fn run(args: &[OsString]) -> Result<(), String> {
    let args = parse(args)?;
    let outcome = match &args.previous {
        Some(previous) => update(&args, previous),
        None => raise(&args),
    }
    .unwrap_or_else(|reason| Outcome::Refused { reason });
    hide_core::diagnostic!(json!({
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

/// How long an update waits for the folder's lock.
const LOCK_WAIT: Duration = Duration::from_secs(5);

/// The folder's lock, held for one update's whole length.
struct Held {
    _lock: Lock,
}

/// Why the folder's lock was not taken.
#[derive(Debug)]
enum HoldError {
    /// Another update held it past the wait: a later try may take it.
    Busy(String),
    Failed(String),
}

impl From<HoldError> for String {
    fn from(error: HoldError) -> Self {
        match error {
            HoldError::Busy(reason) | HoldError::Failed(reason) => reason,
        }
    }
}

/// Takes the folder's lock, waiting a few seconds for another update to
/// finish.
fn hold(state_dir: &Path) -> Result<Held, HoldError> {
    hold_within(state_dir, LOCK_WAIT)
}

fn hold_within(state_dir: &Path, wait: Duration) -> Result<Held, HoldError> {
    let lock_path = hide_kit::layout::core_update_lock(state_dir);
    let failed =
        |error: std::io::Error| HoldError::Failed(format!("{}: {error}", lock_path.display()));
    let file = hide_platform::fs::private::open_or_create_file(&lock_path).map_err(failed)?;
    match hide_platform::fs::lock::lock_file(file, Mode::Exclusive, wait, &|| false)
        .map_err(failed)?
    {
        Waited::Locked(lock) => Ok(Held { _lock: lock }),
        Waited::TimedOut | Waited::Cancelled => Err(HoldError::Busy(format!(
            "{} is held by another update",
            lock_path.display()
        ))),
    }
}

/// What starts a build's core and answers its pid once it takes links.
trait Starter {
    fn replace(&self, state_dir: &Path, program: &Path) -> Result<u32, String>;
}

impl Starter for CoreStarter {
    fn replace(&self, state_dir: &Path, program: &Path) -> Result<u32, String> {
        CoreStarter::replace(self, state_dir, program)
    }
}

fn update(args: &Args, previous: &Path) -> Result<Outcome, String> {
    let program =
        std::env::current_exe().map_err(|error| format!("this hided has no path: {error}"))?;
    update_with(args, previous, &program, || {
        let home = hide_platform::host::home_dir().map_err(|error| error.to_string())?;
        CoreStarter::for_account(&home)
    })
}

/// The update of the core to `program`'s build, started by the starter
/// `starter` makes once nothing else holds the state folder.
fn update_with<S: Starter>(
    args: &Args,
    previous: &Path,
    program: &Path,
    starter: impl FnOnce() -> Result<S, String>,
) -> Result<Outcome, String> {
    let _held = hold(&args.state_dir)?;
    let builds = replaceable(previous, program)?;
    replace(args, &builds, previous, program, &starter()?)
}

/// What stands in the way of replacing the core, read before anything is
/// stopped or started: the two programs are builds of one install. The two
/// builds when nothing does.
fn replaceable(previous_program: &Path, program: &Path) -> Result<(Build, Build), String> {
    let new = build_of(program)?;
    let previous = build_of(previous_program)?;
    if new.root != previous.root {
        return Err(format!(
            "{} and {} are not builds of one install",
            program.display(),
            previous_program.display()
        ));
    }
    Ok((new, previous))
}

/// Replaces the core by `program`'s build, with the way back to
/// `previous_program`'s; `builds` is what [`replaceable`] answered for the
/// two.
fn replace<S: Starter>(
    args: &Args,
    (new, previous): &(Build, Build),
    previous_program: &Path,
    program: &Path,
    starter: &S,
) -> Result<Outcome, String> {
    log(
        &args.intent,
        "update.started",
        json!({"previous": previous.version, "build": new.version}),
    );
    match starter.replace(&args.state_dir, program) {
        Ok(pid) => {
            adopt(new, &previous.version);
            log(&args.intent, "update.done", json!({"pid": pid}));
            Ok(Outcome::Updated {
                pid,
                program: program.to_path_buf(),
            })
        }
        Err(reason) => {
            log(&args.intent, "update.failed", json!({"reason": reason}));
            let pid = starter
                .replace(&args.state_dir, previous_program)
                .map_err(|again| {
                    log(&args.intent, "rollback.failed", json!({"reason": again}));
                    format!("the update failed ({reason}) and the previous build did not start again: {again}")
                })?;
            adopt(previous, &new.version);
            log(&args.intent, "rollback.done", json!({"pid": pid}));
            Ok(Outcome::RolledBack { pid, reason })
        }
    }
}

/// The core a state folder runs, as a raise reads it.
#[derive(Clone, Debug)]
struct Running {
    /// Its build's hash and release (`/health`).
    build: String,
    release: Release,
    /// The program its starter runs it from; none for a core no starter
    /// started, such as the one `hide connect` started before the account
    /// was paired, which runs on until it ends.
    started: Option<PathBuf>,
    /// The program it is.
    program: Option<PathBuf>,
}

/// What a raise reads and does on the account, beside its starter.
trait Account {
    /// The folder's core, when one answers.
    fn running(&self) -> Result<Option<Running>, String>;
    /// Whether a desktop app opened the folder's core and is still there
    /// (`hide_kit::app`): the app updates its core.
    fn app(&self) -> Result<bool, String>;
    /// Whether a starter of the account's starts the folder's core again.
    fn kept(&self) -> Result<bool, String>;
    /// The install root the account's starter runs its builds from.
    fn core_root(&self) -> PathBuf;
    /// The build `program` is of, put in `root`: where its `hided` is.
    fn place(&self, program: &Path, root: &Path) -> Result<PathBuf, String>;
    /// Stops a core no starter runs.
    fn stop(&self) -> Result<(), String>;
}

/// This machine's account, on `state_dir`.
struct ThisAccount<'a> {
    home: PathBuf,
    state_dir: &'a Path,
    starter: &'a CoreStarter,
}

impl Account for ThisAccount<'_> {
    fn running(&self) -> Result<Option<Running>, String> {
        let Some((_, health)) = crate::cli::healthy_daemon_in(self.state_dir) else {
            return Ok(None);
        };
        let build = health["build"]
            .as_str()
            .ok_or("the core's health names no build")?
            .to_owned();
        let release = serde_json::from_value(health["release"].clone())
            .map_err(|error| format!("the core's health names no release: {error}"))?;
        let path = |value: &Value| value.as_str().map(PathBuf::from);
        Ok(Some(Running {
            build,
            release,
            started: path(&health["starter"]["program"]),
            program: path(&health["program"]),
        }))
    }

    fn app(&self) -> Result<bool, String> {
        hide_kit::app::present(self.state_dir)
    }

    fn kept(&self) -> Result<bool, String> {
        self.starter.keeps(self.state_dir)
    }

    fn core_root(&self) -> PathBuf {
        hide_kit::layout::core_root(&self.home)
    }

    fn place(&self, program: &Path, root: &Path) -> Result<PathBuf, String> {
        crate::always_on::install_this_build(program, root)
    }

    fn stop(&self) -> Result<(), String> {
        crate::core_install::starter::stop_running(self.state_dir)
    }
}

fn raise(args: &Args) -> Result<Outcome, String> {
    let program =
        std::env::current_exe().map_err(|error| format!("this hided has no path: {error}"))?;
    let build = crate::build_id::of_file(&program)?;
    let home = hide_platform::host::home_dir().map_err(|error| error.to_string())?;
    let starter = CoreStarter::for_account(&home)?;
    let account = ThisAccount {
        home,
        state_dir: &args.state_dir,
        starter: &starter,
    };
    raise_with(
        args,
        &program,
        (&build, &Release::of_this_build()),
        &account,
        &starter,
    )
}

/// The raise of the folder's core to `program`'s build, `ours`.
fn raise_with<S: Starter>(
    args: &Args,
    program: &Path,
    ours: (&str, &Release),
    account: &dyn Account,
    starter: &S,
) -> Result<Outcome, String> {
    let _held = match hold(&args.state_dir) {
        Ok(held) => held,
        Err(HoldError::Busy(_)) => return Err(BUSY.to_owned()),
        Err(error) => return Err(error.into()),
    };
    let Some(core) = account.running()? else {
        return Err(NOT_RUNNING.to_owned());
    };
    match crate::build_order::standing(ours, (&core.build, &core.release)) {
        Standing::Newer => {}
        Standing::Same => return Ok(Outcome::Current),
        Standing::Older => {
            log(
                &args.intent,
                "raise.kept",
                json!({"core": core.release.shown()}),
            );
            return Ok(Outcome::Kept);
        }
        Standing::Unordered => return Err(OTHER_BUILD.to_owned()),
    }
    // The app that opened this core updates it (B23): a core a paired
    // machine raised past the app's build would refuse the app's own
    // connection, with no release for the app to catch up to.
    if account.app()? {
        log(
            &args.intent,
            "raise.left_to_app",
            json!({"core": core.release.shown()}),
        );
        return Ok(Outcome::LeftToApp);
    }
    // Both builds stand in the root the account's starter runs from: the
    // one the core runs is there when its starter started it, and is put
    // there from the core's own folder when nothing did.
    let previous = match &core.started {
        Some(started) => started.clone(),
        None => {
            if !account.kept()? {
                return Err(NOT_KEPT.to_owned());
            }
            let own = core.program.as_deref().ok_or(NOT_KEPT)?;
            account.place(own, &account.core_root())?
        }
    };
    let new = account.place(program, &build_of(&previous)?.root)?;
    // Every refusal comes before the stop: a raise that ends refused
    // leaves the core that ran.
    let builds = replaceable(&previous, &new)?;
    if core.started.is_none() {
        // Its starter never started it, so replacing the starter's run
        // would not end it, and the new core would wait on its lock.
        account.stop()?;
    }
    replace(args, &builds, &previous, &new, starter)
}

/// `current` leads to `running`, and the other build of the update stays.
fn adopt(running: &Build, other: &str) {
    if let Err(error) = hide_host::kit::adopt_build(&running.root, &running.version, other) {
        // The core runs either way; the kit's next pass points it again.
        hide_core::diagnostic!(json!({
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
    hide_core::diagnostic!(record);
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
        assert_eq!(args.previous, Some(PathBuf::from("/x/hided")));
        // With none it is a raise, which reads the build from the core.
        let raise: Vec<OsString> = ["--intent", "update-2"]
            .into_iter()
            .map(OsString::from)
            .collect();
        assert_eq!(parse(&raise).unwrap().previous, None);
    }

    /// Starts the builds a test names as failing with why, and records each.
    #[derive(Default)]
    struct Starts {
        failing: std::cell::RefCell<Vec<PathBuf>>,
        started: std::cell::RefCell<Vec<PathBuf>>,
    }

    impl Starter for &Starts {
        fn replace(&self, _: &Path, program: &Path) -> Result<u32, String> {
            self.started.borrow_mut().push(program.to_path_buf());
            if self
                .failing
                .borrow()
                .iter()
                .any(|failing| failing == program)
            {
                Err("the core took no links within 30 s".to_owned())
            } else {
                Ok(7)
            }
        }
    }

    /// Two builds of one install, the previous and the new, and the state
    /// folder they run on.
    fn install(root: &Path) -> (Args, PathBuf, PathBuf) {
        let build = |version: &str| {
            let program = root.join("builds").join(version).join("hided");
            std::fs::create_dir_all(program.parent().unwrap()).unwrap();
            std::fs::write(&program, "").unwrap();
            // What the root's `current` leads to (`hide_kit::current`).
            for beside in ["hide", "hide-agent-hooks"] {
                let name = format!("{beside}{}", std::env::consts::EXE_SUFFIX);
                std::fs::write(program.with_file_name(name), version).unwrap();
            }
            program
        };
        let previous = build("aaaaaaaaaaaaaaaa");
        let new = build("bbbbbbbbbbbbbbbb");
        let state_dir = root.join("state");
        std::fs::create_dir_all(&state_dir).unwrap();
        (
            Args {
                state_dir,
                previous: None,
                intent: "update-1".to_owned(),
            },
            previous,
            new,
        )
    }

    // D-27, B41: before this machine's Herdr session moved, a new build that
    // takes no links gives way to the previous build again.
    #[test]
    fn an_update_that_fails_before_the_move_starts_the_previous_build_again() {
        let root = tempfile::tempdir().unwrap();
        let (args, previous, new) = install(root.path());
        let starts = Starts::default();
        starts.failing.borrow_mut().push(new.clone());
        let outcome = update_with(&args, &previous, &new, || Ok(&starts)).unwrap();
        assert!(
            matches!(outcome, Outcome::RolledBack { pid: 7, .. }),
            "{outcome:?}"
        );
        assert_eq!(*starts.started.borrow(), [new, previous]);
    }

    /// An account a raise runs on: the core the folder runs, whether a
    /// starter keeps it, and what the raise placed and stopped.
    struct Machine {
        root: PathBuf,
        core: Option<Running>,
        app: bool,
        kept: bool,
        placed: std::cell::RefCell<Vec<(PathBuf, PathBuf)>>,
        stopped: std::cell::Cell<u32>,
    }

    impl Account for Machine {
        fn running(&self) -> Result<Option<Running>, String> {
            Ok(self.core.clone())
        }

        fn app(&self) -> Result<bool, String> {
            Ok(self.app)
        }

        fn kept(&self) -> Result<bool, String> {
            Ok(self.kept)
        }

        fn core_root(&self) -> PathBuf {
            self.root.join("builds")
        }

        /// A build keeps its version folder's name wherever it is put.
        fn place(&self, program: &Path, root: &Path) -> Result<PathBuf, String> {
            self.placed
                .borrow_mut()
                .push((program.to_path_buf(), root.to_path_buf()));
            let version = program.parent().unwrap().file_name().unwrap();
            let placed = root.join(version).join("hided");
            std::fs::create_dir_all(placed.parent().unwrap()).unwrap();
            std::fs::write(&placed, "").unwrap();
            Ok(placed)
        }

        fn stop(&self) -> Result<(), String> {
            self.stopped.set(self.stopped.get() + 1);
            Ok(())
        }
    }

    fn release(order: u64) -> Release {
        Release {
            version: "1.2.10".to_owned(),
            order,
            commit: None,
        }
    }

    /// A raise of a core of build order 5 that its starter runs from
    /// `previous`, by the build a paired machine put in its own folder.
    fn raise_fixture(root: &Path) -> (Args, Machine, PathBuf, PathBuf) {
        let (args, previous, _) = install(root);
        let uploaded = root.join("helper").join("cccccccccccccccc").join("hided");
        std::fs::create_dir_all(uploaded.parent().unwrap()).unwrap();
        std::fs::write(&uploaded, "").unwrap();
        let machine = Machine {
            root: root.to_path_buf(),
            core: Some(Running {
                build: "old".to_owned(),
                release: release(5),
                started: Some(previous.clone()),
                program: Some(previous.clone()),
            }),
            app: false,
            kept: true,
            placed: Default::default(),
            stopped: Default::default(),
        };
        (args, machine, previous, uploaded)
    }

    // B23: an older core its starter runs is replaced by the build a paired
    // machine brought, put beside the one it runs, and never stopped by
    // hand, since replacing the starter's run ends it.
    #[test]
    fn a_raise_puts_the_newer_build_beside_the_core_s_and_replaces_it() {
        let root = tempfile::tempdir().unwrap();
        let (args, machine, previous, uploaded) = raise_fixture(root.path());
        let starts = Starts::default();
        let outcome =
            raise_with(&args, &uploaded, ("new", &release(6)), &machine, &&starts).unwrap();
        let placed = previous
            .parent()
            .unwrap()
            .with_file_name("cccccccccccccccc")
            .join("hided");
        assert_eq!(
            outcome,
            Outcome::Updated {
                pid: 7,
                program: placed.clone()
            }
        );
        assert_eq!(*starts.started.borrow(), [placed]);
        assert_eq!(machine.stopped.get(), 0);
    }

    // B23: a core of this build is left alone, a newer one is never
    // lowered, and one neither orders is refused; none of them is touched.
    #[test]
    fn a_raise_leaves_a_core_that_is_not_older_as_it_is() {
        let root = tempfile::tempdir().unwrap();
        let (args, machine, _, uploaded) = raise_fixture(root.path());
        let starts = Starts::default();
        let raise = |build: &str, ours: &Release| {
            raise_with(&args, &uploaded, (build, ours), &machine, &&starts)
        };
        assert_eq!(raise("old", &release(5)), Ok(Outcome::Current));
        assert_eq!(raise("lower", &release(4)), Ok(Outcome::Kept));
        let unnumbered = Release {
            version: "local".to_owned(),
            ..release(9)
        };
        assert_eq!(raise("other", &unnumbered), Err(OTHER_BUILD.to_owned()));
        assert!(starts.started.borrow().is_empty());
        assert!(machine.placed.borrow().is_empty());
        assert_eq!(machine.stopped.get(), 0);
    }

    // B23: the core `hide connect` started before the account was paired
    // runs on under no starter. Its build is put in the starter's root for
    // the way back, it is stopped so the new core's start is not left
    // waiting on its lock, and a new build that takes no links gives way
    // to it again.
    #[test]
    fn a_raise_of_a_core_no_starter_started_stops_it_and_keeps_its_build_for_the_way_back() {
        let root = tempfile::tempdir().unwrap();
        let (args, mut machine, _, uploaded) = raise_fixture(root.path());
        let own = root
            .path()
            .join("helper")
            .join("aaaaaaaaaaaaaaaa")
            .join("hided");
        machine.core = Some(Running {
            started: None,
            program: Some(own.clone()),
            ..machine.core.clone().unwrap()
        });
        let builds = machine.core_root();
        let (kept, new) = (
            builds.join("aaaaaaaaaaaaaaaa").join("hided"),
            builds.join("cccccccccccccccc").join("hided"),
        );
        let starts = Starts::default();
        starts.failing.borrow_mut().push(new.clone());
        let outcome =
            raise_with(&args, &uploaded, ("new", &release(6)), &machine, &&starts).unwrap();
        assert!(
            matches!(outcome, Outcome::RolledBack { pid: 7, .. }),
            "{outcome:?}"
        );
        assert_eq!(
            *machine.placed.borrow(),
            [(own, builds.clone()), (uploaded, builds)]
        );
        assert_eq!(machine.stopped.get(), 1);
        assert_eq!(*starts.started.borrow(), [new, kept]);
    }

    // B23: a core nothing starts again, or none at all, is not replaced: a
    // start that failed would leave the account with no core.
    #[test]
    fn a_raise_never_stops_a_core_nothing_would_start_again() {
        let root = tempfile::tempdir().unwrap();
        let (args, mut machine, previous, uploaded) = raise_fixture(root.path());
        let starts = Starts::default();
        machine.kept = false;
        machine.core = Some(Running {
            started: None,
            ..machine.core.clone().unwrap()
        });
        let raise = |machine: &Machine| {
            raise_with(&args, &uploaded, ("new", &release(6)), machine, &&starts)
        };
        assert_eq!(raise(&machine), Err(NOT_KEPT.to_owned()));
        machine.core = None;
        assert_eq!(raise(&machine), Err(NOT_RUNNING.to_owned()));
        assert_eq!(machine.stopped.get(), 0);
        assert!(starts.started.borrow().is_empty());
        let _ = previous;
    }

    // B23: a core a desktop app opened is that app's to update: a raise of
    // the older core touches nothing.
    #[test]
    fn a_raise_leaves_a_core_its_app_opened_to_the_app() {
        let root = tempfile::tempdir().unwrap();
        let (args, mut machine, _, uploaded) = raise_fixture(root.path());
        machine.app = true;
        let starts = Starts::default();
        assert_eq!(
            raise_with(&args, &uploaded, ("new", &release(6)), &machine, &&starts),
            Ok(Outcome::LeftToApp)
        );
        assert!(machine.placed.borrow().is_empty());
        assert_eq!(machine.stopped.get(), 0);
        assert!(starts.started.borrow().is_empty());
    }

    // B23: two updates of one folder never overlap, so the raises of two
    // machines happen one after the other. While one holds the folder's
    // lock a second is told it is busy, and takes it once the first ended.
    #[test]
    fn a_second_update_of_a_folder_is_busy_until_the_first_ends() {
        let root = tempfile::tempdir().unwrap();
        let wait = Duration::from_millis(50);
        let first = hold_within(root.path(), wait).unwrap_or_else(|_| panic!("the folder's lock"));
        assert!(matches!(
            hold_within(root.path(), wait),
            Err(HoldError::Busy(_))
        ));
        drop(first);
        assert!(hold_within(root.path(), wait).is_ok());
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
