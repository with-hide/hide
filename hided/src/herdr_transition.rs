//! Moving this machine's Herdr session into the core's own engine (PRD
//! engine-bundle-4-herdr-removal D-3 to D-8, D-25 to D-27), the daemon's
//! half. The record, the conversion and the handover of Hide's state are the
//! core's (`hide_core::herdr_import`); this half decides whose Herdr runs on
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
//! The start, the unasked move and the core's looks and moves read and
//! write the same record; each change is made under the record's own lock
//! (`hide_core::herdr_import`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hide_core::herdr_import::{self, Asked, Look, Seen, Take, Taken};
use hide_host::herdr_save;
use serde_json::json;

use crate::env::Env;

/// This machine's default Herdr session and the engine it moves into.
pub(crate) struct Place {
    engine_dir: PathBuf,
    save: herdr_save::Place,
    home: PathBuf,
}

impl Place {
    /// The place of `env`'s machine (`herdr_save::Place`): the account's
    /// default Herdr session, where Hide's Herdr ran.
    pub(crate) fn of(env: &Env) -> Option<Self> {
        let save = herdr_save::Place::of_account(&env.home)
            .map_err(|reason| log("herdr_transition.unplaced", json!({ "reason": reason })))
            .ok()?;
        Some(Self {
            engine_dir: hide_kit::layout::engine_dir(&env.state_dir),
            save,
            home: env.home.clone(),
        })
    }

    fn fetch(&self) -> herdr_save::Fetched {
        herdr_save::fetch(&self.save.session)
    }
}

fn log(kind: &str, fields: serde_json::Value) {
    let mut record = json!({ "component": "herdr_transition", "kind": kind });
    if let (Some(record), serde_json::Value::Object(fields)) = (record.as_object_mut(), fields) {
        record.extend(fields);
    }
    hide_core::diagnostic!(record);
}

/// Before the core starts: the move asked for is made, the moved panes'
/// Hide state is given to the engine, and what of the save is not moved yet
/// is recorded, so the engine's cleanup never owns a pane of it (D-25). A
/// failure is logged and leaves the start to go on with what the engine
/// saved: the record holds what is not moved, and the screen offers it.
pub(crate) fn before_core(env: &Env, node: &str) -> Look {
    devices_before_core(&env.state_dir);
    match Place::of(env) {
        Some(place) => before_core_at(&place, &env.state_dir, node),
        None => Look::Nothing,
    }
}

/// Each device whose move was asked (D-10, B18): the save its node read is
/// moved into the device's engine and its panes' Hide state given to it,
/// before any link to the device is up, so the device's engine opens with
/// them as this machine's does. A device's registrations are scoped by its
/// id, which they keep.
fn devices_before_core(state_dir: &Path) {
    let devices = hide_kit::layout::engine_dir(state_dir).join("devices");
    let Ok(entries) = std::fs::read_dir(&devices) else {
        return;
    };
    for entry in entries.flatten() {
        let Some(device) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let engine_dir = entry.path();
        // The kept save goes once its panes' state is given, so a start cut
        // short after the import gives it on the next.
        let given = (|| {
            let Some(kept) = herdr_import::kept_save(&engine_dir)? else {
                return Ok(None);
            };
            if let Some(asked) = herdr_import::asked(&engine_dir)? {
                let outcome = herdr_import::import(&engine_dir, &kept.save, asked)?;
                log(
                    "herdr_transition.device_moved",
                    json!({ "device": device, "outcome": outcome }),
                );
            }
            let given = herdr_import::adopt(
                state_dir,
                &herdr_import::Handover {
                    machine: &kept.machine,
                    device: &device,
                    from_scope: &device,
                    to_scope: &device,
                },
                &herdr_import::moved(&engine_dir)?,
            )?;
            herdr_import::drop_kept_save(&engine_dir)?;
            Ok::<_, String>(Some(given))
        })();
        match given {
            Ok(Some(given)) => log(
                "herdr_transition.device_adopted",
                json!({ "device": device, "registrations": given }),
            ),
            Ok(None) => {}
            Err(reason) => log(
                "herdr_transition.refused",
                json!({ "device": device, "reason": reason }),
            ),
        }
    }
}

