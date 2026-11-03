//! Moving this machine's Herdr session into the core's own engine (PRD
//! engine-bundle-4-herdr-removal D-3 to D-8, D-25 to D-27), the daemon's
//! half. The record, the conversion and the handover of Hide's state are the
//! core's (`herdr_core::herdr_import`); this half decides whose Herdr runs on
//! this machine's socket, stops Hide's own when its session moves, and has
//! the supervisor start the core role again, so the engine opens with what
//! moved: the one path a Herdr session takes into the engine (D-7).
//!
//! A move has two halves around the core's start. Once the core is ready
//! (D-27), a move is committed in the record, Hide's Herdr is stopped, and
//! the move is asked for: on its own for Hide's session that no server runs
//! (D-6, B1), or when the operator answers the core's question or imports a
//! save (`LocalMachine`). The core role then starts again, and before the
//! engine opens it moves the asked workspaces, gives their panes' Hide state
//! to the engine, and records what of the save is not moved yet.
//!
//! Every step of the record runs under one lock in this process: the start,
//! the unasked move and the core's looks and moves read and write the same
//! record.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use herdr_core::herdr_import::{self, Asked, Look, Seen, Take, Taken};
use serde_json::json;

use crate::env::Env;

/// How long Hide's Herdr has to end on each way of asking it (D-6).
const STOP_WITHIN: Duration = Duration::from_secs(10);

static RECORD: Mutex<()> = Mutex::new(());

fn hold_record() -> MutexGuard<'static, ()> {
    RECORD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// This machine's default Herdr session and the engine it moves into.
pub(crate) struct Place {
    engine_dir: PathBuf,
    session: PathBuf,
    socket: PathBuf,
}

impl Place {
    /// The place of `env`'s machine, when its core runs its own engine: the
    /// session Herdr keeps beside its default socket in its config folder
    /// (Herdr v0.9.5 `persist/io.rs`, `session_path`), which is the session
    /// Hide's Herdr ran.
    pub(crate) fn of(env: &Env) -> Option<Self> {
        if !env.own_engine {
            return None;
        }
        let variables = |name: &str| match name {
            "HOME" => Some(env.home.clone().into_os_string()),
            _ => std::env::var_os(name),
        };
        let config = hide_platform::host::herdr_config_dir_from(&variables).ok()?;
        Some(Self {
            engine_dir: hide_kit::layout::engine_dir(&env.state_dir),
            session: config.join("session.json"),
            socket: config.join("herdr.sock"),
        })
    }
}

fn log(kind: &str, fields: serde_json::Value) {
    let mut record = json!({ "component": "herdr_transition", "kind": kind });
    if let (Some(record), serde_json::Value::Object(fields)) = (record.as_object_mut(), fields) {
        record.extend(fields);
    }
    herdr_core::diagnostic!(record);
}

/// Before the core starts: the move asked for is made, the moved panes'
/// Hide state is given to the engine, and what of the save is not moved yet
/// is recorded, so the engine's cleanup never owns a pane of it (D-25). A
/// failure is logged and leaves the start to go on with what the engine
/// saved: the record holds what is not moved, and the screen offers it.
pub(crate) fn before_core(env: &Env, node: &str) -> Look {
    match Place::of(env) {
        Some(place) => before_core_at(&place, &env.state_dir, node),
        None => Look::Nothing,
    }
}

fn before_core_at(place: &Place, state_dir: &Path, node: &str) -> Look {
    let _held = hold_record();
    let look = (|| {
        let look = match herdr_import::asked(&place.engine_dir)? {
            Some(asked) => {
                let outcome = herdr_import::import(&place.engine_dir, &place.session, asked)?;
                log("herdr_transition.moved", json!({ "outcome": outcome }));
                herdr_import::look(&place.engine_dir, &place.session)?
            }
            None => herdr_import::look(&place.engine_dir, &place.session)?,
        };
        let moved = herdr_import::moved(&place.engine_dir)?;
        let given = herdr_import::adopt(
            state_dir,
            node,
            &place.socket.to_string_lossy(),
            &place.engine_dir,
            &moved,
        )?;
        if given > 0 {
            log(
                "herdr_transition.adopted",
                json!({ "registrations": given }),
            );
        }
        Ok::<_, String>(look)
    })();
    look.unwrap_or_else(|reason| {
        log("herdr_transition.refused", json!({ "reason": reason }));
        Look::Nothing
    })
}

/// Who runs the Herdr server on this machine's default socket.
#[derive(Debug, Eq, PartialEq)]
enum Server {
    /// None runs: the save is Hide's when Hide registered an agent on that
    /// socket (D-5).
    Stopped { hide_used: bool },
    /// Hide's Herdr runs it: a Hide package's (also one renamed aside or in
    /// the Trash), or a runtime copy's, by the file it runs (D-5), as the
    /// process `pid` that started at `started`.
    Hide { pid: u32, started: u64 },
    /// Someone else's Herdr, or one whose program the system will not name:
    /// read only, never stopped (D-8).
    Other,
}

