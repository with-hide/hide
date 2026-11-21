//! What the machine taking the core says about itself before a move (PRD
//! core-host-node-move B3, D-04, D-25, D-26), answered by `hided core-move
//! inspect` there: its engine can take the core, the account's GUI session runs
//! (the only one its login item runs in), the machine does not sleep by
//! itself on power, `gh` is signed in and every agent Hide AI asks answers.
//! A failing check says what it found; hide changes none of it.
//!
//! The checks of the account's logins (`gh`, Hide AI's agents) run the way
//! the moved core will: in a one-shot job of the GUI session with the core
//! login item's environment (`hided core-move check`), because a login kept
//! in the login keychain can answer otherwise to the SSH session `inspect`
//! runs in. The others read no login and run in `inspect` itself.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use super::control::{CheckId, FailedCheck};
pub use crate::core_install::checks::Programs;
use crate::core_install::checks::{Logins, gh_signed_in, gui_session, sleep_off};

/// How long the job of the logins' checks may take to answer: `gh` and
/// each agent's login probe.
const LOGINS_ANSWER_WITHIN: Duration = Duration::from_secs(30);

/// Whether the core's engine can move between machines. A move hands each
/// machine's panes over and renames the scope that names it in the
/// delivery ledger; it did both through Herdr, and the core's own engine
/// does neither yet, so every move is refused here with this reason until
/// it does (with-hide/hide-private#491).
pub fn engine_moves() -> Result<(), String> {
    Err("this version of Hide cannot move its core between machines yet".to_owned())
}

/// The scope that names this machine in the delivery ledger's host scopes,
/// which a move rewrites: a Herdr socket before. The core's own engine names
/// none a move can rewrite yet (`engine_moves`).
pub fn movable_scope() -> Option<String> {
    None
}

/// Every check that fails on this machine, in the dialog's order; `ai` is
/// the agents Hide AI asks, as `[[provider, model]]`, for the Ai check.
pub fn failing(
    home: &Path,
    state_dir: &Path,
    ai: Option<&str>,
    programs: &Programs,
) -> Vec<FailedCheck> {
    let stop = AtomicBool::new(false);
    let mut failed: Vec<FailedCheck> = [
        (CheckId::Engine, engine_moves()),
        (CheckId::GuiSession, gui_session(home, programs, &stop)),
        (CheckId::Sleep, sleep_off(home, programs, &stop)),
    ]
    .into_iter()
    .filter_map(|(check, result)| result.err().map(|detail| FailedCheck { check, detail }))
    .collect();
    // With no GUI session no job runs there; the logins are checked once
    // the operator logs in, which that failing check asks for.
    let no_session = failed
        .iter()
        .any(|check| check.check == CheckId::GuiSession);
    if !(no_session && matches!(programs.logins, Logins::InTheCoreSession)) {
        match logins_in_the_core_session(home, state_dir, ai, programs) {
            Ok(logins) => failed.extend(logins),
            Err(detail) => failed.push(FailedCheck {
                check: CheckId::GuiSession,
                detail: format!("the checks of this account's logins did not run there: {detail}"),
            }),
        }
    }
    failed
}

/// The logins' checks, run by `hided core-move check` in the job: each that
/// fails.
pub fn logins_failing(home: &Path, ai: Option<&str>, programs: &Programs) -> Vec<FailedCheck> {
    let stop = AtomicBool::new(false);
    let mut failed = Vec::new();
    if let Err(detail) = gh_signed_in(home, programs, &stop) {
        failed.push(FailedCheck {
            check: CheckId::Gh,
            detail,
        });
    }
    let asks = ai.map(serde_json::from_str::<Vec<(String, String)>>);
    let ready = match asks {
        None => Ok(()),
        Some(Ok(asks)) => hide_core::hide_ai_ready_here(&asks),
        Some(Err(error)) => Err(format!("--ai is not a list of agents: {error}")),
    };
    if let Err(detail) = ready {
        failed.push(FailedCheck {
            check: CheckId::Ai,
            detail,
        });
    }
    failed
}