fn before_core_at(place: &Place, state_dir: &Path, node: &str) -> Look {
    let look = (|| {
        let look = match herdr_import::asked(&place.engine_dir)? {
            Some(mut asked) => {
                // A Herdr started again since the move was asked has its
                // conversations open: they wait rather than open twice (D-8).
                if asked.resume
                    && herdr_save::server(&place.save, &place.home) != herdr_save::Server::Stopped
                {
                    log("herdr_transition.resume_withheld", json!({}));
                    asked.resume = false;
                }
                let outcome = herdr_import::import(&place.engine_dir, &place.fetch(), asked)?;
                log("herdr_transition.moved", json!({ "outcome": outcome }));
                herdr_import::look(&place.engine_dir, &place.fetch())?
            }
            None => herdr_import::look(&place.engine_dir, &place.fetch())?,
        };
        let moved = herdr_import::moved(&place.engine_dir)?;
        let given = herdr_import::adopt(
            state_dir,
            &herdr_import::Handover {
                machine: node,
                device: node,
                from_scope: &place.save.socket.to_string_lossy(),
                to_scope: &place.engine_dir.to_string_lossy(),
            },
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
    match herdr_save::server(&place.save, &place.home) {
        herdr_save::Server::Stopped => Server::Stopped {
            hide_used: herdr_import::hide_used(state_dir, &place.save.socket.to_string_lossy()),
        },
        herdr_save::Server::Hide { pid, started } => Server::Hide { pid, started },
        herdr_save::Server::Other => Server::Other,
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
    if let Err(reason) = ask(place, true, true) {
        log("herdr_transition.refused", json!({ "reason": reason }));
        return false;
    }
    restart.notify_one();
    true
}

/// Asks for the move, for the core's next start; `commit` marks it
/// committed first, when the start is to give Hide's registrations to the
/// engine, a step that cannot be undone (D-27). A read-only import of a save
/// Hide never used commits nothing, so a failed update may still go back.
fn ask(place: &Place, resume: bool, commit: bool) -> Result<(), String> {
    if commit {
        herdr_import::commit(&place.engine_dir)?;
    }
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
        let server = server(place, &self.state_dir);
        // The start gives registrations to the engine only when Hide made
        // some on that socket; a move commits before it stops Hide's Herdr.
        let commit = matches!(take, Take::Move)
            || herdr_import::hide_used(&self.state_dir, &place.save.socket.to_string_lossy());
        let resume = match (take, server) {
            (Take::Move, Server::Other) => {
                return Ok(Taken::Failed(
                    "the Herdr on this machine's socket is not Hide's".to_owned(),
                ));
            }
            (Take::Move, Server::Hide { pid, started }) => {
                herdr_import::commit(&place.engine_dir)?;
                let refused = |reason: &str| {
                    log(
                        "herdr_transition.stop_refused",
                        json!({ "pid": pid, "reason": reason }),
                    );
                };
                if let Err(reason) = herdr_save::stop(&place.save.socket, pid, started, &refused) {
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
        ask(place, resume, commit)?;
        log(
            "herdr_transition.asked",
            json!({ "take": format!("{take:?}"), "resume": resume, "committed": commit }),
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
        let server = server(place, &self.state_dir);
        let look = herdr_import::look(&place.engine_dir, &place.fetch()).unwrap_or_else(|reason| {
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
        self.take_at(place, take).unwrap_or_else(Taken::Failed)
    }

    fn dismiss(&self) {
        let Some(place) = &self.place else {
            return;
        };
        if let Err(reason) = herdr_import::dismiss(&place.engine_dir) {
            log("herdr_transition.refused", json!({ "reason": reason }));
        }
    }

    fn restart(&self) {
        self.restart.notify_one();
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
                "schema_version": hide_core::SCHEMA_VERSION,
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

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::FutureExt;
    use hide_core::herdr_import::Machine;

    /// A machine whose Herdr save is a real Herdr 0.9.5 save in `dir`, with
    /// no server on its socket; nothing here reaches this host's Herdr.
    fn place(dir: &Path) -> (Place, PathBuf) {
        let config = dir.join("config").join("herdr");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../hide-core/tests/fixtures/herdr-session/herdr-0.9.5.json"),
            config.join("session.json"),
        )
        .unwrap();
        let state = dir.join("state");
        std::fs::create_dir_all(&state).unwrap();
        let place = Place {
            engine_dir: hide_kit::layout::engine_dir(&state),
            save: herdr_save::Place {
                session: config.join("session.json"),
                socket: config.join("herdr.sock"),
            },
            home: dir.join("home"),
        };
        (place, state)
    }

    /// Hide registered an agent of `node` in pane `w1:p1` on `socket`.
    fn registered(state: &Path, node: &str, socket: &Path) {
        let ledger = hide_core::delivery::ledger::from_bytes(
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
        hide_core::delivery::ledger::save(&hide_kit::layout::delivery_ledger(state), &ledger)
            .unwrap();
    }

    // D-10, B18: a device's move, asked with the save its node read, is made
    // at the next start before any link is up; its registration keeps the
    // device's id as its scope and takes its pane's id, and a later start
    // moves nothing again.
    #[test]
    fn a_device_s_asked_move_is_made_at_the_next_start_with_the_save_its_node_read() {
        let dir = tempfile::tempdir().unwrap();
        let (place, state) = place(dir.path());
        registered(&state, "m1", Path::new("d1"));
        let engine_dir = hide_kit::layout::device_engine_dir(&state, "d1").unwrap();
        herdr_import::commit(&engine_dir).unwrap();
        herdr_import::keep_save(
            &engine_dir,
            &herdr_import::KeptSave {
                machine: "m1".into(),
                save: place.fetch(),
            },
        )
        .unwrap();
        herdr_import::ask(&engine_dir, herdr_import::Asked { resume: true }).unwrap();

        devices_before_core(&state);
        assert_eq!(
            herdr_import::moved(&engine_dir).unwrap(),
            ["w1".to_owned(), "w2".to_owned()].into()
        );
        assert_eq!(herdr_import::kept_save(&engine_dir).unwrap(), None);
        let ledger =
            hide_core::delivery::ledger::load(&hide_kit::layout::delivery_ledger(&state)).unwrap();
        assert_eq!(
            (
                ledger.agents[0].host_scope.as_str(),
                ledger.agents[0].instance.as_str()
            ),
            ("d1", "w1:p1")
        );
        devices_before_core(&state);
        assert_eq!(herdr_import::asked(&engine_dir).unwrap(), None);
        // This machine's save is not the device's: it is still offered.
        assert!(matches!(
            before_core_at(&place, &state, "mac"),
            Look::Offer(_)
        ));
    }

    // D-8: a Herdr started again on the socket after the move was asked
    // keeps its conversations: the start imports them waiting, not opened
    // in two places.
    #[test]
    fn a_herdr_started_again_before_the_start_keeps_its_conversations_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let (place, state) = place(dir.path());
        herdr_import::ask(&place.engine_dir, Asked { resume: true }).unwrap();
        let _herdr = hide_platform::ipc::LocalListener::bind(&place.save.socket).unwrap();
        let look = before_core_at(&place, &state, "mac");
        assert!(
            matches!(
                look,
                Look::Offer(herdr_import::Offer {
                    workspaces: 0,
                    waiting: 1,
                    ..
                })
            ),
            "{look:?}"
        );
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
        registered(&state, "mac", &place.save.socket);
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
            hide_core::delivery::ledger::load(&hide_kit::layout::delivery_ledger(&state)).unwrap();
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
        // A read-only import of a save Hide never used commits nothing, so a
        // failed update may still go back (D-27).
        assert!(!herdr_import::committed(
            &machine.place.as_ref().unwrap().engine_dir
        ));
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
        std::fs::write(&place.save.session, "{").unwrap();
        let machine = LocalMachine {
            place: Some(place),
            state_dir: state,
            restart: Arc::new(tokio::sync::Notify::new()),
        };
        assert!(matches!(machine.look().look, Look::Unreadable(_)));
    }
}
