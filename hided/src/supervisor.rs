//! The process's supervisor (PRD core-host-node-move Q12): it holds the
//! instance lock and the seat for the process's life, runs one role at a
//! time on the seat, and drives a core move, which switches the role from
//! the core to the move screen to the node, or back to the core when the
//! move is undone. A start that finds a move unresolved in the journal
//! resolves it before it runs any role of its own choosing.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use herdr_core::node_migration::{self, copy};
use hide_node_link::device::{SshProblem, Wake};
use serde_json::json;

use crate::core_move::abandoned::{self, Abandoned, PeerStep};
use crate::core_move::back::{self, Released, Resumed};
use crate::core_move::control::{
    Asked, CheckId, FailedCheck, MoveControl, MoveRequest, MoveState, MoveView, Release,
};
use crate::core_move::driver::{self, Remote, TargetSays};
use crate::core_move::handover::{self, Handover, HandoverState, now_unix_ms};
use crate::core_move::journal::{
    self, BackPhase, Direction, ForwardPhase, Journal, MoveFailure, MoveStep, Phase,
};
use crate::core_move::screen::MoveScreen;
use crate::env::Env;
use crate::placement::{self, Placement};
use crate::{RunningDaemon, RunningNode, seat, server, state_file};

mod move_back;
use move_back::{
    answer_release, back, back_checks_failed, back_rollback, commit_back, prepare_back,
    release_core, retire_and_commit,
};

/// How long past the target's lease a driver that cannot reach it waits
/// before it takes the lease as ended: the answer that told it the lease
/// took time to arrive, and two machines' clocks run at slightly different
/// rates.
const LEASE_MARGIN: Duration = Duration::from_secs(30);
/// How long the node's first link has to be taken.
const LINK_WITHIN: Duration = Duration::from_secs(30);
/// How often a driver that cannot reach the other machine tries again.
const RETRY_EVERY: Duration = Duration::from_secs(2);
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

/// Why the process's roles end with no next one.
#[derive(Debug)]
enum Halt {
    /// The process was asked to stop (`Seat::stop`); a move under way
    /// resumes from its journal at the next start, as after a kill.
    Stopped,
    Failed(String),
}

impl From<String> for Halt {
    fn from(reason: String) -> Self {
        Self::Failed(reason)
    }
}

pub async fn run(env: Env) -> Result<(), String> {
    let state_dir = env.state_dir.clone();
    if let Some(error) = crate::env::herdr_bin_error(&env) {
        return Err(error);
    }
    let lock = state_file::acquire_lock(&env.state_dir).map_err(|error| error.to_string())?;
    // The process's records go beside its state from its start, so a move
    // resumed before any role runs is logged where its roles log.
    if let Err(error) = herdr_core::diagnostics::install(&env.state_dir.join("core-state.json")) {
        eprintln!(
            "{}",
            json!({"kind": "diagnostics.open_failed", "message": error.to_string()})
        );
    }
    // Listened for before anything else waits, so a stop at any moment of
    // the process's life is kept.
    let stop = crate::ending::Ending::on_stop_signal();
    let seat = seat::Seat::serve(server::bind(env.bind).await?, state_file::new_token(), stop)?;
    let mut requests = seat
        .moves
        .take_requests()
        .ok_or("the move's requests were taken twice")?;
    // Recorded once for the process, so a window finds it whichever role
    // runs, the move screen included; a request before the first role
    // waits for it (`seat`).
    crate::record_daemon(&state_dir, &seat.parts(), env.herdr_socket_path.clone())?;
    seat.parts().starting("resume");
    // Whatever role runs, the moves given up here are ended on their
    // machines.
    let ending_abandoned = tokio::spawn(end_abandoned(env.clone(), seat.stop.clone()));
    let ran = async {
        let mut role = resume(&env, &seat).await?;
        while let Some(current) = role.take() {
            role = match current {
                Role::Core(running) => core_turn(&env, &seat, running, &mut requests).await?,
                Role::Node(running) => node_turn(&env, &seat, running, &mut requests).await?,
            };
        }
        Ok::<(), Halt>(())
    }
    .await;
    let ran = match ran {
        Ok(()) => Ok(()),
        Err(Halt::Stopped) => {
            log("process.stopped", json!({}));
            Ok(())
        }
        Err(Halt::Failed(reason)) => Err(reason),
    };
    ending_abandoned.abort();
    seat.close().await;
    // Its own state only, before the instance lock is released: a daemon
    // started after it writes its own.
    state_file::forget_daemon(&state_dir, std::process::id());
    drop(lock);
    ran
}

/// Runs the core until it stops or a move takes it; answers the next role.
async fn core_turn(
    env: &Env,
    seat: &seat::Seat,
    running: RunningDaemon,
    requests: &mut tokio::sync::mpsc::Receiver<Asked>,
) -> Result<Option<Role>, Halt> {
    enum Event {
        Stop,
        Request(MoveRequest),
        Release(Release),
        LeaseEnded,
    }
    let gate = Arc::clone(&running.move_gate);
    let event = tokio::select! {
        () = running.shutdown.ended() => Event::Stop,
        () = seat.stop.ended() => Event::Stop,
        Some(asked) = requests.recv() => match asked {
            Asked::Window(request) => Event::Request(request),
            Asked::Release(release) => Event::Release(release),
        },
        () = lease_end(&gate) => Event::LeaseEnded,
    };
    match event {
        Event::Release(release) => {
            let record = release_core(env, &running, &release);
            if !answer_release(&env.state_dir, &release, record).await {
                return Ok(Some(Role::Core(running)));
            }
            // The stop is recorded: this process runs no role after it, and
            // its next start runs none until the move resumes it.
            stop_core(&env.state_dir, running).await;
            Ok(None)
        }
        Event::Request(MoveRequest::CheckBack | MoveRequest::Back) => {
            seat.moves.set(MoveView {
                state: MoveState::ChecksFailed,
                direction: Some(Direction::Back),
                failed: vec![FailedCheck {
                    check: CheckId::Connection,
                    detail: "the core runs on this machine".to_owned(),
                }],
                ..MoveView::default()
            });
            Ok(Some(Role::Core(running)))
        }
        Event::Stop => {
            stop_core(&env.state_dir, running).await;
            Ok(None)
        }
        Event::Request(MoveRequest::Check { device }) => {
            let Some(prepared) =
                unless_stopped(&seat.stop, prepare(env, &running, &device, &seat.moves)).await
            else {
                stop_core(&env.state_dir, running).await;
                return Err(Halt::Stopped);
            };
            match prepared {
                Ok(_) => seat.moves.set(MoveView {
                    state: MoveState::Ready,
                    device: Some(device),
                    ..MoveView::default()
                }),
                Err(failed) => checks_failed(&seat.moves, &device, failed),
            }
            Ok(Some(Role::Core(running)))
        }
        // A pending core's window is refused before its request reaches
        // here (`core_pending`).
        Event::Request(MoveRequest::Start { device }) => {
            forward(env, seat, running, &device).await.map(Some)
        }
        Event::LeaseEnded => {
            // The link and the lease's end are decided under one lock: a
            // link taken first leaves nothing to end.
            let Some(intent) = gate.expire(now_unix_ms()) else {
                return Ok(Some(Role::Core(running)));
            };
            stop_core(&env.state_dir, running).await;
            give_back_pending(
                &env.state_dir,
                &env.home,
                &hide_kit::login_agents(&env.home),
                &intent,
            );
            Ok(None)
        }
    }
}