impl Server {
    fn shown(&self) -> herdr_import::Server {
        match self {
            Self::Stopped { .. } => herdr_import::Server::Stopped,
            Self::Hide { .. } => herdr_import::Server::Hide,
            Self::Other => herdr_import::Server::Other,
        }
    }
}

/// Who runs the Herdr server on `place`'s socket.
fn server(place: &Place, state_dir: &Path) -> Server {
    let stream = match hide_platform::ipc::LocalStream::connect(&place.socket) {
        Ok(stream) => stream,
        Err(_) => {
            return Server::Stopped {
                hide_used: herdr_import::hide_used(state_dir, &place.socket.to_string_lossy()),
            };
        }
    };
    let Ok(pid) = stream.peer_pid() else {
        return Server::Other;
    };
    drop(stream);
    let ours = hide_platform::process::image_of(pid).is_ok_and(|image| {
        image
            .path
            .parent()
            .is_some_and(hide_kit::holds_package_runtime)
    });
    match hide_platform::process::start_time(pid) {
        Ok(started) if ours => Server::Hide { pid, started },
        _ => Server::Other,
    }
}

/// Once the core is ready (D-27): Hide's own Herdr session that no server
/// runs moves without asking (D-6, B1), and `restart` is told to start the
/// core role again with it; whether it was. While Hide's Herdr runs the core
/// asks the operator first, and any other save is only offered to import.
pub(crate) fn after_ready(env: &Env, look: &Look, restart: &tokio::sync::Notify) -> bool {
    Place::of(env).is_some_and(|place| after_ready_at(&place, &env.state_dir, look, restart))
}

fn after_ready_at(
    place: &Place,
    state_dir: &Path,
    look: &Look,
    restart: &tokio::sync::Notify,
) -> bool {
    let Look::Offer(offer) = look else {
        return false;
    };
    let _held = hold_record();
    // Only the first move goes unasked: a save Herdr wrote after one is
    // offered on the screen (D-9), and a move that failed is not made again
    // at every start.
    if herdr_import::committed(&place.engine_dir)
        || server(place, state_dir) != (Server::Stopped { hide_used: true })
    {
        return false;
    }
    log(
        "herdr_transition.move_unasked",
        json!({ "workspaces": offer.workspaces, "agents": offer.agents }),
    );
    if let Err(reason) = ask(place, true) {
        log("herdr_transition.refused", json!({ "reason": reason }));
        return false;
    }
    restart.notify_one();
    true
}

/// Commits the move, before its first step that cannot be undone (D-27),
/// and asks for it, for the core's next start.
fn ask(place: &Place, resume: bool) -> Result<(), String> {
    herdr_import::commit(&place.engine_dir)?;
    herdr_import::ask(&place.engine_dir, Asked { resume })
}

/// What the core asks about this machine's Herdr save (D-6 to D-8, B2 to
/// B5, B11, B14 to B17).
pub(crate) struct LocalMachine {
    /// `None` on a core that runs no engine of its own.
    place: Option<Place>,
    state_dir: PathBuf,
    restart: Arc<tokio::sync::Notify>,
}

impl LocalMachine {
    pub(crate) fn new(env: &Env, restart: Arc<tokio::sync::Notify>) -> Self {
        Self {
            place: Place::of(env),
            state_dir: env.state_dir.clone(),
            restart,
        }
    }

    fn take_at(&self, place: &Place, take: Take) -> Result<Taken, String> {
        let resume = match (take, server(place, &self.state_dir)) {
            (Take::Move, Server::Other) => {
                return Ok(Taken::Failed(
                    "the Herdr on this machine's socket is not Hide's".to_owned(),
                ));
            }
            (Take::Move, Server::Hide { pid, started }) => {
                herdr_import::commit(&place.engine_dir)?;
                if let Err(reason) = stop(&place.socket, pid, started) {
                    log(
                        "herdr_transition.not_stopped",
                        json!({ "pid": pid, "reason": reason }),
                    );
                    herdr_import::not_stopped(&place.engine_dir)?;
                    return Ok(Taken::NotStopped);
                }
                true
            }
            // An import stops nothing: the conversations of a Herdr that
            // runs on wait until it no longer does (D-8, B15).
            (_, Server::Stopped { .. }) => true,
            (Take::Import, Server::Hide { .. } | Server::Other) => false,
        };
        ask(place, resume)?;
        log(
            "herdr_transition.asked",
            json!({ "take": format!("{take:?}"), "resume": resume }),
        );
        self.restart.notify_one();
        Ok(Taken::Restarting)
    }
}

