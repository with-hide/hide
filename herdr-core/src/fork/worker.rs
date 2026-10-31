//! Running a fork through the engine: the new pane and its tab, the agent
//! started on the parent's conversation, both executions registered, and
//! each agent's own way of making the fork.

use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use crate::engine::Engine;
use serde_json::json;

use crate::checkout_owner::open::create_tab_in;
use crate::fork::{ForkPlacement, ForkRequest};
use crate::live::{
    CONTROL_WAIT, LiveContext, confirm_session_launch, current_session, failed, fetch_session,
    start_agent_checked, start_error_message,
};
use crate::runtime::Runtime;

pub fn spawn_agent_fork(context: LiveContext, request: ForkRequest) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-agent-fork".to_owned())
        .spawn(move || {
            // Includes pane creation and the start acknowledgment, so this
            // input wait never extends Herdr's 120-second start reservation.
            let input_deadline =
                Instant::now() + crate::agent_start::AGENT_START_DEADLINE - Duration::from_secs(5);
            finish_fork_worker(&context, &request.parent_pane_id, || {
                let pane = run_agent_fork_with_registration(
                    context.engine.as_ref(),
                    context.node.as_ref(),
                    &request,
                    &|| fork_execution_current(&context, &request),
                    |tab_id, pane_id| {
                        let Some(runtime) = context.runtime.upgrade() else {
                            return;
                        };
                        let changed = match runtime.lock() {
                            Ok(mut guard) => {
                                guard.ingest_fork_pane(&request.parent_pane_id, tab_id, pane_id)
                            }
                            Err(_) => return,
                        };
                        drop(runtime);
                        if changed {
                            context.notifier.notify();
                        }
                    },
                    |parent, child| {
                        fork_execution_current(&context, &request)?;
                        crate::coordination::link_fork(&context, parent, child, &request.name)
                    },
                )?;
                // The fork already succeeded. Readiness owns only its held
                // input and must never close a slow but working pane.
                wait_for_fork_input(&context, &request, &pane, input_deadline);
                Ok(pane)
            });
        })
        .map(|_| ())
        .map_err(|error| format!("fork worker could not be started: {error}"))
}

fn wait_for_fork_input(
    context: &LiveContext,
    request: &ForkRequest,
    pane: &str,
    deadline: Instant,
) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::agent_start::confirm_fork_started(
            context.engine.as_ref(),
            pane,
            &request.name,
            request.agent.kind(),
            &|| fork_input_owner_alive(&context.runtime),
            deadline,
        )
    }))
    .unwrap_or_else(|_| Err("fork input confirmation panicked".into()));
    if let Err(reason) = outcome {
        crate::diagnostic!(serde_json::json!({
            "component": "terminal_input", "kind": "terminal.fork_input_unconfirmed",
            "pane_id": pane, "reason": reason,
        }));
        if let Some(runtime) = context.runtime.upgrade() {
            runtime
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .discard_fork_input(&request.parent_pane_id);
            context.notifier.notify();
        }
    }
}

fn fork_input_owner_alive(owner: &Weak<Mutex<Runtime>>) -> Result<(), String> {
    if owner.strong_count() == 0 {
        Err("The fork's daemon stopped.".into())
    } else {
        Ok(())
    }
}

/// Every worker exit settles its input holder, including an unwinding provider
/// or connector. Runtime teardown settles it if the weak owner has ended.
pub(crate) fn finish_fork_worker(
    context: &LiveContext,
    parent_pane_id: &str,
    work: impl FnOnce() -> Result<String, String>,
) {
    let started = Instant::now();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
        .unwrap_or_else(|_| Err("fork worker panicked".into()));
    let Some(runtime) = context.runtime.upgrade() else {
        return;
    };
    let changed = runtime
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .ingest_fork_result(parent_pane_id, result, started.elapsed().as_millis());
    if changed {
        context.notifier.notify();
    }
}

fn fork_execution_current(context: &LiveContext, request: &ForkRequest) -> Result<(), String> {
    let runtime = context.runtime.upgrade().ok_or("runtime stopped")?;
    if !hide_session::Agent::from_kind(request.agent.kind())
        .is_some_and(hide_session::Agent::requires_native_proof)
    {
        return Ok(());
    }
    let runtime = runtime.lock().map_err(|_| "runtime unavailable")?;
    if !runtime.fork_request_is_current(request, &context.engine)
        || !Arc::ptr_eq(&runtime.own_node(), &context.node)
    {
        return Err("session_fork_execution_changed".into());
    }
    Ok(())
}

/// Starts a fork where the operator asked for it, in its own tab or beside
/// the source. Herdr makes the pane without moving the keyboard; `created`
/// hands it to the core, which moves the keyboard to it and places its tab.
/// A pane whose agent never started is closed; registration never turns a
/// working fork into a failure.
fn run_agent_fork_with_registration(
    engine: &dyn Engine,
    node: &dyn crate::node_access::NodeLink,
    request: &ForkRequest,
    current: &(dyn Fn() -> Result<(), String> + Sync),
    created: impl FnOnce(Option<&str>, &str),
    register: impl FnOnce(&str, &str) -> Result<(), String>,
) -> Result<String, String> {
    let checked = current_session(engine, current);
    let proven = hide_session::Agent::from_kind(request.agent.kind())
        .is_some_and(hide_session::Agent::requires_native_proof);
    let mutation_engine: &dyn Engine = if proven { checked.as_ref() } else { engine };
    let check = || {
        current()?;
        confirm_session_launch(
            node,
            request.agent.kind(),
            &request.session_id,
            request.cwd.as_deref(),
            request.source_reference.as_ref(),
        )?;
        current()
    };
    check()?;
    // The arguments are settled before any pane exists, so a choice the agent
    // does not take never leaves a pane to close. The choice is checked before
    // the fork is made, so a refused one never leaves a session an OpenCode 2
    // server made either.
    let choice =
        crate::runtime::agent_choice::launch_arguments(
            request.agent.kind(),
            &request.launch,
            &request.permission,
        )?;
    let (mut args, made) = fork_arguments(node, request)?;
    args.extend(choice);
    // Everything after an OpenCode 2 fork can still fail (the parent's tab,
    // the new tab, the agent's start, or the parent closed meanwhile), and the
    // session OpenCode's server made stays in OpenCode's list: it is named in
    // the reason and a diagnostic for the operator to find, never deleted.
    let mut made_pane = None;
    let opened = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<String, String> {
        // A split takes no command, so only a fork in its own tab runs the
        // integration's launch.
        let launch = matches!(request.placement, ForkPlacement::Tab)
            .then(|| {
                crate::agent_start::shell_launch(
                    Some(node),
                    &format!("herdr-core:fork:{}:start", request.name),
                )
            })
            .flatten();
        let shell = crate::agent_start::ShellStart::of_tab(true, launch.as_ref());
        let child_pane_id = match request.placement {
            // `Fork agent`: the fork's own tab, which the core puts right of the
            // source's tab once Herdr has named it (`created`).
            ForkPlacement::Tab => {
                let cwd = request
                    .cwd
                    .as_deref()
                    .filter(|cwd| !cwd.trim().is_empty())
                    .ok_or("Session fork has no confirmed checkout")?;
                let snapshot = fetch_session(mutation_engine)
                    .map_err(|error| error.message().to_owned())?;
                let mut parents = snapshot.layouts.iter().filter(|layout| {
                    layout
                        .panes
                        .iter()
                        .any(|pane| pane.pane_id == request.parent_pane_id)
                });
                let parent = parents
                    .next()
                    .ok_or("Session fork parent is no longer in its workspace")?;
                if parents.next().is_some() {
                    return Err("Session fork parent has ambiguous workspace ownership".into());
                }
                let label = request
                    .tab_label
                    .as_deref()
                    .ok_or("Session fork has no checkout to name its tab in")?;
                check()?;
                let tab = create_tab_in(
                    mutation_engine,
                    &parent.workspace_id,
                    cwd,
                    label,
                    false,
                    launch
                        .as_ref()
                        .map(|launch| launch.env.clone())
                        .unwrap_or_default(),
                    launch.as_ref().map(|launch| launch.command.as_slice()),
                )
                .map_err(|error| error.message().to_owned())?;
                made_pane = Some(tab.pane_id.clone());
                created(Some(&tab.tab_id), &tab.pane_id);
                tab.pane_id
            }
            // `Fork agent to the side`: a split right of the source, in its tab.
            ForkPlacement::Side => {
                let pane_id = mutation_engine
                    .split_beside(
                        CONTROL_WAIT,
                        &request.parent_pane_id,
                        request.cwd.as_deref(),
                        Default::default(),
                    )
                    .map_err(failed("pane.split"))?;
                made_pane = Some(pane_id.clone());
                created(None, &pane_id);
                pane_id
            }
        };
        let started = start_agent_checked(
            mutation_engine,
            Some(node),
            &format!("herdr-core:fork:{}:start", request.name),
            &child_pane_id,
            shell,
            &request.name,
            request.agent.kind(),
            args,
            request.codex_daemon,
            &check,
        )
        .map_err(start_error_message);
        let started = started.and_then(|pane_id| {
            if request.agent == crate::fork::ForkableAgent::Antigravity {
                fork_antigravity(mutation_engine, node, request, &pane_id)?;
            }
            Ok(pane_id)
        });
        match started {
            Ok(pane_id) => {
                if let Err(message) = register(&request.parent_pane_id, &pane_id) {
                    crate::diagnostic!(serde_json::json!({
                        "component": "fork",
                        "kind": "registration_failed",
                        "parent_pane_id": request.parent_pane_id,
                        "child_pane_id": pane_id,
                        "message": message,
                    }));
                }
                Ok(pane_id)
            }
            Err(error) => {
                // This child is already ours on the original server. Cleanup is
                // not a new fork effect and must not target its replacement.
                let closed = engine
                    .close_pane(CONTROL_WAIT, &child_pane_id)
                    .map_err(failed("pane.close"));
                Err(match closed {
                    Ok(_) => error,
                    Err(close_error) => format!(
                        "{error}; the created pane {child_pane_id} could not be closed: {close_error}"
                    ),
                })
            }
        }
    })).unwrap_or_else(|_| {
        if let Some(pane) = made_pane {
            // Closing can fail too, but must never prevent settling the hold.
            let closed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                engine
                    .close_pane(CONTROL_WAIT, &pane)
                    .map_err(failed("pane.close"))
            }));
            if !matches!(closed, Ok(Ok(_))) {
                crate::diagnostic!(json!({"component": "pane_fork", "kind": "pane.fork.close_failed", "pane_id": pane}));
            }
        }
        Err("fork worker panicked".into())
    });
    opened.map_err(|error| match made {
        Some(session) => opencode_fork_left(request, &session, error),
        None => error,
    })
}