async fn node_turn(
    env: &Env,
    seat: &seat::Seat,
    running: RunningNode,
    requests: &mut tokio::sync::mpsc::Receiver<Asked>,
) -> Result<Option<Role>, Halt> {
    loop {
        tokio::select! {
            () = running.shutdown.ended() => break,
            () = seat.stop.ended() => {
                running.stop();
                break;
            }
            Some(asked) = requests.recv() => match asked {
                Asked::Release(release) => {
                    let _ = release.reply.send(Err("this machine runs no core".to_owned()));
                }
                Asked::Window(MoveRequest::CheckBack) => {
                    let Some(prepared) =
                        unless_stopped(&seat.stop, prepare_back(env, &running, &seat.moves)).await
                    else {
                        running.stop();
                        break;
                    };
                    match prepared {
                        Ok(prepared) => seat.moves.set(MoveView {
                            state: MoveState::Ready,
                            direction: Some(Direction::Back),
                            device: Some(prepared.journal.peer.device),
                            ..MoveView::default()
                        }),
                        Err(failed) => back_checks_failed(&seat.moves, failed),
                    }
                }
                Asked::Window(MoveRequest::Back) => {
                    return back(env, seat, running).await.map(Some);
                }
                Asked::Window(MoveRequest::Check { device } | MoveRequest::Start { device }) => {
                    seat.moves.set(MoveView {
                        state: MoveState::ChecksFailed,
                        direction: Some(Direction::Forward),
                        device: Some(device),
                        failed: vec![FailedCheck {
                            check: CheckId::Connection,
                            detail: "the core does not run on this machine".to_owned(),
                        }],
                        ..MoveView::default()
                    });
                }
            },
        }
    }
    stop_node(running).await;
    Ok(None)
}

async fn stop_node(running: RunningNode) {
    let ended = tokio::task::spawn_blocking(move || drop(running)).await;
    if ended.is_err() {
        log("node.stop_failed", json!({}));
    }
}

/// Never answers while the gate is open; answers at the pending lease's
/// recorded deadline, which no request in between moves.
async fn lease_end(gate: &crate::core_move::gate::MoveGate) {
    let Some(until) = gate.lease_until_unix_ms() else {
        return std::future::pending().await;
    };
    let mut open = gate.subscribe();
    tokio::select! {
        _ = open.wait_for(|open| *open) => std::future::pending().await,
        () = tokio::time::sleep(Duration::from_millis(until.saturating_sub(now_unix_ms()))) => {}
    }
}

/// A pending core whose move never linked gives its copy back to
/// `move-incoming`, where the driver's retry finds it, and leaves the
/// folder with no brain state.
fn give_back_pending(
    state_dir: &Path,
    home: &Path,
    agents: &hide_platform::user_agents::UserAgents,
    intent: &str,
) {
    let given = (|| -> Result<(), String> {
        let held = crate::core_move::handover::hold(state_dir)?;
        match held.read()? {
            Some(record) if record.intent == intent && record.state.is_pending() => {}
            _ => return Ok(()),
        }
        copy::unplace(
            state_dir,
            &crate::core_move::target::incoming(state_dir, intent),
            &herdr_core::hide_ai_settings_path(home),
        )
        .map_err(|refusal| refusal.to_string())?;
        held.remove()?;
        // Last, once nothing of the move is left: a login would otherwise
        // start an empty core on the folder. A copy not given back keeps it,
        // so the next login's pending core gives it back again.
        crate::login_item::forget(agents, home, state_dir)
    })();
    log(
        "pending.lease_ended",
        json!({"intent": intent, "given_back": given.is_ok(), "error": given.err()}),
    );
}