impl herdr_import::Machine for LocalMachine {
    fn look(&self) -> Seen {
        let Some(place) = &self.place else {
            return Seen::default();
        };
        let _held = hold_record();
        let server = server(place, &self.state_dir);
        let look = herdr_import::look(&place.engine_dir, &place.session).unwrap_or_else(|reason| {
            log("herdr_transition.refused", json!({ "reason": reason }));
            Look::Unreadable(reason)
        });
        // Hide's own session no server runs moves without asking once the
        // core is ready (`after_ready`): it is no offer.
        let look = match look {
            Look::Offer(_)
                if server == (Server::Stopped { hide_used: true })
                    && !herdr_import::committed(&place.engine_dir) =>
            {
                Look::Nothing
            }
            look => look,
        };
        let outcome = herdr_import::outcome(&place.engine_dir).unwrap_or_else(|reason| {
            log("herdr_transition.refused", json!({ "reason": reason }));
            None
        });
        Seen {
            look,
            server: server.shown(),
            outcome,
        }
    }

    fn take(&self, take: Take) -> Taken {
        let Some(place) = &self.place else {
            return Taken::Failed("this core runs no engine of its own".to_owned());
        };
        let _held = hold_record();
        self.take_at(place, take).unwrap_or_else(Taken::Failed)
    }

    fn dismiss(&self) {
        let Some(place) = &self.place else {
            return;
        };
        let _held = hold_record();
        if let Err(reason) = herdr_import::dismiss(&place.engine_dir) {
            log("herdr_transition.refused", json!({ "reason": reason }));
        }
    }
}

/// `hide import herdr [--yes]` (B17): what the core offers for this
/// machine's Herdr save, and with `yes` the move or import the screen's
/// button makes, asked of the core as one event: the core role then starts
/// again with it, so the answer says it was asked, not what it came to.
pub(crate) fn cli_answer(
    core: &crate::core::CoreHandle,
    yes: bool,
) -> Result<serde_json::Value, String> {
    // A core on Herdr has no session of Herdr's to move.
    if core.pane_holders().is_none() {
        return Ok(json!({ "own_engine": false }));
    }
    let shown = core.herdr_import_shown()?;
    let action = shown
        .as_ref()
        .filter(|shown| (shown.workspaces > 0 || shown.waiting > 0) && shown.stage != "moving")
        .and_then(|shown| match shown.kind.as_str() {
            "move" => Some("move"),
            "import" => Some("import"),
            _ => None,
        });
    let asked = match (yes, action) {
        (true, Some(action)) => {
            let event = json!({
                "schema_version": herdr_core::SCHEMA_VERSION,
                "kind": "herdr_import",
                "payload": { "action": action },
            });
            core.dispatch(event.to_string().into_bytes())?;
            true
        }
        _ => false,
    };
    Ok(json!({ "offered": shown, "asked": asked }))
}

/// Stops the Herdr server `pid`, which started at `started`, on `socket`:
/// by its own `server.stop`, which saves its session as its quit does, else
/// by the system's polite end (D-6); either way it is confirmed gone by its
/// pid and start, so a pid a new process took is never read as it. The
/// socket call needs no program file, so a server whose file an update
/// deleted stops the same way (B42).
fn stop(socket: &Path, pid: u32, started: u64) -> Result<(), String> {
    let same = || hide_platform::process::start_time(pid).is_ok_and(|now| now == started);
    let asked = herdr_core::engine::herdr::transition::stop_server(socket, STOP_WITHIN);
    if wait_gone(&same) {
        return Ok(());
    }
    if let Err(reason) = asked {
        log(
            "herdr_transition.stop_refused",
            json!({ "pid": pid, "reason": reason }),
        );
    }
    if same() {
        hide_platform::process::terminate(pid)
            .map_err(|error| format!("Herdr's server {pid} could not be ended: {error}"))?;
    }
    if wait_gone(&same) {
        Ok(())
    } else {
        Err(format!("Herdr's server {pid} did not end"))
    }
}