/// The reason a fork whose OpenCode 2 session was made did not open, naming
/// that session, and its diagnostic.
fn opencode_fork_left(request: &ForkRequest, session: &str, error: String) -> String {
    crate::diagnostic!(serde_json::json!({
        "component": "fork",
        "kind": "opencode_fork_left",
        "parent_pane_id": request.parent_pane_id,
        "session_id": session,
        "message": error,
    }));
    format!(
        "{error}; OpenCode made the fork's session {session}, which stays in OpenCode's session list"
    )
}

/// The agent's arguments that open the fork. Every agent but OpenCode spells
/// its own fork ([`crate::fork::ForkableAgent::resume_arguments`]). OpenCode's
/// depends on the version holding the session, which its node reads from the
/// database (`hide_session::opencode::fork_route`): 1.x forks with its own
/// flag, and 2.x's server makes the fork here, once, before the fork's pane
/// exists, so every later step of this fork, a retried start included, opens
/// that one session; the session it made comes back beside the arguments. A
/// node that cannot fork it ends the fork with its reason before any pane is
/// made, and one whose answer never came says a fork may exist.
fn fork_arguments(
    node: &dyn crate::node_access::NodeLink,
    request: &ForkRequest,
) -> Result<(Vec<String>, Option<String>), String> {
    if request.agent != crate::fork::ForkableAgent::OpenCode {
        return Ok((request.agent.resume_arguments(&request.session_id), None));
    }
    let cwd = request
        .cwd
        .as_deref()
        .ok_or("Session fork has no confirmed checkout")?;
    // Busy answers only a route read that met OpenCode writing its
    // database, before the node forked anything, so it is asked again.
    let fork: hide_node_link::protocol::OpenCodeFork = read_past_writes(
        node,
        || hide_node_link::protocol::Call::OpenCodeFork {
            source: request.session_id.clone(),
            cwd: cwd.to_owned(),
        },
        hide_node_link::protocol::OPENCODE_FORK_TIMEOUT,
        Instant::now() + Duration::from_secs(10),
    )
    .map_err(|error| match error {
        crate::node_access::LinkError::Unknown(_) => format!(
            "OpenCode could not confirm the fork: {error}; a fork may exist in OpenCode's session list"
        ),
        error => format!("OpenCode could not fork the session: {error}"),
    })?;
    match fork {
        hide_node_link::protocol::OpenCodeFork::Flag => {
            Ok((request.agent.resume_arguments(&request.session_id), None))
        }
        hide_node_link::protocol::OpenCodeFork::Forked { id } => {
            Ok((crate::fork::opencode_forked_arguments(&id)?, Some(id)))
        }
    }
}

/// How long agy gets to come up in the fork's pane, and then to write the
/// conversation its `/fork` made.
const ANTIGRAVITY_FORK_WAIT: Duration = Duration::from_secs(30);
const ANTIGRAVITY_FORK_POLL: Duration = Duration::from_millis(250);
/// Backspaces sent before each read of agy's composer while it is cleared
/// for `/fork`, and the most sent in all: a read says whether the composer
/// is empty, not how much it holds, and the stray answer it clears is a few
/// characters (`27;1$y`).
const ANTIGRAVITY_ERASE_BATCH: usize = 8;
const ANTIGRAVITY_ERASE_LIMIT: usize = 64;
/// The clearing's own bound, counted from its start: the waits before it
/// may have spent the fork's opening wait on a slow start, the case the
/// stray answer comes with.
const ANTIGRAVITY_CLEAR_WAIT: Duration = Duration::from_secs(10);
/// How many reads of an agent's records in a row may meet the agent writing
/// them before the reader gives up: 5 s of polls, far past one write.
const NATIVE_READ_IN_MOTION_LIMIT: usize = 20;
const NATIVE_READ_IN_MOTION_POLL: Duration = Duration::from_millis(250);

/// One read of an agent's native records on a node, read again while it
/// only met the agent writing them, which the node answers as busy
/// (`hide_session::ReadRefusal::InMotion`). A read that answers is one
/// consistent snapshot, so it needs no second read to agree with; the wait
/// ends at `deadline` or after [`NATIVE_READ_IN_MOTION_LIMIT`] reads in a
/// row, whichever is first, and answers the last read's busy refusal.
#[allow(clippy::disallowed_methods)] // a production wait, not test code: bounded by count and deadline
pub(crate) fn read_past_writes<T: serde::de::DeserializeOwned>(
    node: &dyn crate::node_access::NodeLink,
    call: impl Fn() -> hide_node_link::protocol::Call,
    timeout: Duration,
    deadline: Instant,
) -> Result<T, crate::node_access::LinkError> {
    let mut in_motion = 0;
    loop {
        match crate::node_access::call_as(node, call(), timeout) {
            Err(crate::node_access::LinkError::Refused(error))
                if error.code == hide_node_link::ErrorCode::Busy
                    && in_motion + 1 < NATIVE_READ_IN_MOTION_LIMIT
                    && Instant::now() < deadline =>
            {
                in_motion += 1;
                thread::sleep(NATIVE_READ_IN_MOTION_POLL);
            }
            answer => return answer,
        }
    }
}

