//! The process's supervisor (PRD core-host-node-move Q12): it holds the
//! instance lock and the seat for the process's life and runs one role at a
//! time on the seat. A start runs the role its folder names: the node role
//! where a placement record says this machine's core runs on another one,
//! which only a core moved there before 1.2.0 left, and the core role
//! everywhere else. A folder that such a move left unfinished starts no role
//! (`moved_core::unfinished`). A node that splits from its core is followed
//! by the core role in the same process (`moved_core::split`).

use std::path::Path;
use std::time::Duration;

use serde_json::json;

use crate::env::Env;
use crate::{RunningDaemon, RunningNode, placement, seat, server, state_file};

/// How long the core role has to stop before its process ends instead:
/// launchd's own allowance (`ExitTimeOut`, 20 s) for a login item's core it
/// boots out, so the bootout's wait (`UserAgents::unload`) covers it.
const CORE_STOP_WITHIN: Duration = Duration::from_secs(20);
/// The exit of a process whose core did not stop in time.
const STOP_UNCONFIRMED_EXIT: i32 = 3;

// One per process and never collected, so the size between its roles is
// not worth a box.
#[allow(clippy::large_enum_variant)]
enum Role {
    Core(RunningDaemon),
    Node(RunningNode),
}

pub async fn run(env: Env) -> Result<(), String> {
    let state_dir = env.state_dir.clone();
    let lock = state_file::acquire_lock(&env.state_dir).map_err(|error| error.to_string())?;
    // The process's records go beside its state from its start, so what
    // happens before any role runs is logged where its roles log.
    if let Err(error) = hide_core::diagnostics::install(&env.state_dir.join("core-state.json")) {
        eprintln!(
            "{}",
            json!({"kind": "diagnostics.open_failed", "message": error.to_string()})
        );
    }
    if let Some(record) = hide_node::diagnostics::raise_open_files() {
        hide_core::diagnostic!(record);
    }
    // Listened for before anything else waits, so a stop at any moment of
    // the process's life is kept.
    let stop = crate::ending::Ending::on_stop_signal();
    let seat = seat::Seat::serve(server::bind(env.bind).await?, state_file::new_token(), stop)?;
    // Recorded once for the process, so a window finds it whichever role
    // runs; a request before the first role waits for it (`seat`).
    crate::record_daemon(&state_dir, &seat.parts())?;
    seat.parts().starting("resume");
    let ran = async {
        let mut role = Some(first_role(&env, &seat).await?);
        while let Some(current) = role.take() {
            role = match current {
                Role::Core(running) => core_turn(&env, &seat, running).await?,
                Role::Node(running) => node_turn(&env, &seat, running).await?,
            };
        }
        Ok::<(), String>(())
    }
    .await;
    seat.close().await;
    // Its own state only, before the instance lock is released: a daemon
    // started after it writes its own.
    state_file::forget_daemon(&state_dir, std::process::id());
    drop(lock);
    ran
}

/// The role a start runs.
async fn first_role(env: &Env, seat: &seat::Seat) -> Result<Role, String> {
    if let Some(reason) = crate::moved_core::unfinished(&env.state_dir)? {
        log("start.refused", json!({"reason": reason}));
        return Err(reason);
    }
    let node = hide_core::node::NodeId::of_this_machine()?;
    if placement::read(&env.state_dir, node.as_str())?.is_some() {
        return Ok(Role::Node(
            crate::start_node_role(env.clone(), seat.parts()).await?,
        ));
    }
    Ok(Role::Core(
        crate::start_core_role(env.clone(), seat.parts()).await?,
    ))
}

/// Runs the core until it stops; answers the next role.
async fn core_turn(
    env: &Env,
    seat: &seat::Seat,
    running: RunningDaemon,
) -> Result<Option<Role>, String> {
    let import_restart = std::sync::Arc::clone(&running.import_restart);
    let restart = tokio::select! {
        () = running.shutdown.ended() => false,
        () = seat.stop.ended() => false,
        () = import_restart.notified() => true,
    };
    stop_core(&env.state_dir, running).await;
    if !restart {
        return Ok(None);
    }
    // A Herdr save's import was asked for: the core role starts again and
    // imports it before its engine opens; the pane holders run on (PRD
    // engine-bundle-4-herdr-removal D-7).
    log("herdr_import.restart", json!({}));
    Ok(Some(Role::Core(
        crate::start_core_role(env.clone(), seat.parts()).await?,
    )))
}