/// Stops the core role and waits until its last saves are on disk. A core
/// that has not stopped within [`CORE_STOP_WITHIN`], or whose stop
/// panicked, cannot be ended apart from this process, so the process ends,
/// unsuccessfully: its next start
/// (a login item's keep-alive, or the window's `hide connect`) resolves
/// the move its journal records, and a move that stopped nothing yet is
/// undone.
async fn stop_core(state_dir: &Path, running: RunningDaemon) {
    // On a thread of its own, so a stop that never returns is never
    // dropped on this one.
    let stopping = tokio::task::spawn_blocking(move || {
        if crate::env::fixture_core_stop_hangs() {
            loop {
                std::thread::park();
            }
        }
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
    let mut record = json!({"component": "core_move", "kind": "core.stop_unconfirmed"});
    if let (Some(record), Some(fields)) = (record.as_object_mut(), unconfirmed.as_object()) {
        record.extend(fields.clone());
    }
    // The records queued before it first, then this one at once, which a
    // drain that timed out leaves written too.
    herdr_core::diagnostics::drain();
    if let Err(error) = herdr_core::diagnostics::record_now(
        &state_dir.join(herdr_core::node_migration::CORE_STATE),
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
    let mut record = json!({"component": "core_move", "kind": kind});
    if let (Some(record), Some(fields)) = (record.as_object_mut(), fields.as_object()) {
        record.extend(fields.clone());
    }
    herdr_core::diagnostic!(record);
}

/// What a wait on the peer does with a failure asking again cannot change.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Permanent {
    /// The wait ends with it.
    Ends,
    /// It is asked again, ever more slowly: after the link that carries the
    /// intent may have been sent only the peer's answer decides, and the
    /// asking is a read, so repeating it repeats nothing (D-07).
    AskedAgain,
}

/// Asks the peer with `ask` until it answers. A transient failure
/// (`MoveFailure::transient`) is asked again every [`RETRY_EVERY`] unless
/// `give_up` takes it; a permanent one is `permanent`'s to end or ask again
/// after a wait that grows to a minute. The window shows Waiting with the
/// failure meanwhile, and a failure is logged as `kind` when it changes.
/// A stop of the process ends the wait at once, the ask in flight
/// included: the journal resumes the move at the next start.
async fn wait_on_peer<T: Send + 'static>(
    stop: &crate::ending::Ending,
    moves: &MoveControl,
    kind: &str,
    intent: &str,
    permanent: Permanent,
    ask: impl Fn() -> Result<T, MoveFailure> + Send + Sync + 'static,
    mut give_up: impl FnMut(&MoveFailure) -> bool,
) -> Result<Result<T, MoveFailure>, Halt> {
    let ask = Arc::new(ask);
    let mut last: Option<MoveFailure> = None;
    let mut slower = crate::backoff::Backoff::default();
    loop {
        let asked = {
            let ask = Arc::clone(&ask);
            unless_stopped(stop, tokio::task::spawn_blocking(move || ask()))
                .await
                .ok_or(Halt::Stopped)?
                .map_err(|error| error.to_string())?
        };
        let failure = match asked {
            Ok(answer) => return Ok(Ok(answer)),
            Err(failure) => failure,
        };
        if last.as_ref() != Some(&failure) {
            log(kind, json!({"intent": intent, "failure": failure}));
            last = Some(failure.clone());
        }
        let given_up = failure.transient() && give_up(&failure);
        let Some(wait) = next_ask(&failure, permanent, given_up, &mut slower) else {
            return Ok(Err(failure));
        };
        moves.update(|view| {
            view.state = MoveState::Waiting;
            view.cause = Some(failure.clone());
        });
        unless_stopped(stop, tokio::time::sleep(wait))
            .await
            .ok_or(Halt::Stopped)?;
    }
}

/// Asks `remove` again, ever more slowly, until it removes what it names:
/// the move has decided where the core runs, and this machine starts its
/// role only once nothing of the move says otherwise. Until then the window
/// shows Waiting with the failure, which is logged as `kind` when it
/// changes.
async fn until_removed(
    seat: &seat::Seat,
    intent: &str,
    kind: &str,
    remove: impl Fn() -> Result<(), String>,
) -> Result<(), Halt> {
    let mut slower = crate::backoff::Backoff::default();
    let mut last = None;
    loop {
        let reason = match remove() {
            Ok(()) => return Ok(()),
            Err(reason) => reason,
        };
        if last.as_ref() != Some(&reason) {
            log(kind, json!({"intent": intent, "reason": reason}));
            last = Some(reason.clone());
        }
        seat.moves.update(|view| {
            view.state = MoveState::Waiting;
            view.cause = Some(MoveFailure::Local { reason });
        });
        unless_stopped(&seat.stop, tokio::time::sleep(slower.failed()))
            .await
            .ok_or(Halt::Stopped)?;
    }
}

/// Runs `work` unless the process is asked to stop first; `None` when it
/// was, with `work` dropped. Blocking work it started runs on to its own
/// end, which the process's exit bounds (`hided` main).
async fn unless_stopped<T>(
    stop: &crate::ending::Ending,
    work: impl std::future::Future<Output = T>,
) -> Option<T> {
    tokio::select! {
        biased;
        () = stop.ended() => None,
        done = work => Some(done),
    }
}

/// How long a wait on the peer waits before it asks again after `failure`;
/// `None` ends the wait. Only a transient failure is asked again at once,
/// unless the caller gave it up; a permanent one is asked again, ever more
/// slowly, only where `permanent` says so.
fn next_ask(
    failure: &MoveFailure,
    permanent: Permanent,
    given_up: bool,
    slower: &mut crate::backoff::Backoff,
) -> Option<Duration> {
    if failure.transient() {
        slower.reset();
        return (!given_up).then_some(RETRY_EVERY);
    }
    match permanent {
        Permanent::Ends => None,
        Permanent::AskedAgain => Some(slower.failed()),
    }
}

/// The role a start runs: a move the journal left unresolved first, then
/// the placement record's choice.
async fn resume(env: &Env, seat: &seat::Seat) -> Result<Option<Role>, Halt> {
    // A core this folder stopped or retired for a move back starts again
    // only when that move resumes it; a start meanwhile (a login, launchd's
    // keep-alive) ends at once and successfully, so it is not retried.
    if let Some(record) = crate::core_move::handover::read(&env.state_dir)?
        && matches!(
            record.state,
            HandoverState::StoppedFor | HandoverState::Retired
        )
    {
        log(
            "start.held",
            json!({"intent": record.intent, "state": record.state}),
        );
        return Ok(None);
    }
    if let Some(journal) = journal::read(&env.state_dir)?
        && journal.phase.holds_the_core()
        && journal.phase.direction() == Direction::Back
    {
        log(
            "move.resumed",
            json!({"intent": journal.intent, "phase": journal.phase}),
        );
        let remote = Arc::new(remote_for(env, &journal)?);
        let screen = MoveScreen::mount(&seat.parts(), env.vite_origin.as_deref());
        let step = journal.phase.step();
        let role = if journal.phase == Phase::Back(BackPhase::Retiring) {
            // Taken here before the start that ended it could finish.
            if abandoned::read(&env.state_dir)?
                .iter()
                .any(|kept| kept.intent == journal.intent)
            {
                commit_back(env, seat, journal, screen).await?
            } else {
                retire_and_commit(env, seat, journal, remote, screen).await?
            }
        } else {
            back_rollback(
                env,
                seat,
                journal,
                &remote,
                screen,
                step,
                MoveFailure::Local {
                    reason: "the move was interrupted".to_owned(),
                },
            )
            .await?
        };
        return Ok(Some(role));
    }
    resume_forward(env, seat).await.map(Some)
}

async fn resume_forward(env: &Env, seat: &seat::Seat) -> Result<Role, Halt> {
    if let Some(journal) = journal::read(&env.state_dir)?
        && journal.phase.holds_the_core()
    {
        log(
            "move.resumed",
            json!({"intent": journal.intent, "phase": journal.phase}),
        );
        let remote = Arc::new(remote_for(env, &journal)?);
        return match journal.phase {
            Phase::Forward(ForwardPhase::Committed) => {
                let node = crate::start_node_role(env.clone(), seat.parts()).await?;
                commit(env, &seat.moves, journal, &remote).await;
                Ok(Role::Node(node))
            }
            // Given up before the start that ended it could finish.
            Phase::Forward(ForwardPhase::AttachSent)
                if abandoned::read(&env.state_dir)?
                    .iter()
                    .any(|kept| kept.intent == journal.intent) =>
            {
                abandon_here(env, seat, journal, None).await
            }
            Phase::Forward(ForwardPhase::AttachSent) => {
                let node = crate::start_node_role(env.clone(), seat.parts()).await?;
                link_outcome(env, seat, journal, remote, node).await
            }
            _ => {
                let step = journal.phase.step();
                rollback(
                    env,
                    seat,
                    journal,
                    &remote,
                    step,
                    MoveFailure::Local {
                        reason: "the move was interrupted".to_owned(),
                    },
                )
                .await
            }
        };
    }
    let node = herdr_core::node::NodeId::of_this_machine()?;
    if placement::read(&env.state_dir, node.as_str())?.is_some() {
        return Ok(Role::Node(
            crate::start_node_role(env.clone(), seat.parts()).await?,
        ));
    }
    Ok(Role::Core(
        crate::start_core_role(env.clone(), seat.parts()).await?,
    ))
}

/// The peer `journal` names. It reaches nothing yet, so a peer whose alias
/// does not resolve now is a failure of the first wait that asks it, never
/// of the start that resumes the move.
fn remote_for(env: &Env, journal: &Journal) -> Result<Remote, String> {
    Ok(Remote::new(
        &env.home,
        &journal.peer.alias,
        &journal.peer.program,
        &own_build(env)?,
    )
    .with_state_dir(&journal.peer.state_dir))
}

/// This daemon's build, the only one whose move answers it reads.
fn own_build(env: &Env) -> Result<String, String> {
    env.build
        .clone()
        .map_or_else(crate::build_id::of_current_exe, Ok)
}

/// What a move is made from, once every check passed.
struct Prepared {
    journal: Journal,
    labels: serde_json::Value,
    remote: Remote,
}

/// The connection check stopped on `ssh`: the view says what the operator
/// does about it as the device row does, which [`checks_failed`] keeps.
pub(super) fn stopped_on_ssh(moves: &MoveControl, ssh: Option<SshProblem>) {
    moves.update(|view| {
        view.ssh_problem =
            ssh.map(|problem| herdr_core::DeviceProblem::of(problem).code().to_owned());
    });
}

fn checks_failed(moves: &MoveControl, device: &str, failed: Vec<FailedCheck>) {
    let checked =
        crate::core_move::control::checks_run(&crate::core_move::control::FORWARD_CHECKS, &failed);
    log(
        "checks.failed",
        json!({"device": device, "checks": failed.iter().map(|check| check.check).collect::<Vec<_>>(), "checked": checked, "failed": failed}),
    );
    moves.set(MoveView {
        state: MoveState::ChecksFailed,
        device: Some(device.to_owned()),
        checked,
        failed,
        ssh_problem: moves.view().ssh_problem,
        ..MoveView::default()
    });
}

/// Runs every check of a move to `device` with nothing changed on either
/// machine, and answers what the move is made from.
async fn prepare(
    env: &Env,
    running: &RunningDaemon,
    device: &str,
    moves: &MoveControl,
) -> Result<Prepared, Vec<FailedCheck>> {
    moves.set(MoveView {
        state: MoveState::Checking,
        device: Some(device.to_owned()),
        ..MoveView::default()
    });
    let fail = |check: CheckId, detail: String| vec![FailedCheck { check, detail }];
    let core = Arc::clone(&running.core);
    let asked = device.to_owned();
    let answer = tokio::task::spawn_blocking(move || core.move_source(&asked))
        .await
        .map_err(|error| fail(CheckId::Connection, error.to_string()))?
        .map_err(|error| fail(CheckId::Connection, error))?;
    let (source, labels) =
        answer.map_err(|refusal| fail(CheckId::Connection, refusal.code().to_owned()))?;
    let mut failed = Vec::new();
    if !source.other_linked_nodes.is_empty() {
        failed.push(FailedCheck {
            check: CheckId::OtherNode,
            detail: source.other_linked_nodes.join(", "),
        });
    }
    let own_socket = env.herdr_socket_path.clone();
    if own_socket.is_none() {
        failed.push(FailedCheck {
            check: CheckId::Herdr,
            detail: "this machine has no Herdr server".to_owned(),
        });
    }
    // This machine's own setting, which moves with the core: not a login
    // of the machine taking it.
    let asks = match herdr_core::hide_ai_asks(&env.home) {
        Ok(asks) => asks,
        Err(detail) => {
            failed.push(FailedCheck {
                check: CheckId::AiSetup,
                detail,
            });
            Vec::new()
        }
    };
    let build = match own_build(env) {
        Ok(build) => build,
        Err(reason) => {
            failed.push(FailedCheck {
                check: CheckId::Build,
                detail: reason,
            });
            return Err(failed);
        }
    };
    // A journal that cannot be read fails the check rather than reading as
    // no earlier try, which would lose the retry's intent and its copy.
    let previous = match journal::read(&env.state_dir) {
        Ok(previous) => previous,
        Err(detail) => {
            failed.push(FailedCheck {
                check: CheckId::Connection,
                detail,
            });
            return Err(failed);
        }
    };
    let retry =
        previous.filter(|journal| journal.peer.device == device && journal.phase.is_rolled_back());
    let home = env.home.clone();
    let (alias, program) = (source.ssh_alias.clone(), source.helper_path.clone());
    let own = build.clone();
    let stranded = retry.clone();
    let sign_in = moves.sign_in();
    let inspected = tokio::task::spawn_blocking(move || {
        let remote = Remote::new(&home, &alias, &program, &own).with_sign_in(sign_in);
        // The operator pressed for these checks, so their dial may ask what
        // ssh asks in the dialog; the move's later dials ask nobody (D-06).
        remote.may_ask(Wake::Allowed);
        let inspected = (|| {
            let inspected = driver::inspect(&remote, &asks)?;
            // A copy an earlier try placed there and could not take back (the
            // device unreachable as it rolled back) is taken back first.
            let inspected = match (&stranded, &inspected.handover) {
                (Some(retry), Some(handover))
                    if handover.intent == retry.intent && handover.state.is_pending() =>
                {
                    let aborted = driver::abort_target(&remote, retry)?;
                    log(
                        "retry.aborted_stranded",
                        json!({"intent": retry.intent, "aborted": format!("{aborted:?}")}),
                    );
                    driver::inspect(&remote, &asks)?
                }
                _ => inspected,
            };
            Ok::<_, MoveFailure>(inspected)
        })();
        remote.may_ask(Wake::NotAllowed);
        let ssh = remote.ssh_problem();
        inspected
            .map(|inspected| (remote, inspected))
            .map_err(|failure| (failure, ssh))
    })
    .await
    .map_err(|error| fail(CheckId::Connection, error.to_string()))?;
    let (remote, inspected) = match inspected {
        Ok(found) => found,
        // Another build's answers are not read: its build is the one check
        // it can fail.
        Err((MoveFailure::OtherBuild { build: theirs }, _)) => {
            failed.push(FailedCheck {
                check: CheckId::Build,
                detail: format!("the device runs {theirs}, this machine {build}"),
            });
            return Err(failed);
        }
        Err((failure, ssh)) => {
            stopped_on_ssh(moves, ssh);
            failed.push(FailedCheck {
                check: CheckId::Connection,
                detail: format!("{failure:?}"),
            });
            return Err(failed);
        }
    };
    if inspected.node != source.device_node {
        failed.push(FailedCheck {
            check: CheckId::Identity,
            detail: inspected.node.clone(),
        });
    }
    failed.extend(inspected.failed.iter().cloned());
    if !source.dormant.is_empty() {
        failed.push(FailedCheck {
            check: CheckId::Dormant,
            detail: source.dormant.join(", "),
        });
    }

    // A core the device retired for a move back holds nothing there.
    let other_move = inspected.handover.as_ref().is_some_and(|handover| {
        handover.state != HandoverState::Retired
            && retry
                .as_ref()
                .is_none_or(|retry| retry.intent != handover.intent)
    });
    let mut held = inspected.brain.clone();
    if let Some(handover) = inspected.handover.as_ref().filter(|_| other_move) {
        held.push(format!("move {}", handover.intent));
    }
    // Named here, before anything stops: the copy to that folder would
    // fail only after this machine's core stopped. It has a check of its
    // own, since clearing earlier records never helps.
    if let Some(reason) = inspected.path_refused.clone() {
        failed.push(FailedCheck {
            check: CheckId::TargetFolder,
            detail: reason,
        });
    }
    // Hide AI settings move with the core and are never merged.
    match (
        &inspected.ai,
        herdr_core::stored_hide_ai_settings(&env.home),
    ) {
        (Ok(None), _) => {}
        (Ok(Some(theirs)), Ok(Some(ours))) if *theirs == ours => {}
        (Ok(Some(_)), _) => held.push("Hide AI settings unlike this machine's".to_owned()),
        (Err(reason), _) => held.push(reason.clone()),
    }
    if !held.is_empty() {
        failed.push(FailedCheck {
            check: CheckId::TargetState,
            detail: format!("{}: {}", inspected.state_dir, held.join(", ")),
        });
    }
    let Some(target_socket) = inspected.herdr_socket.clone() else {
        if !inspected
            .failed
            .iter()
            .any(|check| check.check == CheckId::Herdr)
        {
            failed.push(FailedCheck {
                check: CheckId::Herdr,
                detail: "the device has no Herdr server".to_owned(),
            });
        }
        return Err(failed);
    };
    if !failed.is_empty() {
        return Err(failed);
    }
    let Some(own_socket) = own_socket else {
        return Err(failed);
    };
    let host_name = crate::host_name();
    let (change, ids) = driver::forward_change(
        &source,
        &env.home,
        host_name.as_deref(),
        own_socket,
        target_socket,
    )
    .map_err(|failure| fail(CheckId::Connection, format!("{failure:?}")))?;
    // A retry keeps its intent, so the copy the device holds from it is
    // checked by digest rather than sent again.
    let intent = retry
        .map(|journal| journal.intent)
        .unwrap_or_else(crate::core_move::new_intent);
    let journal = Journal::new(
        intent,
        Phase::Forward(ForwardPhase::Stopping),
        driver::peer(&source, &inspected),
        change,
        ids,
    );
    Ok(Prepared {
        journal,
        labels,
        remote,
    })
}

/// A forward move of this machine's core to `device`.
async fn forward(
    env: &Env,
    seat: &seat::Seat,
    running: RunningDaemon,
    device: &str,
) -> Result<Role, Halt> {
    let moves = Arc::clone(&seat.moves);
    let Some(prepared) = unless_stopped(&seat.stop, prepare(env, &running, device, &moves)).await
    else {
        stop_core(&env.state_dir, running).await;
        return Err(Halt::Stopped);
    };
    let prepared = match prepared {
        Ok(prepared) => prepared,
        Err(failed) => {
            checks_failed(&moves, device, failed);
            return Ok(Role::Core(running));
        }
    };
    let Prepared {
        journal,
        labels,
        remote,
    } = prepared;
    if let Err(reason) = journal::write(&env.state_dir, &journal) {
        moves.set(MoveView {
            state: MoveState::RolledBack,
            device: Some(device.to_owned()),
            step: Some(MoveStep::StopCore),
            cause: Some(MoveFailure::Local { reason }),
            ..MoveView::default()
        });
        return Ok(Role::Core(running));
    }
    log(
        "move.started",
        json!({"intent": journal.intent, "device": device, "target": journal.peer.node}),
    );
    let shown = MoveView::of(&journal, MoveState::Stopping);
    let view = |state: MoveState| MoveView {
        state,
        ..shown.clone()
    };
    moves.set(view(MoveState::Stopping));
    // Mounted before the core stops: a window whose socket the stopping core
    // closes reconnects to the move's screen and hears the move (W1).
    let screen = MoveScreen::mount(&seat.parts(), env.vite_origin.as_deref());
    stop_core(&env.state_dir, running).await;
    moves.set(view(MoveState::Copying));
    let remote = Arc::new(remote);
    let steps = {
        let (state_dir, settings, remote, moves) = (
            env.state_dir.clone(),
            herdr_core::hide_ai_settings_path(&env.home),
            Arc::clone(&remote),
            Arc::clone(&moves),
        );
        tokio::task::spawn_blocking(move || {
            copy_and_start(&state_dir, &settings, journal, &labels, &remote, &moves)
        })
    };
    let journal = match unless_stopped(&seat.stop, steps)
        .await
        .ok_or(Halt::Stopped)?
    {
        Ok(Ok(journal)) => journal,
        Ok(Err((journal, step, cause))) => {
            drop(screen);
            return rollback(env, seat, *journal, &remote, step, cause).await;
        }
        Err(error) => return Err(format!("the move's steps failed: {error}").into()),
    };
    link(env, seat, journal, remote, screen).await
}

type StepFailure = (Box<Journal>, MoveStep, MoveFailure);

/// Stages, sends, places and starts the copy, recording each phase.
fn copy_and_start(
    state_dir: &Path,
    ai_settings: &Path,
    mut journal: Journal,
    labels: &serde_json::Value,
    remote: &Remote,
    moves: &MoveControl,
) -> Result<Journal, StepFailure> {
    let manifest = match driver::stage(state_dir, ai_settings, &journal, labels) {
        Ok(manifest) => manifest,
        Err(cause) => return Err((Box::new(journal), MoveStep::Copy, cause)),
    };
    let progress = |sent: u64, total: u64| {
        moves.update(|view| {
            view.sent = sent;
            view.total = total;
        });
    };
    if let Err(cause) = driver::send(remote, state_dir, &journal, &manifest, &progress) {
        return Err((Box::new(journal), MoveStep::Copy, cause));
    }
    let record = |journal: &mut Journal, phase: Phase| {
        journal.phase = phase;
        journal::write(state_dir, journal).map_err(|reason| MoveFailure::Local { reason })
    };
    if let Err(cause) = record(&mut journal, Phase::Forward(ForwardPhase::Sent)) {
        return Err((Box::new(journal), MoveStep::Copy, cause));
    }
    // Recorded first: a placement whose answer is lost is undone all the
    // same.
    if let Err(cause) = record(&mut journal, Phase::Forward(ForwardPhase::Placed))
        .and_then(|()| driver::place(remote, &journal))
    {
        return Err((Box::new(journal), MoveStep::Copy, cause));
    }
    let placed = std::time::Instant::now();
    moves.update(|view| view.state = MoveState::Starting);
    // Should the answer be lost, the lease the place recorded runs at most
    // its full span from the place's answer.
    let unanswered = handover::PLACED_LEASE + LEASE_MARGIN;
    journal.target_lease_until_unix_ms = Some(now_unix_ms() + unanswered.as_millis() as u64);
    let lease_left = match record(&mut journal, Phase::Forward(ForwardPhase::TargetStarted))
        .and_then(|()| driver::start_target(remote, &journal))
    {
        Ok(lease_left) => lease_left,
        Err(cause) => return Err((Box::new(journal), MoveStep::StartTarget, cause)),
    };
    // What the start took of the lease, which the place's span bounds.
    log(
        "target.started",
        json!({
            "intent": journal.intent,
            "start_ms": placed.elapsed().as_millis() as u64,
            "lease_left_ms": lease_left.as_millis() as u64,
        }),
    );
    journal.target_lease_until_unix_ms =
        Some(now_unix_ms() + (lease_left + LEASE_MARGIN).as_millis() as u64);
    if let Err(cause) = journal::write(state_dir, &journal) {
        return Err((
            Box::new(journal),
            MoveStep::StartTarget,
            MoveFailure::Local { reason: cause },
        ));
    }
    Ok(journal)
}

/// Points this machine at the new core and links to it as a node; the link
/// carrying the intent is the commit.
async fn link(
    env: &Env,
    seat: &seat::Seat,
    mut journal: Journal,
    remote: Arc<Remote>,
    screen: MoveScreen,
) -> Result<Role, Halt> {
    seat.moves.update(|view| view.state = MoveState::Linking);
    let written = placement::write(
        &env.state_dir,
        &Placement {
            alias: journal.peer.alias.clone(),
            node: journal.peer.node.clone(),
            program: journal.peer.program.clone(),
            state_dir: Some(journal.peer.state_dir.clone()),
            move_intent: Some(journal.intent.clone()),
            disconnected: false,
        },
    );
    if let Err(reason) = written {
        drop(screen);
        return rollback(
            env,
            seat,
            journal,
            &remote,
            MoveStep::Reattach,
            MoveFailure::Local { reason },
        )
        .await;
    }
    journal.phase = Phase::Forward(ForwardPhase::AttachSent);
    if let Err(reason) = journal::write(&env.state_dir, &journal) {
        drop(screen);
        return rollback(
            env,
            seat,
            journal,
            &remote,
            MoveStep::Reattach,
            MoveFailure::Local { reason },
        )
        .await;
    }
    let node = crate::start_node_role(env.clone(), seat.parts()).await;
    drop(screen);
    match node {
        Ok(node) => link_outcome(env, seat, journal, remote, node).await,
        Err(reason) => {
            // No link left this machine: the peer cannot have committed,
            // which its handover confirms before the move is undone.
            log(
                "link.not_started",
                json!({"intent": journal.intent, "reason": reason}),
            );
            resolve_unlinked(env, seat, journal, remote, None).await
        }
    }
}

/// Waits for the node's first link and goes forward or back on what the
/// peer's handover says.
async fn link_outcome(
    env: &Env,
    seat: &seat::Seat,
    journal: Journal,
    remote: Arc<Remote>,
    node: RunningNode,
) -> Result<Role, Halt> {
    let role = node.node.role_handle();
    let linked = tokio::task::spawn_blocking(move || {
        role.wait_for(LINK_WITHIN, |phase| {
            matches!(
                phase,
                crate::node_role::Phase::Live(_)
                    | crate::node_role::Phase::Waiting {
                        reason: crate::node_role::LinkFailure::Refused(_)
                    }
            )
        })
    });
    let Some(phase) = unless_stopped(&seat.stop, linked).await else {
        stop_node(node).await;
        return Err(Halt::Stopped);
    };
    let phase = phase.map_err(|error| error.to_string())?;
    if matches!(phase, crate::node_role::Phase::Live(_)) {
        commit(env, &seat.moves, journal, &remote).await;
        return Ok(Role::Node(node));
    }
    log(
        "link.unanswered",
        json!({"intent": journal.intent, "phase": format!("{phase:?}")}),
    );
    resolve_unlinked(env, seat, journal, remote, Some(node)).await
}

/// After the link carrying the intent may have been sent and none was
/// seen taken: the peer's handover decides, and while the peer cannot be
/// reached this machine stays a node and asks again.
async fn resolve_unlinked(
    env: &Env,
    seat: &seat::Seat,
    journal: Journal,
    remote: Arc<Remote>,
    mut node: Option<RunningNode>,
) -> Result<Role, Halt> {
    let waited = offering_abandon(
        env,
        &seat.moves,
        &journal,
        wait_on_peer(
            &seat.stop,
            &seat.moves,
            "status.failed",
            &journal.intent,
            Permanent::AskedAgain,
            {
                let (remote, journal) = (Arc::clone(&remote), journal.clone());
                move || driver::target_status(&remote, &journal)
            },
            |_| false,
        ),
    )
    .await;
    let said = match waited {
        Err(Halt::Stopped) => {
            if let Some(node) = node {
                stop_node(node).await;
            }
            return Err(Halt::Stopped);
        }
        Ok(Offered::Abandoned) => return abandon_here(env, seat, journal, node).await,
        Ok(Offered::Waited(said)) => said,
        Err(halt) => return Err(halt),
    }
    .map_err(|failure| {
        format!(
            "the move {} stopped asking its peer: {failure:?}",
            journal.intent
        )
    })?;
    match said {
        TargetSays::Active => {
            let node = match node {
                Some(node) => node,
                None => crate::start_node_role(env.clone(), seat.parts()).await?,
            };
            commit(env, &seat.moves, journal, &remote).await;
            Ok(Role::Node(node))
        }
        TargetSays::NotCommitted => {
            // The node's link stops before the peer is asked to stop, so no
            // link of it can commit the move meanwhile; the abort itself
            // answers if one did.
            if let Some(node) = node.take() {
                let _ = tokio::task::spawn_blocking(move || drop(node)).await;
            }
            rollback(
                env,
                seat,
                journal,
                &remote,
                MoveStep::Reattach,
                MoveFailure::LinkRefused {
                    reason: "the new core did not take this machine's link".to_owned(),
                },
            )
            .await
        }
    }
}

/// What a wait that offered an abandon ended with.
enum Offered<T> {
    Waited(Result<T, MoveFailure>),
    Abandoned,
}

/// Process-wide: told when a move is given up, so the asking that ends it
/// on its machine starts at once (`end_abandoned`).
static ABANDONED: std::sync::LazyLock<tokio::sync::Notify> =
    std::sync::LazyLock::new(tokio::sync::Notify::new);

/// Runs `wait`, a wait on the peer of the forward move `intent` after the
/// link carrying it may have been sent, offering the operator to give the
/// move up meanwhile (amendment 4) while the record of moves given up has
/// room. An answer and an abandon at once go with the answer, which says
/// what is so.
async fn offering_abandon<T>(
    env: &Env,
    moves: &MoveControl,
    journal: &Journal,
    wait: impl std::future::Future<Output = Result<Result<T, MoveFailure>, Halt>>,
) -> Result<Offered<T>, Halt> {
    let intent = journal.intent.as_str();
    let room = abandoned::has_room(&env.state_dir).unwrap_or_else(|reason| {
        log(
            "abandon.unoffered",
            json!({"intent": intent, "reason": reason}),
        );
        false
    });
    if !room {
        return wait.await.map(Offered::Waited);
    }
    // A move resumed after a restart names its machine to the window
    // only from here.
    moves.update(|view| {
        view.direction = Some(journal.phase.direction());
        view.device = Some(journal.peer.device.clone());
        view.alias = Some(journal.peer.alias.clone());
        view.intent = Some(journal.intent.clone());
    });
    let offer = moves.offer_abandon(intent);
    let ended = tokio::select! {
        biased;
        waited = wait => waited.map(Offered::Waited),
        () = moves.abandoned(&offer) => Ok(Offered::Abandoned),
    };
    moves.end_abandon_offer();
    ended
}

/// The operator gave up a forward move whose peer had not said whether it
/// committed (amendment 4): this machine's core starts again on its own
/// folder, which the move left as it found it, and the peer's core of the
/// move, if one runs, is ended once this machine reaches it
/// (`end_abandoned`). Recording the move as given up is the decision: a
/// start after it finishes this rather than waiting on the peer again.
async fn abandon_here(
    env: &Env,
    seat: &seat::Seat,
    mut journal: Journal,
    node: Option<RunningNode>,
) -> Result<Role, Halt> {
    log(
        "move.abandoned",
        json!({"intent": journal.intent, "peer": journal.peer.node}),
    );
    // No link carrying the intent leaves this machine from here.
    if let Some(node) = node {
        stop_node(node).await;
    }
    abandoned::add(
        &env.state_dir,
        Abandoned {
            intent: journal.intent.clone(),
            peer: journal.peer.clone(),
            step: PeerStep::Abandon,
        },
    )?;
    ABANDONED.notify_one();
    until_removed(seat, &journal.intent, "abandon.unfinished", || {
        placement::remove(&env.state_dir)
    })
    .await?;
    let staging = node_migration::staging_dir(&env.state_dir, &journal.intent);
    if staging.exists()
        && let Err(error) = std::fs::remove_dir_all(&staging)
    {
        // What is left is named by the next move's check.
        log(
            "abandon.unclean",
            json!({"intent": journal.intent, "reason": error.to_string()}),
        );
    }
    journal.phase = Phase::Forward(ForwardPhase::Abandoned);
    if let Err(reason) = journal::write(&env.state_dir, &journal) {
        // The record of moves given up still holds it, so a start finishes
        // this again.
        log(
            "abandon.unrecorded",
            json!({"intent": journal.intent, "reason": reason}),
        );
    }
    let running = crate::start_core_role(env.clone(), seat.parts()).await?;
    seat.moves.set(MoveView::of(&journal, MoveState::Abandoned));
    Ok(Role::Core(running))
}

/// Asks the machine of each move whose wait was given up here for the step
/// it still owes until it answers that the step is done (amendment 4):
/// every round asks every move the record holds; while one is not done the
/// next round waits, longer each time up to a minute, and a move given up
/// meanwhile starts one at once. It ends with the process's stop.
async fn end_abandoned(env: Env, stop: crate::ending::Ending) {
    let mut slower = crate::backoff::Backoff::default();
    // The last failure logged for each move, so a repeat is not.
    let mut last: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    loop {
        let mut pending = false;
        let moves = abandoned::read(&env.state_dir).unwrap_or_else(|reason| {
            pending = true;
            if last.get("") != Some(&reason) {
                log("abandon.unreadable", json!({"reason": reason}));
                last.insert(String::new(), reason);
            }
            Vec::new()
        });
        last.retain(|intent, _| {
            intent.is_empty() || moves.iter().any(|kept| kept.intent == *intent)
        });
        for kept in moves {
            let asked = {
                let (env, kept) = (env.clone(), kept.clone());
                unless_stopped(
                    &stop,
                    tokio::task::spawn_blocking(move || end_on_peer(&env, &kept)),
                )
                .await
            };
            let Some(asked) = asked else {
                return;
            };
            let failure = match asked.map_err(|error| error.to_string()) {
                Ok(Ok(answer)) => {
                    log(
                        "abandon.ended",
                        json!({"intent": kept.intent, "peer": kept.peer.node, "step": kept.step, "answer": answer}),
                    );
                    match abandoned::remove(&env.state_dir, &kept.intent) {
                        Ok(()) => continue,
                        Err(reason) => reason,
                    }
                }
                Ok(Err(failure)) => format!("{failure:?}"),
                Err(error) => error,
            };
            pending = true;
            if last.get(&kept.intent) != Some(&failure) {
                log(
                    "abandon.failed",
                    json!({"intent": kept.intent, "peer": kept.peer.node, "failure": failure}),
                );
                last.insert(kept.intent.clone(), failure);
            }
        }
        let wait = if pending {
            Some(slower.failed())
        } else {
            slower.reset();
            None
        };
        tokio::select! {
            () = stop.ended() => return,
            () = ABANDONED.notified() => slower.reset(),
            () = async {
                match wait {
                    Some(wait) => tokio::time::sleep(wait).await,
                    None => std::future::pending().await,
                }
            } => {}
        }
    }
}

/// Has the peer of the move given up as `kept` run the step it owes, and
/// answers what it said, always with this machine's own build: the peer
/// may run another build by then, whose step would act before its answer
/// is refused as another build's, and its helper folder may have been
/// replaced. So this build is installed under the peer's helper root
/// first, which reuses what is already there, and the record names that
/// `hided` from then on.
fn end_on_peer(env: &Env, kept: &Abandoned) -> Result<&'static str, MoveFailure> {
    let build = own_build(env).map_err(|reason| MoveFailure::Local { reason })?;
    let peer = &kept.peer;
    let remote = |program: &str| {
        Remote::new(&env.home, &peer.alias, program, &build).with_state_dir(&peer.state_dir)
    };
    let packages = crate::core_move::starter::this_program()
        .ok()
        .and_then(|program| program.parent().map(Path::to_path_buf))
        .ok_or_else(|| MoveFailure::Local {
            reason: "this hided carries no build to install".to_owned(),
        })?;
    let program = remote(&peer.program).install_build(&packages)?;
    if program != peer.program {
        log(
            "abandon.installed",
            json!({"intent": kept.intent, "was": peer.program, "program": program}),
        );
        abandoned::set_program(&env.state_dir, &kept.intent, &program)
            .map_err(|reason| MoveFailure::Local { reason })?;
    }
    let remote = remote(&program);
    match kept.step {
        PeerStep::Abandon => match driver::abandon_target(&remote, &kept.intent)? {
            driver::Abandoned::Ended => Ok("ended"),
            // No core of the given-up move runs beside another move's.
            driver::Abandoned::OtherMove { .. } => Ok("other_move"),
        },
        PeerStep::Retire => back::retire(&remote, &kept.intent).map(|leftover| {
            if let Some(reason) = leftover {
                log(
                    "retire.leftover",
                    json!({"intent": kept.intent, "peer": kept.peer.node, "reason": reason}),
                );
            }
            "retired"
        }),
    }
}