/// agy forks only through its `/fork` command (agy 1.4.0, PRD
/// antigravity-first-class D-07). The fork's pane resumed the source
/// conversation; this types `/fork` once Herdr reads agy as idle there, finds
/// the conversation `/fork` made in agy's index on the pane's node, and
/// reports it to Herdr, which would keep the source's id until the next
/// prompt. Everything the fork assumes of agy's screen is in this function;
/// a failure leaves the source conversation as it was and the caller closes
/// the pane.
#[allow(clippy::disallowed_methods)] // a production wait, not test code: polls Herdr and agy's index, bounded by a deadline
fn fork_antigravity(
    engine: &dyn Engine,
    node: &dyn crate::node_access::NodeLink,
    request: &ForkRequest,
    pane_id: &str,
) -> Result<(), String> {
    let cwd = request
        .cwd
        .as_deref()
        .ok_or("Session fork has no confirmed checkout")?;
    let deadline = Instant::now() + ANTIGRAVITY_FORK_WAIT;
    while engine
        .pane_agent_status(CONTROL_WAIT, pane_id)
        .map_err(failed("pane.get"))?
        != "idle"
    {
        if Instant::now() >= deadline {
            return Err("agy did not open the conversation to fork in time".into());
        }
        thread::sleep(ANTIGRAVITY_FORK_POLL);
    }
    // What the checkout holds before /fork, read on the pane's node, so the
    // fork is the row that was not there: no clock is compared, and the
    // source may move on while agy writes it.
    let mark: hide_node_link::protocol::AntigravityForkMark = read_past_writes(
        node,
        || hide_node_link::protocol::Call::AntigravityForkMark {
            source: request.session_id.clone(),
            cwd: cwd.to_owned(),
        },
        Duration::from_secs(5),
        Instant::now() + ANTIGRAVITY_FORK_WAIT,
    )
    .map_err(|error| format!("agy's conversation to fork could not be read: {error}"))?;
    clear_antigravity_composer(engine, pane_id, Instant::now() + ANTIGRAVITY_CLEAR_WAIT)?;
    // The mark's read may have waited on agy writing its index, and the
    // composer's clearing on agy redrawing: `/fork` is typed only into a pane
    // still idle, never into a turn that began since.
    if engine
        .pane_agent_status(CONTROL_WAIT, pane_id)
        .map_err(failed("pane.get"))?
        != "idle"
    {
        return Err("agy started working before /fork could be typed".into());
    }
    engine
        .submit_text(CONTROL_WAIT, pane_id, "/fork")
        .map_err(failed("pane.send_input"))?;
    let deadline = Instant::now() + ANTIGRAVITY_FORK_WAIT;
    let forked = loop {
        let found: Option<String> = read_past_writes(
            node,
            || hide_node_link::protocol::Call::AntigravityFork {
                source: request.session_id.clone(),
                cwd: cwd.to_owned(),
                mark: mark.clone(),
            },
            Duration::from_secs(5),
            deadline,
        )
        .map_err(|error| format!("agy's fork could not be confirmed: {error}"))?;
        if let Some(id) = found {
            break id;
        }
        if Instant::now() >= deadline {
            return Err("agy's /fork made no new conversation".into());
        }
        thread::sleep(ANTIGRAVITY_FORK_POLL);
    };
    let seq = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX)
        });
    engine
        .report_agent_session(CONTROL_WAIT, pane_id, request.agent.kind(), &forked, seq)
        .map_err(failed("pane.report_agent_session"))
}