#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn wait_gone(same: &dyn Fn() -> bool) -> bool {
    // No event tells another process's exit, so its end is polled.
    let deadline = Instant::now() + STOP_WITHIN;
    while same() {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::FutureExt;
    use herdr_core::herdr_import::Machine;

    /// A machine whose Herdr save is a real Herdr 0.9.5 save in `dir`, with
    /// no server on its socket; nothing here reaches this host's Herdr.
    fn place(dir: &Path) -> (Place, PathBuf) {
        let config = dir.join("config").join("herdr");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../herdr-core/tests/fixtures/herdr-session/herdr-0.9.5.json"),
            config.join("session.json"),
        )
        .unwrap();
        let state = dir.join("state");
        std::fs::create_dir_all(&state).unwrap();
        let place = Place {
            engine_dir: hide_kit::layout::engine_dir(&state),
            session: config.join("session.json"),
            socket: config.join("herdr.sock"),
        };
        (place, state)
    }

    /// Hide registered an agent of `node` in pane `w1:p1` on `socket`.
    fn registered(state: &Path, node: &str, socket: &Path) {
        let ledger = herdr_core::delivery::ledger::from_bytes(
            json!({
                "version": 1, "next_id": 2, "letters": [], "watches": [],
                "agents": [{
                    "id": "agent-1", "name": "agent-1", "machine": node,
                    "host_scope": socket.to_string_lossy(), "native_machine": node,
                    "session": "s1", "instance": "term_1", "terminal_id": "term_1",
                    "pane": "w1:p1", "parent": null, "project": "/work",
                    // The actor carries its session's digest, SHA-256 of `s1`.
                    "actor": {
                        "pane_id": "w1:p1", "name": "agent-1", "kind": "claude", "device_id": node,
                        "session": "e8bc163c82eee18733288c7d4ac636db3a6deb013ef2d37b68322be20edc45cc",
                    },
                    "ended": false,
                }],
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        herdr_core::delivery::ledger::save(&hide_kit::layout::delivery_ledger(state), &ledger)
            .unwrap();
    }

    fn told(restart: &tokio::sync::Notify) -> bool {
        restart.notified().now_or_never().is_some()
    }

    // B1, D-5, D-7, D-27: Hide's own session that no server runs moves once
    // the core is ready, without asking; the next start moves it before the
    // engine opens and gives its registration to the engine, and nothing is
    // offered after it.
    #[test]
    fn hide_s_own_stopped_session_moves_without_asking_into_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        let (place, state) = place(dir.path());
        registered(&state, "mac", &place.socket);
        let look = before_core_at(&place, &state, "mac");
        assert!(matches!(look, Look::Offer(_)), "{look:?}");
        let restart = tokio::sync::Notify::new();
        assert!(after_ready_at(&place, &state, &look, &restart));
        assert!(told(&restart));
        assert!(herdr_import::committed(&place.engine_dir));

        assert_eq!(before_core_at(&place, &state, "mac"), Look::Nothing);
        assert_eq!(
            herdr_import::moved(&place.engine_dir).unwrap(),
            ["w1".to_owned(), "w2".to_owned()].into()
        );
        let ledger =
            herdr_core::delivery::ledger::load(&hide_kit::layout::delivery_ledger(&state)).unwrap();
        assert_eq!(
            (
                ledger.agents[0].host_scope.as_str(),
                ledger.agents[0].instance.as_str()
            ),
            (place.engine_dir.to_string_lossy().as_ref(), "w1:p1")
        );
        // A later start does not move it again.
        assert!(!after_ready_at(&place, &state, &Look::Nothing, &restart));
        assert!(!told(&restart));
    }

    // B14, B16, D-8: a save Hide never used is only offered, to import; the
    // import asks for the move and the core's next start makes it.
    #[test]
    fn a_save_hide_never_used_is_offered_and_its_import_moves_on_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        let (place, state) = place(dir.path());
        let look = before_core_at(&place, &state, "mac");
        let restart = Arc::new(tokio::sync::Notify::new());
        assert!(!after_ready_at(&place, &state, &look, &restart));
        let machine = LocalMachine {
            place: Some(place),
            state_dir: state.clone(),
            restart: Arc::clone(&restart),
        };
        let seen = machine.look();
        assert_eq!(
            (&seen.look, seen.server),
            (
                &Look::Offer(herdr_import::Offer {
                    workspaces: 2,
                    tabs: 3,
                    agents: 1,
                    waiting: 0,
                }),
                herdr_import::Server::Stopped
            )
        );
        assert_eq!(machine.take(Take::Import), Taken::Restarting);
        assert!(told(&restart));
        let place = machine.place.as_ref().unwrap();
        assert_eq!(before_core_at(place, &state, "mac"), Look::Nothing);
        assert_eq!(machine.look().look, Look::Nothing);
    }

    // B11: a save that cannot be read is offered as such, with why, and
    // moves nothing.
    #[test]
    fn an_unreadable_save_is_shown_with_why() {
        let dir = tempfile::tempdir().unwrap();
        let (place, state) = place(dir.path());
        std::fs::write(&place.session, "{").unwrap();
        let machine = LocalMachine {
            place: Some(place),
            state_dir: state,
            restart: Arc::new(tokio::sync::Notify::new()),
        };
        assert!(matches!(machine.look().look, Look::Unreadable(_)));
    }
}