/// The move committed: this machine's brain state is set aside and the
/// peer's records of the move are cleared; a step that fails here is done
/// on the next start, since the journal stays `Committed`.
async fn commit(env: &Env, moves: &MoveControl, mut journal: Journal, remote: &Arc<Remote>) {
    journal.phase = Phase::Forward(ForwardPhase::Committed);
    if let Err(reason) = journal::write(&env.state_dir, &journal) {
        log(
            "commit.unrecorded",
            json!({"intent": journal.intent, "reason": reason}),
        );
    }
    log(
        "move.committed",
        json!({"intent": journal.intent, "target": journal.peer.node}),
    );
    let finished = {
        let (state_dir, settings, remote, journal) = (
            env.state_dir.clone(),
            herdr_core::hide_ai_settings_path(&env.home),
            Arc::clone(remote),
            journal.clone(),
        );
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            copy::set_aside(&state_dir, &journal.intent, &settings)
                .map_err(|refusal| refusal.to_string())?;
            let staging = node_migration::staging_dir(&state_dir, &journal.intent);
            if staging.exists() {
                std::fs::remove_dir_all(&staging).map_err(|error| error.to_string())?;
            }
            driver::finish_target(&remote, &journal).map_err(|failure| format!("{failure:?}"))?;
            let node = herdr_core::node::NodeId::of_this_machine()?;
            placement::update(&state_dir, node.as_str(), |placement| {
                placement.move_intent = None;
            })?;
            Ok(())
        })
        .await
        .map_err(|error| error.to_string())
        .and_then(|done| done)
    };
    match finished {
        Ok(()) => {
            journal.phase = Phase::Forward(ForwardPhase::Done);
            if let Err(reason) = journal::write(&env.state_dir, &journal) {
                log(
                    "done.unrecorded",
                    json!({"intent": journal.intent, "reason": reason}),
                );
            }
            log("move.done", json!({"intent": journal.intent}));
        }
        Err(reason) => log(
            "commit.unfinished",
            json!({"intent": journal.intent, "reason": reason}),
        ),
    }
    moves.set(MoveView {
        node: Some(journal.change.old_owner.clone()),
        ..MoveView::of(&journal, MoveState::Done)
    });
}