/// Empties agy's composer in the fork's pane before `/fork` is typed. Herdr's
/// terminal answers the mode query agy sends as it starts, and part of that
/// answer can stay in the composer as text (`27;1$y`, #1529); typed behind
/// it, `/fork` is a prompt and not the command. The pane is the one this
/// fork opened, so what its composer holds is erased, each erase judged by
/// the screen with agy's composer declaration, the one delivery reads a
/// held letter's composer by (`docs/delivery.md`, Reading the composer).
/// A screen the declaration cannot confirm empty (no composer as declared,
/// a cut screen, or text that outlasts the erase, as a hint a later agy
/// draws might) is typed into as before and logged, so a declaration that
/// no longer matches agy's screen never stops a fork. Bounded by
/// `deadline` and `ANTIGRAVITY_ERASE_LIMIT`.
#[allow(clippy::disallowed_methods)] // a production wait, not test code: agy redraws between erases
fn clear_antigravity_composer(
    engine: &dyn Engine,
    pane_id: &str,
    deadline: Instant,
) -> Result<(), String> {
    use hide_agent_adapter::composer::Verdict;
    let composer = hide_agent_adapter::adapter("agy")
        .and_then(|row| row.composer.supported())
        .ok_or("agy declares no composer to read before /fork")?;
    let mut sent = 0;
    let unconfirmed = |reason: &str, sent: usize| {
        crate::diagnostic!(serde_json::json!({
            "component": "pane_fork",
            "kind": "fork.composer_unconfirmed",
            "pane_id": pane_id,
            "agent": "agy",
            "reason": reason,
            "backspaces": sent,
        }));
        Ok(())
    };
    loop {
        let screen = engine
            .read_screen(CONTROL_WAIT, pane_id)
            .map_err(failed("pane.read"))?;
        // A cut screen may have lost the composer's rows.
        if screen.truncated {
            return unconfirmed("read_truncated", sent);
        }
        match composer.judge(&screen.text) {
            Verdict::Empty => return Ok(()),
            Verdict::NotFound => return unconfirmed("not_found", sent),
            Verdict::Holds if sent >= ANTIGRAVITY_ERASE_LIMIT || Instant::now() >= deadline => {
                return unconfirmed("not_empty", sent);
            }
            Verdict::Holds => {
                engine
                    .send_keys(
                        CONTROL_WAIT,
                        pane_id,
                        &["backspace"; ANTIGRAVITY_ERASE_BATCH],
                    )
                    .map_err(failed("pane.send_keys"))?;
                sent += ANTIGRAVITY_ERASE_BATCH;
                thread::sleep(ANTIGRAVITY_FORK_POLL);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use hide_herdr_client::HERDR_PROTOCOL_REVISION;

    use super::*;
    use crate::fake_herdr::FakeHerdr;

    #[test]
    fn a_native_fork_gets_its_own_unfocused_tab_and_failed_start_closes_only_its_pane() {
        for agent in [
            crate::fork::ForkableAgent::Pi,
            crate::fork::ForkableAgent::Omp,
        ] {
            native_fork_tab_journey(agent);
        }
    }

    fn native_fork_tab_journey(agent: crate::fork::ForkableAgent) {
        let kind = agent.kind();
        for case in [
            "success",
            "refused",
            "panic-created",
            "duplicate-before",
            "duplicate-after",
            "execution-after",
            "source-missing-before",
            "source-missing-after",
        ] {
            let refused = case == "refused";
            let home = tempfile::tempdir().unwrap();
            let cwd = hide_platform::fs::identity::canonical(home.path()).unwrap();
            let folder = crate::fixture::native_session_folder(home.path(), kind, &cwd);
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(
                folder.join("native.jsonl"),
                format!(
                    "{}\n",
                    json!({"type":"session", "version":3, "id":"native-pi", "cwd":cwd})
                ),
            )
            .unwrap();
            let source = folder.join("native.jsonl");
            let before = std::fs::read(&source).unwrap();
            let other_before = [before.as_slice(), b"{\"type\":\"message\",\"id\":\"other-message\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"different history\"}]}}\n"].concat();
            if case == "duplicate-before" {
                std::fs::copy(&source, folder.join("duplicate.jsonl")).unwrap();
            }
            if case == "source-missing-before" {
                std::fs::write(folder.join("other.jsonl"), &other_before).unwrap();
                std::fs::remove_file(&source).unwrap();
            }
            let changed = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let observed_changed = Arc::clone(&changed);
            // The fork's tab runs this account's integrated shell and
            // waits for its prompt token, independently of installed shells.
            let shell = crate::fixture::integrated_shell(home.path());
            let expected_shell = shell.shell.clone().unwrap();
            let node = hide_node::Local::new(Some(home.path().to_path_buf())).with_shell(shell);
            let expected_cwd = cwd.display().to_string();
            let tab_source = source.clone();
            let tab_other = other_before.clone();
            let herdr = FakeHerdr::start_with_errors("pi-fork-tab", move |method, params| {
                Ok(match method {
                    "session.snapshot" => json!({"type": "session_snapshot", "snapshot": {
                        "version": "fixture", "protocol": HERDR_PROTOCOL_REVISION,
                        "workspaces": [], "tabs": [], "panes": [], "agents": [],
                        "layouts": [{"workspace_id": "w1", "tab_id": "w1:t1", "zoomed": false,
                            "area": {"x":0,"y":0,"width":80,"height":24},
                            "focused_pane_id": "parent-pane",
                            "panes": [{"pane_id":"parent-pane", "focused":true,
                                "rect":{"x":0,"y":0,"width":80,"height":24}}], "splits": []}]
                    }}),
                    "layout.apply" => {
                        if case == "source-missing-after" {
                            std::fs::write(tab_source.with_file_name("other.jsonl"), &tab_other)
                                .unwrap();
                            std::fs::remove_file(&tab_source).unwrap();
                        }
                        if case == "duplicate-after" {
                            std::fs::copy(
                                &tab_source,
                                tab_source.with_file_name("duplicate.jsonl"),
                            )
                            .unwrap();
                        }
                        if case == "execution-after" {
                            observed_changed.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                        assert_eq!(params["workspace_id"], "w1");
                        assert_eq!(params["root"]["cwd"], expected_cwd);
                        assert_eq!(params["focus"], false);
                        // Named as a new tab, so the strip shows the agent;
                        // it runs the node's shell with the integration's
                        // launch.
                        assert_eq!(params["tab_label"], "Tab 2");
                        assert_eq!(params["root"]["command"][0], expected_shell);
                        json!({"type":"layout_apply","layout":{"workspace_id":"w1","tab_id":"w1:t2",
                            "zoomed":false,"focused_pane_id":"child-pane",
                            "root":{"type":"pane","pane_id":"child-pane"}}})
                    }
                    "pane.process_info" => json!({"type":"pane_process_info","process_info":{
                        "pane_id":"child-pane","shell_pid":42,"foreground_process_group_id":42,
                        "foreground_processes":[{"pid":42,"name":"zsh"}]}}),
                    "pane.get" => {
                        assert_eq!(params["pane_id"], "child-pane");
                        json!({"type":"pane_info","pane":{
                            "pane_id":"child-pane","terminal_id":"child-terminal","workspace_id":"w1","tab_id":"w1:t2","focused":false,"agent_status":"unknown","revision":1,
                            "tokens":{"hide_prompt":"1"}}})
                    }
                    "agent.start" => {
                        assert_eq!(params["pane_id"], "child-pane");
                        assert_eq!(params["kind"], kind);
                        assert_eq!(params["args"], json!(["--fork", "native-pi"]));
                        if refused {
                            return Err(("start_refused".into(), "fixture refused".into()));
                        }
                        json!({"type":"agent_started","argv":[],"agent":{
                            "pane_id":"child-pane","terminal_id":"child-terminal","workspace_id":"w1","tab_id":"w1:t2","focused":false,"agent_status":"idle","revision":2}})
                    }
                    "pane.close" => {
                        assert_eq!(params["pane_id"], "child-pane");
                        json!({"type":"ok"})
                    }
                    other => panic!("unexpected {other}"),
                })
            });
            let request = ForkRequest {
                source_reference: Some(crate::sidebar::SessionAgentSessionPayload {
                    kind: "path".into(),
                    value: source.display().to_string(),
                }),
                connection_generation: 0,
                codex_daemon: Default::default(),
                parent_pane_id: "parent-pane".into(),
                agent,
                session_id: "native-pi".into(),
                cwd: Some(cwd.display().to_string()),
                name: "fork-pi-1".into(),
                tab_label: Some("Tab 2".into()),
                placement: ForkPlacement::Tab,
                launch: Default::default(),
                permission: Default::default(),
            };
            let created = std::cell::RefCell::new(Vec::new());
            let result = run_agent_fork_with_registration(
                herdr.engine().as_ref(),
                &node,
                &request,
                &|| {
                    if changed.load(std::sync::atomic::Ordering::SeqCst) {
                        Err("session_fork_execution_changed".into())
                    } else {
                        Ok(())
                    }
                },
                |tab, pane| {
                    created
                        .borrow_mut()
                        .push((tab.map(str::to_owned), pane.to_owned()));
                    assert_ne!(case, "panic-created", "fixture callback panicked");
                },
                |_, _| Ok(()),
            );
            assert_eq!(result.is_err(), case != "success", "{case}");
            // The core hears of the fork's tab once, as soon as Herdr made it,
            // whatever happens to its start afterwards.
            let made_tab = herdr
                .methods()
                .iter()
                .any(|method| method == "layout.apply");
            assert_eq!(
                created.into_inner(),
                if made_tab {
                    vec![(Some("w1:t2".to_owned()), "child-pane".to_owned())]
                } else {
                    Vec::new()
                },
                "{case}"
            );
            assert_eq!(
                std::fs::read(if source.exists() {
                    source.clone()
                } else {
                    folder.join("other.jsonl")
                })
                .unwrap(),
                if case.starts_with("source-missing") {
                    other_before
                } else {
                    before
                },
                "fork never rewrites either conversation"
            );
            if case == "success" {
                assert_eq!(result.unwrap(), "child-pane");
            }
            let mut expected = vec![
                "session.snapshot",
                "layout.apply",
                "pane.process_info",
                "pane.get",
                "agent.start",
            ];
            if case == "duplicate-before" || case == "source-missing-before" {
                expected.clear();
            } else if case == "duplicate-after"
                || case == "execution-after"
                || case == "source-missing-after"
            {
                expected.pop();
                if case == "execution-after" {
                    // Retired execution is now refused before even the
                    // shell-read connection; the owned child still closes.
                    expected.pop();
                    expected.pop();
                }
                expected.push("pane.close");
            } else if case == "panic-created" {
                expected = vec!["session.snapshot", "layout.apply", "pane.close"];
            } else if refused {
                expected.push("pane.close");
            }
            assert_eq!(herdr.methods(), expected, "{case}");
        }
    }

    #[test]
    fn an_opencode_fork_proves_its_root_session_and_checkout_before_and_after_its_tab() {
        for case in [
            "success",
            "refused",
            "other-checkout",
            "child-session",
            "removed-after",
            "execution-after",
        ] {
            let home = tempfile::tempdir().unwrap();
            let cwd = hide_platform::fs::identity::canonical(home.path()).unwrap();
            let checkout = cwd.display().to_string();
            let directory = if case == "other-checkout" {
                "/elsewhere/app".to_owned()
            } else {
                checkout.clone()
            };
            crate::fixture::opencode_database(
                home.path(),
                &[
                    ("ses_root", None, directory.as_str()),
                    ("ses_child", Some("ses_root"), directory.as_str()),
                ],
            );
            let session = if case == "child-session" {
                "ses_child"
            } else {
                "ses_root"
            };
            let database = home.path().join(".local/share/opencode/opencode.db");
            let changed = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let observed_changed = Arc::clone(&changed);
            // The fork's tab runs this account's integrated shell and
            // waits for its prompt token, independently of installed shells.
            let shell = crate::fixture::integrated_shell(home.path());
            let expected_shell = shell.shell.clone().unwrap();
            let node = hide_node::Local::new(Some(home.path().to_path_buf())).with_shell(shell);
            let expected_cwd = checkout.clone();
            let herdr = FakeHerdr::start_with_errors("opencode-fork-tab", move |method, params| {
                Ok(match method {
                    "session.snapshot" => json!({"type": "session_snapshot", "snapshot": {
                        "version": "fixture", "protocol": HERDR_PROTOCOL_REVISION,
                        "workspaces": [], "tabs": [], "panes": [], "agents": [],
                        "layouts": [{"workspace_id": "w1", "tab_id": "w1:t1", "zoomed": false,
                            "area": {"x":0,"y":0,"width":80,"height":24},
                            "focused_pane_id": "parent-pane",
                            "panes": [{"pane_id":"parent-pane", "focused":true,
                                "rect":{"x":0,"y":0,"width":80,"height":24}}], "splits": []}]
                    }}),
                    "layout.apply" => {
                        if case == "removed-after" {
                            rusqlite::Connection::open(&database)
                                .unwrap()
                                .execute("DELETE FROM session WHERE id = 'ses_root'", [])
                                .unwrap();
                        }
                        if case == "execution-after" {
                            observed_changed.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                        assert_eq!(params["root"]["cwd"], expected_cwd);
                        assert_eq!(params["focus"], false);
                        // The fork's tab runs the node's shell with the
                        // integration's launch.
                        assert_eq!(params["root"]["command"][0], expected_shell);
                        json!({"type":"layout_apply","layout":{"workspace_id":"w1","tab_id":"w1:t2",
                            "zoomed":false,"focused_pane_id":"child-pane",
                            "root":{"type":"pane","pane_id":"child-pane"}}})
                    }
                    "pane.process_info" => json!({"type":"pane_process_info","process_info":{
                        "pane_id":"child-pane","shell_pid":42,"foreground_process_group_id":42,
                        "foreground_processes":[{"pid":42,"name":"zsh"}]}}),
                    "pane.get" => {
                        assert_eq!(params["pane_id"], "child-pane");
                        json!({"type":"pane_info","pane":{
                            "pane_id":"child-pane","terminal_id":"child-terminal","workspace_id":"w1","tab_id":"w1:t2","focused":false,"agent_status":"unknown","revision":1,
                            "tokens":{"hide_prompt":"1"}}})
                    }
                    "agent.start" => {
                        assert_eq!(params["kind"], "opencode");
                        assert_eq!(params["args"], json!(["-s", "ses_root", "--fork"]));
                        if case == "refused" {
                            return Err(("start_refused".into(), "fixture refused".into()));
                        }
                        json!({"type":"agent_started","argv":[],"agent":{
                            "pane_id":"child-pane","terminal_id":"child-terminal","workspace_id":"w1","tab_id":"w1:t2","focused":false,"agent_status":"idle","revision":2}})
                    }
                    "pane.close" => {
                        assert_eq!(params["pane_id"], "child-pane");
                        json!({"type":"ok"})
                    }
                    other => panic!("unexpected {other}"),
                })
            });
            let request = ForkRequest {
                source_reference: Some(crate::sidebar::SessionAgentSessionPayload {
                    kind: "id".into(),
                    value: session.into(),
                }),
                connection_generation: 0,
                codex_daemon: Default::default(),
                parent_pane_id: "parent-pane".into(),
                agent: crate::fork::ForkableAgent::OpenCode,
                session_id: session.into(),
                cwd: Some(checkout.clone()),
                name: "fork-opencode-1".into(),
                tab_label: Some("Tab 2".into()),
                placement: ForkPlacement::Tab,
                launch: Default::default(),
                permission: Default::default(),
            };
            let result = run_agent_fork_with_registration(
                herdr.engine().as_ref(),
                &node,
                &request,
                &|| {
                    if changed.load(std::sync::atomic::Ordering::SeqCst) {
                        Err("session_fork_execution_changed".into())
                    } else {
                        Ok(())
                    }
                },
                |_, _| {},
                |_, _| Ok(()),
            );
            assert_eq!(result.is_err(), case != "success", "{case}");
            let expected: &[&str] = match case {
                "success" => &[
                    "session.snapshot",
                    "layout.apply",
                    "pane.process_info",
                    "pane.get",
                    "agent.start",
                ],
                "refused" => &[
                    "session.snapshot",
                    "layout.apply",
                    "pane.process_info",
                    "pane.get",
                    "agent.start",
                    "pane.close",
                ],
                "other-checkout" | "child-session" => &[],
                "removed-after" => &[
                    "session.snapshot",
                    "layout.apply",
                    "pane.process_info",
                    "pane.get",
                    "pane.close",
                ],
                _ => &["session.snapshot", "layout.apply", "pane.close"],
            };
            assert_eq!(herdr.methods(), expected, "{case}");
        }
    }

    /// A node that forks OpenCode sessions as `answer` says and records each
    /// ask in `log`, beside the Herdr requests the test records there.
    struct OpenCodeForking {
        inner: hide_node::Local,
        answer: Result<hide_node_link::protocol::OpenCodeFork, &'static str>,
        log: Arc<Mutex<Vec<String>>>,
    }

    impl crate::node_access::NodeLink for OpenCodeForking {
        fn call(
            &self,
            call: hide_node_link::protocol::Call,
            timeout: Duration,
        ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
            let hide_node_link::protocol::Call::OpenCodeFork { source, cwd } = &call else {
                return self.inner.call(call, timeout);
            };
            self.log
                .lock()
                .unwrap()
                .push(format!("opencode_fork {source} {cwd}"));
            match &self.answer {
                Ok(fork) => Ok(serde_json::to_value(fork).unwrap().into()),
                Err(reason) => Err(crate::node_access::LinkError::Refused(
                    hide_node_link::HostError::new(hide_node_link::ErrorCode::Io, *reason),
                )),
            }
        }

        fn reader_features(&self) -> Option<&hide_node_link::sessions::ReaderFeatures> {
            self.inner.reader_features()
        }

        fn in_process(&self) -> bool {
            self.inner.in_process()
        }
    }

    /// OpenCode 2.0.29's interactive command has no `--fork`: the node's
    /// server forks the session once, before the fork's tab exists, and the
    /// tab opens the session it made; a node that cannot fork it ends the fork
    /// with its reason and no tab, and a fork whose start then fails names the
    /// session OpenCode made, which stays in OpenCode's list.
    #[test]
    fn an_opencode_2_fork_is_made_by_its_server_before_its_tab_and_the_tab_opens_it() {
        for case in ["forked", "refused", "start_failed"] {
            let home = tempfile::tempdir().unwrap();
            let cwd = hide_platform::fs::identity::canonical(home.path()).unwrap();
            let checkout = cwd.display().to_string();
            crate::fixture::opencode_2_database(
                home.path(),
                &[("ses_root", None, checkout.as_str())],
            );
            let log = Arc::new(Mutex::new(Vec::new()));
            let node = OpenCodeForking {
                inner: hide_node::Local::new(Some(home.path().to_path_buf()))
                    .with_shell(crate::fixture::integrated_shell(home.path())),
                answer: if case == "refused" {
                    Err("opencode_fork_timed_out")
                } else {
                    Ok(hide_node_link::protocol::OpenCodeFork::Forked {
                        id: "ses_fork".into(),
                    })
                },
                log: Arc::clone(&log),
            };
            let herdr_log = Arc::clone(&log);
            let herdr = FakeHerdr::start_with_errors(
                "opencode-2-fork-tab",
                move |method, params| {
                    herdr_log.lock().unwrap().push(method.to_owned());
                    Ok(match method {
                        "session.snapshot" => json!({"type": "session_snapshot", "snapshot": {
                            "version": "fixture", "protocol": HERDR_PROTOCOL_REVISION,
                            "workspaces": [], "tabs": [], "panes": [], "agents": [],
                            "layouts": [{"workspace_id": "w1", "tab_id": "w1:t1", "zoomed": false,
                                "area": {"x":0,"y":0,"width":80,"height":24},
                                "focused_pane_id": "parent-pane",
                                "panes": [{"pane_id":"parent-pane", "focused":true,
                                    "rect":{"x":0,"y":0,"width":80,"height":24}}], "splits": []}]
                        }}),
                        "layout.apply" => {
                            json!({"type":"layout_apply","layout":{"workspace_id":"w1","tab_id":"w1:t2",
                        "zoomed":false,"focused_pane_id":"child-pane",
                        "root":{"type":"pane","pane_id":"child-pane"}}})
                        }
                        "pane.process_info" => json!({"type":"pane_process_info","process_info":{
                        "pane_id":"child-pane","shell_pid":42,"foreground_process_group_id":42,
                        "foreground_processes":[{"pid":42,"name":"zsh"}]}}),
                        "pane.get" => json!({"type":"pane_info","pane":{
                        "pane_id":"child-pane","terminal_id":"child-terminal","workspace_id":"w1","tab_id":"w1:t2","focused":false,"agent_status":"unknown","revision":1,
                        "tokens":{"hide_prompt":"1"}}}),
                        "agent.start" => {
                            assert_eq!(params["kind"], "opencode");
                            assert_eq!(params["args"], json!(["-s", "ses_fork"]));
                            if case == "start_failed" {
                                return Err(("start_refused".into(), "fixture refused".into()));
                            }
                            json!({"type":"agent_started","argv":[],"agent":{
                            "pane_id":"child-pane","terminal_id":"child-terminal","workspace_id":"w1","tab_id":"w1:t2","focused":false,"agent_status":"idle","revision":2}})
                        }
                        "pane.close" => {
                            assert_eq!(params["pane_id"], "child-pane");
                            json!({"type":"ok"})
                        }
                        other => panic!("unexpected {other}"),
                    })
                },
            );
            let request = ForkRequest {
                source_reference: Some(crate::sidebar::SessionAgentSessionPayload {
                    kind: "id".into(),
                    value: "ses_root".into(),
                }),
                connection_generation: 0,
                codex_daemon: Default::default(),
                parent_pane_id: "parent-pane".into(),
                agent: crate::fork::ForkableAgent::OpenCode,
                session_id: "ses_root".into(),
                cwd: Some(checkout.clone()),
                name: "fork-opencode-2".into(),
                tab_label: Some("Tab 2".into()),
                placement: ForkPlacement::Tab,
                launch: Default::default(),
                permission: Default::default(),
            };
            let result = run_agent_fork_with_registration(
                herdr.engine().as_ref(),
                &node,
                &request,
                &|| Ok(()),
                |_, _| {},
                |_, _| Ok(()),
            );
            let fork = format!("opencode_fork ses_root {checkout}");
            let opened = [
                fork.as_str(),
                "session.snapshot",
                "layout.apply",
                "pane.process_info",
                "pane.get",
                "agent.start",
            ];
            match case {
                "forked" => {
                    assert_eq!(result.unwrap(), "child-pane");
                    assert_eq!(*log.lock().unwrap(), opened);
                }
                "refused" => {
                    let error = result.unwrap_err();
                    assert!(error.contains("opencode_fork_timed_out"), "{error}");
                    assert_eq!(*log.lock().unwrap(), [fork.as_str()]);
                }
                _ => {
                    let error = result.unwrap_err();
                    assert!(
                        error.ends_with(
                            "OpenCode made the fork's session ses_fork, which stays in OpenCode's session list"
                        ),
                        "{error}"
                    );
                    let mut closed = opened.to_vec();
                    closed.push("pane.close");
                    assert_eq!(*log.lock().unwrap(), closed);
                }
            }
        }
    }

    #[test]
    fn a_grok_fork_resumes_its_proven_session_as_a_new_one_in_its_own_tab() {
        let id = "0199b000-0000-7000-8000-0000000000f0";
        for case in [
            "success",
            "refused",
            "summary-changed-before",
            "execution-after",
        ] {
            let home = tempfile::tempdir().unwrap();
            let cwd = hide_platform::fs::identity::canonical(home.path()).unwrap();
            let source = crate::fixture::grok_session(home.path(), &cwd, id);
            let summary = source.with_file_name("summary.json");
            if case == "summary-changed-before" {
                let other = std::fs::read_to_string(&summary)
                    .unwrap()
                    .replace(id, "0199b000-0000-7000-8000-0000000000f1");
                std::fs::write(&summary, other).unwrap();
            }
            let before = std::fs::read(&summary).unwrap();
            let changed = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let observed_changed = Arc::clone(&changed);
            // The fork's tab runs this account's integrated shell and
            // waits for its prompt token, independently of installed shells.
            let shell = crate::fixture::integrated_shell(home.path());
            let expected_shell = shell.shell.clone().unwrap();
            let node = hide_node::Local::new(Some(home.path().to_path_buf())).with_shell(shell);
            let herdr = FakeHerdr::start_with_errors("grok-fork-tab", move |method, params| {
                Ok(match method {
                    "session.snapshot" => json!({"type": "session_snapshot", "snapshot": {
                        "version": "fixture", "protocol": HERDR_PROTOCOL_REVISION,
                        "workspaces": [], "tabs": [], "panes": [], "agents": [],
                        "layouts": [{"workspace_id": "w1", "tab_id": "w1:t1", "zoomed": false,
                            "area": {"x":0,"y":0,"width":80,"height":24},
                            "focused_pane_id": "parent-pane",
                            "panes": [{"pane_id":"parent-pane", "focused":true,
                                "rect":{"x":0,"y":0,"width":80,"height":24}}], "splits": []}]
                    }}),
                    "layout.apply" => {
                        if case == "execution-after" {
                            observed_changed.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                        assert_eq!(params["focus"], false);
                        // The fork's tab runs the node's shell with the
                        // integration's launch.
                        assert_eq!(params["root"]["command"][0], expected_shell);
                        json!({"type":"layout_apply","layout":{"workspace_id":"w1","tab_id":"w1:t2",
                            "zoomed":false,"focused_pane_id":"child-pane",
                            "root":{"type":"pane","pane_id":"child-pane"}}})
                    }
                    "pane.process_info" => json!({"type":"pane_process_info","process_info":{
                        "pane_id":"child-pane","shell_pid":42,"foreground_process_group_id":42,
                        "foreground_processes":[{"pid":42,"name":"zsh"}]}}),
                    "pane.get" => {
                        assert_eq!(params["pane_id"], "child-pane");
                        json!({"type":"pane_info","pane":{
                            "pane_id":"child-pane","terminal_id":"child-terminal","workspace_id":"w1","tab_id":"w1:t2","focused":false,"agent_status":"unknown","revision":1,
                            "tokens":{"hide_prompt":"1"}}})
                    }
                    "agent.start" => {
                        assert_eq!(params["kind"], "grok");
                        assert_eq!(params["args"], json!(["--resume", id, "--fork-session"]));
                        if case == "refused" {
                            return Err(("start_refused".into(), "fixture refused".into()));
                        }
                        json!({"type":"agent_started","argv":[],"agent":{
                            "pane_id":"child-pane","terminal_id":"child-terminal","workspace_id":"w1","tab_id":"w1:t2","focused":false,"agent_status":"idle","revision":2}})
                    }
                    "pane.close" => json!({"type":"ok"}),
                    other => panic!("unexpected {other}"),
                })
            });
            // The label overlay hands a native-file agent its proven file.
            let request = ForkRequest {
                source_reference: Some(crate::sidebar::SessionAgentSessionPayload {
                    kind: "path".into(),
                    value: source.display().to_string(),
                }),
                connection_generation: 0,
                codex_daemon: Default::default(),
                parent_pane_id: "parent-pane".into(),
                agent: crate::fork::ForkableAgent::Grok,
                session_id: id.into(),
                cwd: Some(cwd.display().to_string()),
                name: "fork-grok-1".into(),
                tab_label: Some("Tab 2".into()),
                placement: ForkPlacement::Tab,
                launch: Default::default(),
                permission: Default::default(),
            };
            let result = run_agent_fork_with_registration(
                herdr.engine().as_ref(),
                &node,
                &request,
                &|| {
                    if changed.load(std::sync::atomic::Ordering::SeqCst) {
                        Err("session_fork_execution_changed".into())
                    } else {
                        Ok(())
                    }
                },
                |_, _| {},
                |_, _| Ok(()),
            );
            assert_eq!(result.is_err(), case != "success", "{case}: {result:?}");
            assert_eq!(std::fs::read(&summary).unwrap(), before, "{case}");
            let expected: &[&str] = match case {
                "success" => &[
                    "session.snapshot",
                    "layout.apply",
                    "pane.process_info",
                    "pane.get",
                    "agent.start",
                ],
                "refused" => &[
                    "session.snapshot",
                    "layout.apply",
                    "pane.process_info",
                    "pane.get",
                    "agent.start",
                    "pane.close",
                ],
                "summary-changed-before" => &[],
                _ => &["session.snapshot", "layout.apply", "pane.close"],
            };
            assert_eq!(herdr.methods(), expected, "{case}");
        }
    }

    #[test]
    fn an_agy_fork_types_fork_once_and_reports_the_one_conversation_it_made() {
        let source = "a9c0e2f4-0000-4000-8000-000000000001";
        let fork = "a9c0e2f4-0000-4000-8000-000000000002";
        // `moved`: the node's clock is behind the core's, so agy stamps the
        // fork's row in the past, and the source takes another step while
        // agy writes it; the fork is still the one row that was not there.
        // `untrusted`: agy does not trust the checkout, so the resumed pane
        // would open on agy's trust question, which `/fork` must not answer.
        // `in-motion`: the first index reads meet agy writing its index
        // (1 fork in 3 failed on a live agy 1.4.0) and are read again.
        // `unreadable`: a refusal that is not agy writing ends the fork at
        // its first read, with no second one.
        // `stray`: the composer holds text before `/fork` (Herdr's answer to
        // agy's mode query, `27;1$y`, #1529), which is erased first.
        // `stuck`: text outlasts the erase, as a hint a later agy draws in an
        // empty composer might, and `/fork` is typed as before.
        // `no-composer`: the screen shows no composer as declared, and
        // `/fork` is typed as before.
        const AGY_EMPTY: &str =
            include_str!("../../../hide-agent-adapter/tests/fixtures/composer/agy-0-empty.ansi");
        const AGY_TYPED: &str =
            include_str!("../../../hide-agent-adapter/tests/fixtures/composer/agy-1-typed.ansi");
        for case in [
            "stray",
            "stuck",
            "no-composer",
            "success",
            "ambiguous",
            "moved",
            "untrusted",
            "in-motion",
            "unreadable",
        ] {
            let home = tempfile::tempdir().unwrap();
            let cwd = hide_platform::fs::identity::canonical(home.path()).unwrap();
            if case != "untrusted" {
                let settings = home.path().join(".gemini/antigravity-cli/settings.json");
                std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
                std::fs::write(
                    &settings,
                    json!({ "trustedWorkspaces": [cwd.to_str().unwrap()] }).to_string(),
                )
                .unwrap();
            }
            crate::fixture::antigravity_conversation(
                home.path(),
                &cwd,
                source,
                4,
                Some("2026-10-03 01:00:00+00:00"),
            );
            let node = DatabaseInMotion {
                inner: hide_node::Local::new(Some(home.path().to_path_buf())),
                left: std::sync::atomic::AtomicUsize::new(match case {
                    "in-motion" => 3,
                    "unreadable" => 1,
                    _ => 0,
                }),
                // What the node answers for each: agy writing its index is
                // busy, an index Hide cannot read is not.
                refusal: if case == "unreadable" {
                    (hide_node_link::ErrorCode::Io, "antigravity_db_unreadable")
                } else {
                    (hide_node_link::ErrorCode::Busy, "antigravity_db_changed")
                },
            };
            let index_home = home.path().to_path_buf();
            let index_cwd = cwd.clone();
            let statuses = Arc::new(Mutex::new(vec!["working", "idle"]));
            let held = Arc::new(Mutex::new(matches!(case, "stray" | "stuck")));
            let herdr = FakeHerdr::start("agy-fork", move |method, params| match method {
                "pane.read" => {
                    assert_eq!(params["format"], "ansi");
                    let text = if case == "no-composer" {
                        "$ "
                    } else if *held.lock().unwrap() {
                        AGY_TYPED
                    } else {
                        AGY_EMPTY
                    };
                    json!({"type":"pane_read","read":{
                        "pane_id":"child-pane","workspace_id":"w1","tab_id":"w1:t2",
                        "source":"visible","format":"ansi","revision":1,
                        "truncated":false,"text":text}})
                }
                "pane.send_keys" => {
                    assert_eq!(params["keys"], json!(vec!["backspace"; 8]));
                    if case == "stray" {
                        *held.lock().unwrap() = false;
                    }
                    json!({"type":"ok"})
                }
                "pane.get" => {
                    let mut statuses = statuses.lock().unwrap();
                    let status = if statuses.len() > 1 {
                        statuses.remove(0)
                    } else {
                        statuses[0]
                    };
                    json!({"type":"pane_info","pane":{
                        "pane_id":"child-pane","terminal_id":"child-terminal","workspace_id":"w1",
                        "tab_id":"w1:t2","focused":false,"agent_status":status,"revision":1}})
                }
                "pane.send_input" => {
                    assert!(
                        case == "stuck" || !*held.lock().unwrap(),
                        "/fork was typed behind text"
                    );
                    assert_eq!(params["text"], "/fork");
                    assert_eq!(params["keys"], json!(["enter"]));
                    // agy writes the fork's row at the source's step count.
                    crate::fixture::antigravity_conversation(
                        &index_home,
                        &index_cwd,
                        fork,
                        4,
                        (case == "moved").then_some("2026-10-01 00:00:00+00:00"),
                    );
                    if case == "moved" {
                        rusqlite::Connection::open(
                            index_home.join(".gemini/antigravity-cli/conversation_summaries.db"),
                        )
                        .unwrap()
                        .execute(
                            "UPDATE conversation_summaries SET step_count = 5 \
                             WHERE conversation_id = ?1",
                            [source],
                        )
                        .unwrap();
                    }
                    if case == "ambiguous" {
                        crate::fixture::antigravity_conversation(
                            &index_home,
                            &index_cwd,
                            "a9c0e2f4-0000-4000-8000-000000000003",
                            4,
                            None,
                        );
                    }
                    json!({"type":"ok"})
                }
                "pane.report_agent_session" => {
                    assert_eq!(params["agent_session_id"], fork);
                    assert_eq!(params["agent"], "agy");
                    assert_eq!(params["source"], "herdr:antigravity_cli");
                    json!({"type":"ok"})
                }
                other => panic!("unexpected {other}"),
            });
            let request = ForkRequest {
                source_reference: None,
                connection_generation: 0,
                codex_daemon: Default::default(),
                parent_pane_id: "parent-pane".into(),
                agent: crate::fork::ForkableAgent::Antigravity,
                session_id: source.into(),
                cwd: Some(cwd.display().to_string()),
                name: "fork-agy-1".into(),
                tab_label: Some("Tab 2".into()),
                placement: ForkPlacement::Tab,
                launch: Default::default(),
                permission: Default::default(),
            };
            let result = fork_antigravity(herdr.engine().as_ref(), &node, &request, "child-pane");
            // Eight erases of eight reach the limit of 64.
            let mut stuck = vec!["pane.get", "pane.get", "pane.read"];
            for _ in 0..8 {
                stuck.extend(["pane.send_keys", "pane.read"]);
            }
            stuck.extend(["pane.get", "pane.send_input", "pane.report_agent_session"]);
            let expected: &[&str] = match case {
                "stray" => {
                    assert_eq!(result, Ok(()), "{case}");
                    &[
                        "pane.get",
                        "pane.get",
                        "pane.read",
                        "pane.send_keys",
                        "pane.read",
                        "pane.get",
                        "pane.send_input",
                        "pane.report_agent_session",
                    ]
                }
                "stuck" => {
                    assert_eq!(result, Ok(()), "{case}");
                    &stuck
                }
                "success" | "moved" | "in-motion" | "no-composer" => {
                    assert_eq!(result, Ok(()), "{case}");
                    &[
                        "pane.get",
                        "pane.get",
                        "pane.read",
                        "pane.get",
                        "pane.send_input",
                        "pane.report_agent_session",
                    ]
                }
                "unreadable" => {
                    assert!(
                        result
                            .as_ref()
                            .unwrap_err()
                            .contains("antigravity_db_unreadable"),
                        "{result:?}"
                    );
                    &["pane.get", "pane.get"]
                }
                "untrusted" => {
                    assert!(
                        result
                            .as_ref()
                            .unwrap_err()
                            .contains("antigravity_fork_untrusted"),
                        "{result:?}"
                    );
                    &["pane.get", "pane.get"]
                }
                _ => {
                    assert!(
                        result
                            .as_ref()
                            .unwrap_err()
                            .contains("antigravity_fork_ambiguous"),
                        "{result:?}"
                    );
                    &[
                        "pane.get",
                        "pane.get",
                        "pane.read",
                        "pane.get",
                        "pane.send_input",
                    ]
                }
            };
            assert_eq!(herdr.methods(), expected, "{case}");
            assert_eq!(
                node.left.load(std::sync::atomic::Ordering::SeqCst),
                0,
                "{case}"
            );
        }
    }

    /// The route Fork, Sleep's wake and resume prove before they start an
    /// agent that keeps its records in a database, agy's index or
    /// OpenCode's database (Windows run 39104633599 refused an OpenCode
    /// wake this way): a proof that meets the agent writing is read again,
    /// and one that cannot read the database is refused at its first read.
    #[test]
    fn a_database_route_proof_is_read_again_only_while_it_meets_the_agent_writing() {
        for agent in ["agy", "opencode"] {
            for (left, code, proven) in [
                (3, hide_node_link::ErrorCode::Busy, true),
                (1, hide_node_link::ErrorCode::Io, false),
            ] {
                let home = tempfile::tempdir().unwrap();
                let cwd = hide_platform::fs::identity::canonical(home.path()).unwrap();
                let (id, reasons) = if agent == "agy" {
                    let id = "a9c0e2f4-0000-4000-8000-000000000001";
                    crate::fixture::antigravity_conversation(home.path(), &cwd, id, 4, None);
                    let transcript = hide_session::antigravity::transcript(home.path(), id);
                    std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
                    std::fs::write(
                        &transcript,
                        include_str!(
                            "../../../hide-session/tests/fixtures/adapters/antigravity-1.3.6/transcript_full.jsonl"
                        ),
                    )
                    .unwrap();
                    (id, ("antigravity_db_busy", "antigravity_db_unreadable"))
                } else {
                    let id = "ses_root";
                    crate::fixture::opencode_database(
                        home.path(),
                        &[(id, None, cwd.to_str().unwrap())],
                    );
                    (id, ("opencode_db_busy", "opencode_db_unreadable"))
                };
                let node = DatabaseInMotion {
                    inner: hide_node::Local::new(Some(home.path().to_path_buf())),
                    left: std::sync::atomic::AtomicUsize::new(left),
                    refusal: (code, if proven { reasons.0 } else { reasons.1 }),
                };
                let reference = crate::sidebar::SessionAgentSessionPayload {
                    kind: "id".into(),
                    value: id.into(),
                };
                let result =
                    confirm_session_launch(&node, agent, id, cwd.to_str(), Some(&reference));
                assert_eq!(result.is_ok(), proven, "{agent} {code:?}: {result:?}");
                assert_eq!(
                    node.left.load(std::sync::atomic::Ordering::SeqCst),
                    0,
                    "{agent} {code:?}"
                );
            }
        }
    }

    /// An OpenCode fork whose route read met OpenCode writing its database
    /// forked nothing, so it is asked again; one that cannot read the
    /// database is refused at its first ask.
    #[test]
    fn an_opencode_fork_is_asked_again_only_while_its_route_meets_opencode_writing() {
        for (left, code, forked) in [
            (3, hide_node_link::ErrorCode::Busy, true),
            (1, hide_node_link::ErrorCode::Io, false),
        ] {
            let home = tempfile::tempdir().unwrap();
            let checkout = hide_platform::fs::identity::canonical(home.path())
                .unwrap()
                .display()
                .to_string();
            crate::fixture::opencode_database(home.path(), &[("ses_root", None, &checkout)]);
            let node = DatabaseInMotion {
                inner: hide_node::Local::new(Some(home.path().to_path_buf())),
                left: std::sync::atomic::AtomicUsize::new(left),
                refusal: (code, "opencode_db_busy"),
            };
            let request = ForkRequest {
                source_reference: None,
                connection_generation: 0,
                codex_daemon: Default::default(),
                parent_pane_id: "parent-pane".into(),
                agent: crate::fork::ForkableAgent::OpenCode,
                session_id: "ses_root".into(),
                cwd: Some(checkout),
                name: "fork-opencode-1".into(),
                tab_label: None,
                placement: ForkPlacement::Tab,
                launch: Default::default(),
                permission: Default::default(),
            };
            let result = fork_arguments(&node, &request);
            assert_eq!(result.is_ok(), forked, "{code:?}: {result:?}");
            assert_eq!(
                node.left.load(std::sync::atomic::Ordering::SeqCst),
                0,
                "{code:?}"
            );
        }
    }

    /// A node whose next `left` reads of an agent's database are refused
    /// with `refusal`.
    struct DatabaseInMotion {
        inner: hide_node::Local,
        left: std::sync::atomic::AtomicUsize,
        refusal: (hide_node_link::ErrorCode, &'static str),
    }

    impl crate::node_access::NodeLink for DatabaseInMotion {
        fn reader_features(&self) -> Option<&hide_node_link::sessions::ReaderFeatures> {
            self.inner.reader_features()
        }

        fn call(
            &self,
            call: hide_node_link::protocol::Call,
            timeout: Duration,
        ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
            if matches!(
                call,
                hide_node_link::protocol::Call::AntigravityForkMark { .. }
                    | hide_node_link::protocol::Call::AntigravityFork { .. }
                    | hide_node_link::protocol::Call::SessionActivity { .. }
                    | hide_node_link::protocol::Call::OpenCodeFork { .. }
            ) && self
                .left
                .try_update(
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                    |left| left.checked_sub(1),
                )
                .is_ok()
            {
                return Err(crate::node_access::LinkError::Refused(
                    hide_node_link::HostError::new(self.refusal.0, self.refusal.1),
                ));
            }
            self.inner.call(call, timeout)
        }
    }

    #[test]
    fn internal_registration_failure_keeps_the_started_fork() {
        let herdr = FakeHerdr::start("fork-registration-failure", |method, _| match method {
            "pane.split" => json!({"type": "pane_info", "pane": {
                "pane_id": "child-pane", "terminal_id": "child-terminal",
                "workspace_id": "w1", "tab_id": "w1:t1", "focused": true,
                "agent_status": "idle", "revision": 1
            }}),
            "pane.process_info" => json!({"type": "pane_process_info", "process_info": {
                "pane_id": "child-pane", "shell_pid": 42, "foreground_process_group_id": 42,
                "foreground_processes": [{"pid": 42, "name": "zsh"}]
            }}),
            "agent.start" => json!({"type": "agent_started", "argv": [], "agent": {
                "pane_id": "child-pane", "terminal_id": "child-terminal",
                "workspace_id": "w1", "tab_id": "w1:t1", "focused": true,
                "agent_status": "idle", "revision": 2
            }}),
            other => panic!("unexpected {other}"),
        });
        let request = ForkRequest {
            source_reference: None,
            connection_generation: 0,
            codex_daemon: crate::codex_launch::CodexDaemon::Present,
            parent_pane_id: "parent-pane".to_owned(),
            agent: crate::fork::ForkableAgent::Codex,
            session_id: "3f2b1c00-0000-4000-8000-000000000001".to_owned(),
            cwd: Some("/checkout".to_owned()),
            name: "fork-parent-pane-1".to_owned(),
            tab_label: None,
            placement: ForkPlacement::Side,
            launch: Default::default(),
            permission: Default::default(),
        };

        let pane_id = run_agent_fork_with_registration(
            herdr.engine().as_ref(),
            &hide_node::Local::new(None),
            &request,
            &|| Ok(()),
            |_, _| {},
            |_, _| Err("ledger_unavailable".into()),
        )
        .expect("registration cannot close a started fork");
        assert_eq!(pane_id, "child-pane");
        assert_eq!(
            herdr.methods(),
            ["pane.split", "pane.process_info", "agent.start",]
        );
    }
}