/// Runs `hided core-move check` where [`Programs`] says and reads its
/// answer. The folder it answers in is removed on every path.
fn logins_in_the_core_session(
    home: &Path,
    state_dir: &Path,
    ai: Option<&str>,
    programs: &Programs,
) -> Result<Vec<FailedCheck>, String> {
    let incoming = hide_kit::layout::move_incoming(state_dir);
    // A check changes nothing in the folder: a parent it made goes with it.
    let made = !incoming.exists();
    let folder = incoming.join(format!("checks-{}", std::process::id()));
    match std::fs::remove_dir_all(&folder) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(format!("{}: {error}", folder.display()));
        }
        _ => {}
    }
    hide_platform::fs::private::create_dir_all(&folder)
        .map_err(|error| format!("{}: {error}", folder.display()))?;
    let answered = run_logins_check(home, state_dir, &folder, ai, programs);
    let removed = std::fs::remove_dir_all(&folder).and_then(|()| {
        if made {
            std::fs::remove_dir(&incoming)
        } else {
            Ok(())
        }
    });
    let failed = answered?;
    removed.map_err(|error| format!("{}: {error}", folder.display()))?;
    Ok(failed)
}

fn run_logins_check(
    home: &Path,
    state_dir: &Path,
    folder: &Path,
    ai: Option<&str>,
    programs: &Programs,
) -> Result<Vec<FailedCheck>, String> {
    let answer = folder.join("answer.json");
    let program =
        std::env::current_exe().map_err(|error| format!("this hided has no path: {error}"))?;
    let mut arguments = vec![
        "core-move".to_owned(),
        "check".to_owned(),
        "--state-dir".to_owned(),
        state_dir.to_string_lossy().into_owned(),
        "--answer".to_owned(),
        answer.to_string_lossy().into_owned(),
    ];
    if let Some(ai) = ai {
        arguments.extend(["--ai".to_owned(), ai.to_owned()]);
    }
    let stop = AtomicBool::new(false);
    match &programs.logins {
        Logins::Here => {
            let mut command = Command::new(&program);
            command.args(&arguments);
            let finished =
                hide_platform::process::run_to_end(&mut command, LOGINS_ANSWER_WITHIN, &stop)
                    .map_err(|failure| format!("the check did not answer: {failure:?}"))?;
            if !finished.succeeded() {
                return Err(format!(
                    "the check exited {:?}: {}",
                    finished.code,
                    finished.last_error_line()
                ));
            }
        }
        Logins::InTheCoreSession => {
            let agents = &programs.agents;
            let label = format!(
                "{}.checks",
                hide_kit::layout::core_login_item(home, state_dir)
            );
            let environment = crate::login_item::core_environment(home, state_dir)?;
            let environment: Vec<(&str, &str)> = environment
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str()))
                .collect();
            let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
            let log = folder.join("check.log");
            let agent = hide_platform::user_agents::LoginAgent {
                label: &label,
                program: &program,
                arguments: &arguments,
                environment: &environment,
                log: &log,
            };
            agents
                .start_once(&agent, folder, home, &stop)
                .map_err(|error| format!("the job {label}: {error}"))?;
            let waited = wait_for_answer(&answer, &|| {
                agents.last_exit(&label, home, &stop).ok().flatten()
            });
            let unloaded = agents.unload(&label, home, &stop);
            waited?;
            unloaded.map_err(|error| format!("the job {label}: {error}"))?;
        }
    }
    let bytes =
        std::fs::read(&answer).map_err(|error| format!("the check left no answer: {error}"))?;
    // The job runs this program, so its line is this build's.
    let line: super::answer::StepLine = serde_json::from_slice(&bytes)
        .map_err(|error| format!("the check's answer is unreadable: {error}"))?;
    match line.answer {
        super::answer::StepAnswer::Checked { failed } => Ok(failed),
        other => Err(format!("the check answered {other:?}")),
    }
}

/// Waits for the job's answer, which it writes once and whole before it
/// exits; a job that `exited` without one fails at once with its exit code.
// The job is another process that announces nothing to this one.
#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn wait_for_answer(answer: &Path, exited: &dyn Fn() -> Option<String>) -> Result<(), String> {
    let deadline = std::time::Instant::now() + LOGINS_ANSWER_WITHIN;
    let mut looks = 0_u32;
    while !answer.exists() {
        // Every half second: each look asks the job's system.
        looks += 1;
        if looks.is_multiple_of(5)
            && let Some(code) = exited()
            && !answer.exists()
        {
            return Err(format!("the check exited {code} without an answer"));
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "no answer within {}s",
                LOGINS_ANSWER_WITHIN.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
