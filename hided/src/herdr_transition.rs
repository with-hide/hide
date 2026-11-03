//! Moving this machine's Herdr session into the core's own engine (PRD
//! engine-bundle-4-herdr-removal D-3 to D-7, D-25 to D-27), the daemon's
//! half. The record, the conversion and the handover of Hide's state are the
//! core's (`herdr_core::herdr_import`); this half decides whose Herdr runs on
//! this machine's socket, stops Hide's own when its session moves, and has
//! the supervisor start the core role again, so the engine opens with what
//! moved: the one path a Herdr session takes into the engine (D-7).
//!
//! A move has two halves around the core's start. Once the core is ready
//! (D-27), a move is committed in the record, Hide's Herdr is stopped, and
//! the move is asked for; the core role then starts again, and before the
//! engine opens it moves the asked workspaces, gives their panes' Hide state
//! to the engine, and records what of the save is not moved yet.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use herdr_core::herdr_import::{self, Asked, Look};
use serde_json::json;

use crate::env::Env;

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
    let Some(place) = Place::of(env) else {
        return Look::Nothing;
    };
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
            &env.state_dir,
            node,
            &place.socket.to_string_lossy(),
            &place.engine_dir,
            &moved,
        )?;
        if given > 0 {
            log("herdr_transition.adopted", json!({ "registrations": given }));
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
pub(crate) enum Server {
    /// None runs: the save is Hide's when Hide registered an agent on that
    /// socket (D-5).
    Stopped { hide_used: bool },
    /// Hide's Herdr runs it: a Hide package's (also one renamed aside or in
    /// the Trash), or a runtime copy's, by the file it runs (D-5).
    Hide,
    /// Someone else's Herdr, or one whose program the system will not name:
    /// read only, never stopped (D-8).
    Other,
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
    if ours { Server::Hide } else { Server::Other }
}

/// Once the core is ready (D-27): Hide's own Herdr session moves without
/// asking when its server does not run (D-6, B1), and `restart` is told to
/// start the core role again with it. While Hide's Herdr runs the core asks
/// the operator first, and someone else's Herdr is only offered to import.
pub(crate) fn after_ready(env: &Env, look: &Look, restart: &Arc<tokio::sync::Notify>) {
    let (Some(place), Look::Offer(offer)) = (Place::of(env), look) else {
        return;
    };
    // Only the first move goes unasked: a save Herdr wrote after one is
    // offered on the screen (D-9), and a move that failed is not made again
    // at every start.
    if herdr_import::committed(&place.engine_dir)
        || server(&place, &env.state_dir) != (Server::Stopped { hide_used: true })
    {
        return;
    }
    log(
        "herdr_transition.move_unasked",
        json!({ "workspaces": offer.workspaces, "agents": offer.agents }),
    );
    if let Err(reason) = move_session(&place) {
        log("herdr_transition.refused", json!({ "reason": reason }));
        return;
    }
    restart.notify_one();
}

/// Commits the move and asks for it, for the core's next start: Hide's
/// Herdr no longer runs, so its agents resume (D-6).
fn move_session(place: &Place) -> Result<(), String> {
    herdr_import::commit(&place.engine_dir)?;
    herdr_import::ask(&place.engine_dir, Asked { resume: true })
}