/// Undoes a move that did not commit: the peer's pending core is stopped
/// and its copy taken back first, then this machine's core starts again on
/// its untouched folder. A peer that cannot be reached is asked again until
/// its lease has surely ended.
async fn rollback(
    env: &Env,
    seat: &seat::Seat,
    mut journal: Journal,
    remote: &Arc<Remote>,
    step: MoveStep,
    cause: MoveFailure,
) -> Result<Role, Halt> {
    log(
        "move.rolling_back",
        json!({"intent": journal.intent, "step": step, "cause": cause}),
    );
    seat.moves.set(MoveView {
        step: Some(step),
        cause: Some(cause.clone()),
        ..MoveView::of(&journal, MoveState::RollingBack)
    });
    let screen = MoveScreen::mount(&seat.parts(), env.vite_origin.as_deref());
    let placed = matches!(
        journal.phase,
        Phase::Forward(
            ForwardPhase::Placed | ForwardPhase::TargetStarted | ForwardPhase::AttachSent
        )
    );
    if placed {
        let attach_sent = journal.phase == Phase::Forward(ForwardPhase::AttachSent);
        // Only a core that started holds a lease; a copy placed with no core
        // on it is given back by the next try's checks.
        let lease_until = journal.target_lease_until_unix_ms;
        let waiting = wait_on_peer(
            &seat.stop,
            &seat.moves,
            "abort.failed",
            &journal.intent,
            if attach_sent {
                Permanent::AskedAgain
            } else {
                Permanent::Ends
            },
            {
                let (remote, journal) = (Arc::clone(remote), journal.clone());
                move || driver::abort_target(&remote, &journal)
            },
            |_| !attach_sent && lease_until.is_none_or(|until| now_unix_ms() > until),
        );
        // Once the link carrying the intent may have been sent, only the
        // peer says whether the move committed, and the operator may give
        // the wait up as in `resolve_unlinked`.
        let aborted = if attach_sent {
            match offering_abandon(env, &seat.moves, &journal, waiting).await? {
                Offered::Waited(aborted) => aborted,
                Offered::Abandoned => {
                    drop(screen);
                    return abandon_here(env, seat, journal, None).await;
                }
            }
        } else {
            waiting.await?
        };
        match aborted {
            Ok(driver::Aborted::Undone) => {}
            Ok(driver::Aborted::CopyLeft { reason }) => {
                // Nothing of the move runs or starts there; its copy is
                // named by that machine's next check.
                log(
                    "rollback.unclean",
                    json!({"intent": journal.intent, "peer": reason}),
                );
            }
            Ok(driver::Aborted::Active) => {
                // The link committed the move after all: go forward.
                drop(screen);
                let node = crate::start_node_role(env.clone(), seat.parts()).await?;
                commit(env, &seat.moves, journal, remote).await;
                return Ok(Role::Node(node));
            }
            // No link carrying the intent left this machine, so no core of
            // the move can commit there: a pending one gives its copy back
            // at its lease's end, and a copy with no core on it is given
            // back by the next try's checks.
            Err(failure) => log(
                "abort.given_up",
                json!({"intent": journal.intent, "failure": failure}),
            ),
        }
    }
    // The core starts here only once no placement says it runs there; the
    // journal keeps its phase until then, so a start meanwhile rolls back
    // again.
    until_removed(seat, &journal.intent, "rollback.unfinished", || {
        placement::remove(&env.state_dir)
    })
    .await?;
    let staging = node_migration::staging_dir(&env.state_dir, &journal.intent);
    if staging.exists()
        && let Err(error) = std::fs::remove_dir_all(&staging)
    {
        // What is left is named by the next move's check.
        log(
            "rollback.unclean",
            json!({"intent": journal.intent, "reason": error.to_string()}),
        );
    }
    journal.phase = Phase::Forward(ForwardPhase::RolledBack {
        failed: step,
        cause: cause.clone(),
    });
    journal::write(&env.state_dir, &journal)?;
    drop(screen);
    let running = crate::start_core_role(env.clone(), seat.parts()).await?;
    log(
        "move.rolled_back",
        json!({"intent": journal.intent, "step": step}),
    );
    seat.moves.set(MoveView {
        step: Some(step),
        cause: Some(cause),
        ..MoveView::of(&journal, MoveState::RolledBack)
    });
    Ok(Role::Core(running))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pending lease ends at the deadline its record holds, however
    /// often the core's loop asked again meanwhile; before, each request
    /// started a fresh lease.
    #[tokio::test]
    async fn the_pending_lease_ends_at_its_recorded_deadline() {
        let dir = tempfile::tempdir().unwrap();
        handover::hold(dir.path())
            .unwrap()
            .write(&Handover::new(
                "i1",
                "m",
                "c",
                HandoverState::Pending {
                    lease_until_unix_ms: now_unix_ms() + 300,
                },
            ))
            .unwrap();
        let gate = crate::core_move::gate::MoveGate::for_start(dir.path()).unwrap();
        for _ in 0..3 {
            let _ = tokio::time::timeout(Duration::from_millis(20), lease_end(&gate)).await;
        }
        tokio::time::timeout(Duration::from_secs(5), lease_end(&gate))
            .await
            .expect("the lease did not end at its deadline");
        assert_eq!(gate.expire(now_unix_ms()), Some("i1".to_owned()));
    }

    /// A pending core whose lease ended gives its copy back and then
    /// forgets its login item, so no login starts an empty core on the
    /// folder; a record of another move is left with its item.
    #[cfg(unix)]
    #[test]
    fn a_core_that_gives_its_copy_back_forgets_its_login_item() {
        use hide_platform::user_agents::{LoginAgent, UserAgents};
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::AtomicBool;
        let home = tempfile::tempdir().unwrap();
        let state = home.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        handover::hold(&state)
            .unwrap()
            .write(&Handover::new(
                "i1",
                "m",
                "c",
                HandoverState::Pending {
                    lease_until_unix_ms: now_unix_ms(),
                },
            ))
            .unwrap();
        // Nothing is loaded; a bootstrap loads nothing either.
        let launchctl = home.path().join("launchctl");
        std::fs::write(
            &launchctl,
            "#!/bin/sh\n[ \"$1\" = print ] && exit 113\nexit 0\n",
        )
        .unwrap();
        std::fs::set_permissions(&launchctl, std::fs::Permissions::from_mode(0o755)).unwrap();
        let agents = UserAgents::fixture(home.path());
        let label = hide_kit::layout::core_login_item(home.path(), &state);
        agents
            .install(
                &LoginAgent {
                    label: &label,
                    program: Path::new("/usr/bin/true"),
                    arguments: &[],
                    environment: &[],
                    log: &home.path().join("core.log"),
                },
                home.path(),
                &AtomicBool::new(false),
            )
            .unwrap();
        let installed = || agents.is_installed(&label, home.path()).unwrap();

        give_back_pending(&state, home.path(), &agents, "i2");
        assert!(installed(), "another move's item was forgotten");
        give_back_pending(&state, home.path(), &agents, "i1");
        assert!(handover::read(&state).unwrap().is_none());
        assert!(!installed(), "the login item is still installed");
    }

    /// A release its asker gave up on never stops the core: one given up
    /// while it waited in line behind a check is dropped before anything is
    /// written, and one given up while its stop was being written is undone,
    /// leaving no handover record or export. The core stops only for a
    /// release it took while its asker still waited, which hears the answer
    /// and can no longer give it up.
    #[tokio::test]
    async fn a_release_its_asker_gave_up_on_never_stops_the_core() {
        let home = tempfile::tempdir().unwrap();
        let state = home.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let release = || {
            let (reply, answer) = std::sync::mpsc::channel();
            let release = Release {
                intent: "i1".to_owned(),
                target: "mbp".to_owned(),
                claim: Arc::default(),
                reply,
            };
            (release, answer)
        };
        // What `release_core` writes.
        let record = |state: &Path| {
            handover::hold(state)
                .unwrap()
                .write(&Handover::new(
                    "i1",
                    "mini",
                    "mbp",
                    HandoverState::StoppedFor,
                ))
                .unwrap();
            let export = back::export_path(state, "i1");
            std::fs::create_dir_all(export.parent().unwrap()).unwrap();
            std::fs::write(&export, "{}").unwrap();
        };

        let (in_line, _answer) = release();
        assert!(in_line.claim.give_up());
        let written = std::cell::Cell::new(false);
        let stops = answer_release(&state, &in_line, async {
            written.set(true);
            Ok(())
        })
        .await;
        assert!(
            !stops && !written.get(),
            "a release given up in line was acted on"
        );

        let (late, _answer) = release();
        let stops = answer_release(&state, &late, async {
            record(&state);
            assert!(late.claim.give_up());
            Ok(())
        })
        .await;
        assert!(
            !stops,
            "the core stops for a release given up while it was written"
        );
        assert_eq!(handover::read(&state).unwrap(), None);
        assert!(
            !back::export_path(&state, "i1").exists(),
            "the export is left"
        );

        let (asked, answer) = release();
        let stops = answer_release(&state, &asked, async {
            record(&state);
            Ok(())
        })
        .await;
        assert!(stops);
        assert_eq!(answer.try_recv().unwrap(), Ok(()));
        assert!(
            !asked.claim.give_up(),
            "the asker gave up a release the core took"
        );
        assert_eq!(
            handover::read(&state).unwrap().unwrap().state,
            HandoverState::StoppedFor
        );
    }

    /// A stop ends a wait on the peer that would ask forever, both between
    /// two asks and while one is under way.
    #[tokio::test]
    async fn a_stop_ends_a_wait_on_the_peer() {
        for asking_blocks in [false, true] {
            let stop = crate::ending::Ending::new();
            // An ask that blocks answers only once this is dropped.
            let (answer, answered) = std::sync::mpsc::channel::<()>();
            let answered = std::sync::Mutex::new(answered);
            let waiting = {
                let stop = stop.clone();
                tokio::spawn(async move {
                    wait_on_peer(
                        &stop,
                        &MoveControl::default(),
                        "status.failed",
                        "i1",
                        Permanent::AskedAgain,
                        move || -> Result<(), MoveFailure> {
                            if asking_blocks {
                                let _ = answered.lock().unwrap().recv();
                            }
                            Err(MoveFailure::Unreachable {
                                reason: "down".to_owned(),
                            })
                        },
                        |_| false,
                    )
                    .await
                })
            };
            tokio::time::sleep(Duration::from_millis(50)).await;
            stop.end();
            let ended = tokio::time::timeout(Duration::from_secs(1), waiting)
                .await
                .expect("the wait did not hear the stop")
                .unwrap();
            assert!(matches!(ended, Err(Halt::Stopped)), "{ended:?}");
            drop(answer);
        }
    }

    /// A failure asking again cannot change ends a wait before the link
    /// that carries the intent was sent, rather than holding the core down
    /// on both machines; after it, only the peer's answer decides, so it is
    /// asked again, ever more slowly. A transient failure is asked again at
    /// once unless the caller gave it up (a lease that ended).
    #[test]
    fn a_wait_asks_again_only_what_asking_again_can_change() {
        let refused = MoveFailure::Refused {
            step: "abort".to_owned(),
            reason: "the record is unreadable".to_owned(),
        };
        let busy = MoveFailure::Busy {
            step: "abort".to_owned(),
            reason: "held".to_owned(),
        };
        let unreachable = MoveFailure::Unreachable {
            reason: "down".to_owned(),
        };
        let mut slower = crate::backoff::Backoff::default();
        assert_eq!(
            next_ask(&refused, Permanent::Ends, false, &mut slower),
            None
        );
        for transient in [&busy, &unreachable] {
            assert_eq!(
                next_ask(transient, Permanent::Ends, false, &mut slower),
                Some(RETRY_EVERY)
            );
            assert_eq!(
                next_ask(transient, Permanent::Ends, true, &mut slower),
                None
            );
        }
        let waits: Vec<_> = (0..3)
            .map(|_| next_ask(&refused, Permanent::AskedAgain, false, &mut slower))
            .collect();
        assert_eq!(waits, [2, 4, 8].map(|secs| Some(Duration::from_secs(secs))));
    }
}