/// Runs the node until it stops, or until its machine was split from the
/// core it dialed (PRD device-connect-iroh B45): its placement record is
/// set aside by then, so the next role, in this process or at any later
/// start, is this machine's own core. The windows keep their address and
/// reconnect to it.
async fn node_turn(
    env: &Env,
    seat: &seat::Seat,
    running: RunningNode,
) -> Result<Option<Role>, String> {
    let mut became_core = running.became_core();
    let split = tokio::select! {
        () = running.shutdown.ended() => false,
        () = seat.stop.ended() => {
            running.stop();
            false
        }
        split = became_core.wait_for(|split| *split) => split.is_ok(),
    };
    stop_node(running).await;
    if !split {
        return Ok(None);
    }
    log("node.became_core", json!({}));
    Ok(Some(Role::Core(
        crate::start_core_role(env.clone(), seat.parts()).await?,
    )))
}

async fn stop_node(running: RunningNode) {
    let ended = tokio::task::spawn_blocking(move || drop(running)).await;
    if ended.is_err() {
        log("node.stop_failed", json!({}));
    }
}

/// Stops the core role and waits until its last saves are on disk. A core
/// that has not stopped within [`CORE_STOP_WITHIN`], or whose stop
/// panicked, cannot be ended apart from this process, so the process ends,
/// unsuccessfully, and its next start (a login item's keep-alive, or the
/// window's `hide connect`) runs the role its folder names.
async fn stop_core(state_dir: &Path, running: RunningDaemon) {
    // On a thread of its own, so a stop that never returns is never
    // dropped on this one.
    let stopping = tokio::task::spawn_blocking(move || {
        // A graceful stop takes hide's `tailscale serve` entry with it; a
        // crash leaves it to the next start's reconcile (PRD D-07).
        tokio::runtime::Handle::current().block_on(running.mobile.shutdown());
        drop(running);
    });
    let unconfirmed = match tokio::time::timeout(CORE_STOP_WITHIN, stopping).await {
        Ok(Ok(())) => return,
        // A stop that panicked may have left its last saves unwritten, so
        // nothing is made from the folder in this process either.
        Ok(Err(error)) => json!({"cause": "panicked", "message": error.to_string()}),
        Err(_) => json!({"cause": "timed_out", "within_ms": CORE_STOP_WITHIN.as_millis() as u64}),
    };
    let mut record = json!({"component": "supervisor", "kind": "core.stop_unconfirmed"});
    if let (Some(record), Some(fields)) = (record.as_object_mut(), unconfirmed.as_object()) {
        record.extend(fields.clone());
    }
    // The records queued before it first, then this one at once, which a
    // drain that timed out leaves written too.
    hide_core::diagnostics::drain();
    if let Err(error) = hide_core::diagnostics::record_now(
        &state_dir.join(hide_core::node_migration::CORE_STATE),
        record,
    ) {
        eprintln!("the unconfirmed stop could not be logged: {error}");
    }
    // `hided.json` is left naming this process, which a crash leaves too:
    // the next start of the folder writes its own once it holds the
    // instance lock, and every reader before that proves the pid by its
    // recorded start (`DaemonState::is_proven_running`), so the pid is
    // never signalled once another process has it.
    std::process::exit(STOP_UNCONFIRMED_EXIT);
}

fn log(kind: &str, fields: serde_json::Value) {
    let mut record = json!({"component": "supervisor", "kind": kind});
    if let (Some(record), Some(fields)) = (record.as_object_mut(), fields.as_object()) {
        record.extend(fields.clone());
    }
    hide_core::diagnostic!(record);
}
