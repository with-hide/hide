//! Hide's agent-hook helper.
//!
//! Two jobs, and a hard line between them:
//!
//! - `hook` is what the installed hook entry runs. It counts one event for
//!   the pane it fired in and reports the pane's totals to Herdr. It never
//!   writes a runtime's configuration and never fails loudly: an agent turn
//!   must not break because Hide could not count something.
//! - `doctor` prints the same hook-install judgement the Settings diagnosis
//!   shows, so the state can be read without launching the app (PRD B30).
//!
//! Installing and removing are deliberately absent. They change a file the
//! operator owns, and the only paths authorised to do that are Hide's first
//! run and the Settings action the operator pressed (PRD D-25, D-31, B28).

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use hide_agent_adapter::HookDialect;
use hide_agent_hooks::counters::{self, Change, Reporter, SubagentInput};
use hide_agent_hooks::diagnosis::Diagnosis;
use hide_agent_hooks::guidance::{self, GuidanceAgent};
use hide_agent_hooks::report;
use hide_agent_hooks::runtime::{AgentRuntime, HookEvent, hook_stdout};
use hide_agent_hooks::{antigravity, basic};
use hide_platform::process::{OwnedChild, OwnerWatch};

const PROMPT_BUDGET: Duration = Duration::from_millis(1_850);
/// How long the spawn guard waits for the daemon's answer, inside the entry's
/// own eight second timeout. It is spent only for a call that starts an agent
/// through Herdr, so an ordinary shell call never waits on it.
const GUARD_BUDGET: Duration = Duration::from_millis(2_500);
/// How long the spawn guard waits for the tool call's payload.
const GUARD_PAYLOAD_BUDGET: Duration = Duration::from_millis(500);
const INTAKE_BUDGET: Duration = Duration::from_millis(1_650);
/// How long a prompt hook waits for the runtime's payload to learn whether
/// the prompt was Hide's bell. A producer that never closes stdin costs this
/// and no more, and the prompt is then treated as the operator's.
const PROMPT_PAYLOAD_BUDGET: Duration = Duration::from_millis(500);

#[path = "../workspace_context.rs"]
mod workspace_context;

#[path = "../plugin/helper.rs"]
mod plugin_helper;

fn main() -> ExitCode {
    let started = Instant::now();
    // Guarded launches acknowledge ownership before parsing, filesystem work
    // or runtime stdin. Keep this guard until all output and intake finish.
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let owner_watch = match OwnerWatch::from_launch() {
        Ok(watch) => watch,
        // Exit 2 from a `PreToolUse` hook refuses the tool call. The outer hook
        // is what an agent runs, and it must never end that way; the inner one
        // is only ever started by it. A script of Hide's in an agent's folder
        // reads any exit as no answer, and an agent's call is never failed by
        // Hide's helper. Cursor's permission hooks get their allow, since
        // Cursor reads output that is not a valid answer as a refusal.
        Err(_)
            if matches!(
                arguments.first().map(String::as_str),
                Some("hook" | "opencode" | "pi" | "omp")
            ) =>
        {
            if cursor_permission_hook(&arguments) {
                let _ = print_line(basic::CURSOR_ALLOW);
            }
            if antigravity_tool_hook(&arguments) {
                print_line(antigravity::ASK);
            }
            return ExitCode::SUCCESS;
        }
        Err(_) => return ExitCode::from(2),
    };
    match arguments.first().map(String::as_str) {
        Some("hook") => {
            if argument_value("--event", &arguments)
                .is_some_and(|event| matches!(event.as_str(), "PostToolUse" | "PostToolUseFailure"))
            {
                if std::panic::catch_unwind(|| run_observation_hook(&arguments, started)).is_err()
                    && let Some(home) = home_directory()
                {
                    hide_agent_hooks::delivery::diagnose(&home, "format");
                }
                return ExitCode::SUCCESS;
            }
            // Grok and Cursor have no Memory: their hooks carry the spawn
            // guard, subagent counts and letters at their declared gaps.
            // Cursor adds session guidance; Grok observes bell submissions
            // and takes letters at the end of a turn too
            // (`hide_agent_hooks::guidance`, `hide_agent_hooks::basic`).
            if let Some(agent) = argument_value("--runtime", &arguments)
                .and_then(|value| GuidanceAgent::from_id(&value))
            {
                if agent == GuidanceAgent::Antigravity {
                    run_antigravity_hook(&arguments, started);
                } else {
                    run_basic_hook(agent, &arguments, started);
                }
                return ExitCode::SUCCESS;
            }
            // An entry an earlier build wrote for an agent Hide no longer
            // supports runs nothing: the kit's retirement takes it out, and
            // until then it must not count a pane or print guidance.
            if argument_value("--runtime", &arguments)
                .is_some_and(|value| GuidanceAgent::is_retired_id(&value))
            {
                return ExitCode::SUCCESS;
            }
            // Cursor and Grok load Claude Code's hooks from
            // `~/.claude/settings.json` beside their own and run both, so under
            // them Claude Code's hook stays out and their own hook is the one
            // that speaks, once (`docs/agent-hooks.md`, Other agents); under
            // OpenCode, which runs it through an operator's bridge plugin,
            // Hide's OpenCode plugin is.
            if argument_value("--runtime", &arguments).and_then(|value| AgentRuntime::parse(&value))
                == Some(AgentRuntime::ClaudeCode)
                && hide_agent_hooks::runtime::silences_claude_hook(|name| std::env::var_os(name))
            {
                return ExitCode::SUCCESS;
            }
            let event = argument_value("--event", &arguments);
            if event.as_deref() == Some(HookEvent::PreToolUse.name()) {
                // A guard that fails answers like one with nothing to say:
                // the call runs (PRD herdr-spawn-guard D-04). A panic is one
                // such failure, and it must not end the hook with a code the
                // agent reads as a refusal.
                if std::panic::catch_unwind(|| run_spawn_guard(&arguments, started)).is_err() {
                    guard_failed(&arguments, "panic");
                }
            } else if event.as_deref() == Some(HookEvent::UserPromptSubmit.name()) {
                run_prompt_hook(&arguments, started + PROMPT_BUDGET);
            } else {
                run_hook(&arguments, started);
            }
            // A hook that could not report is not a failed turn. Its visible
            // outcome is the pane reading as uninstrumented (PRD B32).
            ExitCode::SUCCESS
        }
        // What Hide's script in an agent's own folder asks
        // (`hide_agent_hooks::plugin`): OpenCode's plugin, Pi's and omp's
        // extension.
        Some(agent @ ("opencode" | "pi" | "omp")) => {
            let _ = std::panic::catch_unwind(|| {
                plugin_helper::run(agent, arguments.get(1).map(String::as_str), started)
            });
            ExitCode::SUCCESS
        }
        Some("hook-inner") if owner_watch.is_some() => {
            run_hook(&arguments, started);
            ExitCode::SUCCESS
        }
        // An internal operation must never bypass its parent deadline.
        Some("hook-inner") => ExitCode::from(2),
        Some("doctor") => match run_doctor(&arguments) {
            Ok(text) => {
                println!("{text}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::from(2)
            }
        },
        _ => {
            eprintln!("{}", usage());
            ExitCode::from(2)
        }
    }
}

/// Post observation never prints a permission/context decision or alters the
/// original tool's result. Ordinary calls do no new daemon/Herdr lookup.
fn run_observation_hook(arguments: &[String], started: Instant) {
    use hide_agent_hooks::observation;
    let Some(runtime) = argument_value("--runtime", arguments) else {
        return;
    };
    let Some(adapter) = hide_agent_adapter::adapter(&runtime) else {
        return;
    };
    let hide_agent_adapter::ObservationSupport::Native(dialect) = adapter.observation else {
        return;
    };
    if dialect == hide_agent_adapter::HookDialect::ClaudeCode
        && hide_agent_hooks::runtime::ForeignOrigin::detect(|name| std::env::var_os(name)).is_some()
    {
        return;
    }
    if std::env::var_os("HERDR_PANE_ID").is_none() {
        return;
    }
    let Some(home) = home_directory() else {
        return;
    };
    let Some((payload, truncated)) = read_stdin_before_deadline(started + GUARD_PAYLOAD_BUDGET)
    else {
        hide_agent_hooks::delivery::diagnose(&home, "deadline");
        return;
    };
    if truncated {
        hide_agent_hooks::delivery::diagnose(&home, "format");
        return;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    let failure = argument_value("--event", arguments).as_deref() == Some("PostToolUseFailure");
    if let Some(observed) =
        observation::native(adapter.id, &payload, false, failure, now, &|name| {
            std::env::var(name).ok()
        })
    {
        observation::send(
            &home,
            &observed,
            started + GUARD_PAYLOAD_BUDGET + observation::SEND_BUDGET,
        );
    }
}

fn usage() -> String {
    "usage: hide-agent-hooks hook --runtime <claude-code|codex> \
     --event <SessionStart|UserPromptSubmit|SubagentStart|SubagentStop|Stop|PreToolUse|PostToolUse|PostToolUseFailure> \
     [--memory-injection] [--source <install marker>]\n       \
     hide-agent-hooks hook --runtime grok \
     --event <SessionStart|UserPromptSubmit|PreToolUse|PostToolUse|PostToolUseFailure|SubagentStart|SubagentStop|Stop> \
     [--source <install marker>]\n       \
     hide-agent-hooks hook --runtime antigravity \
     --event <SessionStart|UserPromptSubmit|PreToolUse|PostToolUse|Stop> [--source <install marker>]\n       \
     hide-agent-hooks <opencode|pi|omp> <start|prompt|confirm|tool|observation|letters|subagents>\n       \
     hide-agent-hooks doctor [--json]"
        .to_owned()
}

/// Whether this is a Cursor permission hook (`preToolUse`, `subagentStart`)
/// outside Grok: one whose every answer but a refusal must be Cursor's allow.
fn cursor_permission_hook(arguments: &[String]) -> bool {
    argument_value("--runtime", arguments).and_then(|value| GuidanceAgent::from_id(&value))
        == Some(GuidanceAgent::Cursor)
        && argument_value("--event", arguments)
            .and_then(|value| HookEvent::parse(&value))
            .is_some_and(basic::cursor_permission_event)
        && !inside_grok()
}

/// Whether this is agy's tool hook, whose every answer but a refusal must be
/// agy's ask: agy refuses a call whose hook fails or prints what it cannot read.
fn antigravity_tool_hook(arguments: &[String]) -> bool {
    argument_value("--runtime", arguments).and_then(|value| GuidanceAgent::from_id(&value))
        == Some(GuidanceAgent::Antigravity)
        && argument_value("--event", arguments).as_deref() == Some(HookEvent::PreToolUse.name())
}

fn inside_grok() -> bool {
    hide_agent_hooks::runtime::inside_grok(|name| std::env::var_os(name))
}

/// The hook of Grok or Cursor (PRD grok-cursor-hooks): the spawn guard on a
/// shell call, the subagent count on the subagent and turn events, and
/// Cursor's guidance at session start. It never fails loudly, and Cursor's
/// permission events get an answer on every path.
fn run_basic_hook(agent: GuidanceAgent, arguments: &[String], started: Instant) {
    let Some(dialect) = agent.dialect() else {
        return;
    };
    let Some(event) =
        argument_value("--event", arguments).and_then(|value| HookEvent::parse(&value))
    else {
        return;
    };
    // Grok runs Cursor's `hooks.json` beside its own file; there only Grok's
    // own hook speaks, so nothing is refused or counted twice (D-05).
    if dialect == HookDialect::Cursor && inside_grok() {
        return;
    }
    // Stop has one payload shared by its intake and subagent count.
    let payload = matches!(event, HookEvent::Stop | HookEvent::UserPromptSubmit)
        .then(|| read_stdin_before_deadline(started + GUARD_PAYLOAD_BUDGET))
        .flatten();
    let (answer, intake) = std::panic::catch_unwind(|| match event {
        HookEvent::PreToolUse => basic_tool_call(dialect, agent.id(), started),
        HookEvent::Stop | HookEvent::UserPromptSubmit => {
            basic_turn_event(dialect, event, payload.as_ref(), started)
        }
        HookEvent::SessionStart => (
            guidance::stdout(
                agent,
                &guidance::session_context(workspace_context::live_context().as_deref()),
            ),
            None,
        ),
        _ => (None, None),
    })
    .unwrap_or_else(|_| {
        if event == HookEvent::PreToolUse {
            guard_failed(arguments, "panic");
        }
        (None, None)
    });
    let answer = match answer {
        None if dialect == HookDialect::Cursor && basic::cursor_permission_event(event) => {
            Some(basic::CURSOR_ALLOW.to_owned())
        }
        answer => answer,
    };
    let printed = answer.is_some_and(|answer| print_line(&answer));
    // The letters the answer carries are received once it reached the agent:
    // a flush that failed leaves them pending for the next event.
    if let Some(intake) = intake.filter(|_| printed)
        && let Err(failure) = hide_agent_hooks::delivery::confirm(&intake, started + GUARD_BUDGET)
        && let Some(home) = home_directory()
    {
        hide_agent_hooks::delivery::diagnose_failure(&home, &failure);
    }
    let _ = std::panic::catch_unwind(|| count_basic(dialect, event, payload.as_ref()));
}

fn basic_turn_event(
    dialect: HookDialect,
    event: HookEvent,
    payload: Option<&(Vec<u8>, bool)>,
    started: Instant,
) -> (Option<String>, Option<hide_agent_hooks::delivery::Intake>) {
    use hide_agent_hooks::delivery;
    let Some((payload, truncated)) = payload else {
        return (None, None);
    };
    if !std::env::var("HERDR_PANE_ID").is_ok_and(|pane| !pane.is_empty()) {
        return (None, None);
    }
    let result = (|| match event {
        HookEvent::UserPromptSubmit
            if dialect
                .adapter()
                .prompt_hook
                .is_some_and(|hook| !hook.context) =>
        {
            let prompt = delivery::read_prompt(payload, *truncated)?;
            if prompt.session.is_none() {
                return Ok(None);
            }
            // Observe-only hooks never print or confirm, even if an older
            // peer accidentally answers with context.
            delivery::pull(started + GUARD_BUDGET, &prompt).map(|_| None)
        }
        HookEvent::Stop
            if dialect.adapter().turn_end_letters.is_some()
                && matches!(
                    basic::grok_stop(payload, *truncated),
                    basic::GrokStop::Running {
                        hook_active: false,
                        ..
                    }
                ) =>
        {
            let Some(session) = basic::session(payload, *truncated, dialect)? else {
                return Ok(None);
            };
            delivery::pull_at_turn_end(started + GUARD_BUDGET, &session)
        }
        _ => Ok(None),
    })();
    match result {
        Ok(Some(intake)) => (
            Some(serde_json::json!({"decision":"block", "reason":intake.context}).to_string()),
            Some(intake),
        ),
        Ok(None) => (None, None),
        Err(failure) => {
            if let Some(home) = home_directory() {
                delivery::diagnose_failure(&home, &failure);
            }
            (None, None)
        }
    }
}

/// A Grok or Cursor tool call: the guards' refusal in the agent's shape, or,
/// when the call runs, the letters due while the agent works as the context
/// the agent reads with the tool's result (PRD letters-to-busy-agents D-02),
/// asked in a Herdr pane for the session the payload names and confirmed by
/// the caller once printed. Only the tools the agent's matcher names reach
/// this hook (`guidance`), so a letter waits through the others.
fn basic_tool_call(
    dialect: HookDialect,
    runtime_id: &str,
    started: Instant,
) -> (Option<String>, Option<hide_agent_hooks::delivery::Intake>) {
    let Some((payload, truncated)) = read_stdin_before_deadline(started + GUARD_PAYLOAD_BUDGET)
    else {
        if let Some(home) = home_directory() {
            hide_agent_hooks::guardrails::record_failure(&home, runtime_id, "payload_deadline");
        }
        return (None, None);
    };
    if let Some(reason) = guard_refusal(dialect, runtime_id, started, &payload, truncated) {
        let answer = match dialect {
            HookDialect::Cursor => basic::cursor_deny(&reason),
            // Grok takes Claude Code's refusal envelope.
            _ => hide_agent_hooks::spawn_guard::deny_output(&reason),
        };
        return (Some(answer), None);
    }
    let in_pane = std::env::var("HERDR_PANE_ID").is_ok_and(|value| !value.is_empty());
    if !in_pane || dialect.adapter().tool_letters.is_none() {
        return (None, None);
    }
    let session = match basic::session(&payload, truncated, dialect) {
        Ok(Some(session)) => session,
        Ok(None) => return (None, None),
        Err(failure) => {
            if let Some(home) = home_directory() {
                hide_agent_hooks::delivery::diagnose_failure(&home, &failure);
            }
            return (None, None);
        }
    };
    match hide_agent_hooks::delivery::pull_at_tool(started + GUARD_BUDGET, &session) {
        Ok(Some(intake)) => (
            Some(basic::context_answer(dialect, &intake.context)),
            Some(intake),
        ),
        Ok(None) => (None, None),
        Err(failure) => {
            if let Some(home) = home_directory() {
                hide_agent_hooks::delivery::diagnose_failure(&home, &failure);
            }
            (None, None)
        }
    }
}

/// agy's hook (PRD antigravity-first-class; `hide_agent_hooks::antigravity`):
/// the guidance and the Memory index when a conversation starts, the prompt's
/// Memory cards and the pane's letters on a prompt's first model call, the
/// guards on a tool call and the subagent count. A tool call is answered on
/// every path, since agy refuses one whose hook exits nonzero, prints what it
/// cannot read, times out or is missing, and
/// nothing here ends the hook with another code.
fn run_antigravity_hook(arguments: &[String], started: Instant) {
    let Some(event) =
        argument_value("--event", arguments).and_then(|value| HookEvent::parse(&value))
    else {
        return;
    };
    if event == HookEvent::PreToolUse {
        // A payload that does not arrive in time refuses nothing: the call
        // is left to agy's own rules, and the miss is recorded.
        let refusal = std::panic::catch_unwind(|| {
            let (payload, truncated) = read_stdin_before_deadline(started + GUARD_PAYLOAD_BUDGET)?;
            guard_refusal(
                HookDialect::Antigravity,
                GuidanceAgent::Antigravity.id(),
                started,
                &payload,
                truncated,
            )
        })
        .unwrap_or_else(|_| {
            guard_failed(arguments, "panic");
            None
        });
        print_line(&refusal.map_or_else(
            || antigravity::ASK.to_owned(),
            |reason| antigravity::deny(&reason),
        ));
        return;
    }
    let _ = std::panic::catch_unwind(|| antigravity_event(event, started));
}

/// agy's non-tool events, each reading the payload once.
fn antigravity_event(event: HookEvent, started: Instant) {
    let Some(home) = home_directory() else { return };
    let Some((payload, truncated)) = read_stdin_before_deadline(started + PROMPT_PAYLOAD_BUDGET)
    else {
        return;
    };
    let Some(conversation) = antigravity::Event::read(&payload, truncated).conversation else {
        return;
    };
    let runtime = GuidanceAgent::Antigravity.id();
    // A subagent's conversation opens with its parent's message, which agy
    // writes as it starts it; the start waits a moment for that line.
    let parent_wait = if event == HookEvent::SessionStart {
        Duration::from_millis(300)
    } else {
        Duration::ZERO
    };
    let parent = antigravity::root_of(&home, &conversation, Instant::now() + parent_wait);
    let reporter = |session| Reporter {
        agent: Some(runtime),
        session: Some(session),
    };
    match (event, parent.as_deref()) {
        // A subagent gets nothing of the pane's: no guidance, no Memory and
        // no letter, which are the pane's own session's. Its start and end,
        // a nested subagent's too, are counted under the pane's own
        // conversation, the session Herdr reports for the pane.
        (HookEvent::SessionStart, Some(parent)) => report_count(
            &home,
            event,
            Change::Started(Some(&conversation)),
            reporter(parent),
            true,
        ),
        (HookEvent::Stop, Some(parent)) => report_count(
            &home,
            event,
            Change::Stopped(Some(&conversation)),
            reporter(parent),
            true,
        ),
        (_, Some(_)) => {}
        (HookEvent::SessionStart, None) => {
            let memory_deadline = started + hide_agent_hooks::memory::MEMORY_BUDGET;
            let (index, workspace) = std::thread::scope(|scope| {
                let probing = std::thread::Builder::new()
                    .name("hide-hook-workspace".to_owned())
                    .spawn_scoped(scope, workspace_context::live_context);
                let index = antigravity_memory(
                    &home,
                    HookEvent::SessionStart,
                    &conversation,
                    None,
                    memory_deadline,
                );
                let workspace = match probing {
                    Ok(probe) => probe.join().ok().flatten(),
                    Err(_) => workspace_context::live_context(),
                };
                (index, workspace)
            });
            let mut context = guidance::session_context(workspace.as_deref());
            if let Some(index) = index {
                context.push_str(
                    "

",
                );
                context.push_str(&index);
            }
            if let Some(answer) = antigravity::inject(&context) {
                print_line(&answer);
            }
            report_count(
                &home,
                event,
                Change::SessionStarted,
                reporter(&conversation),
                true,
            );
        }
        // Every model call of a turn runs `PreInvocation`; the prompt's
        // first is the prompt hook, and a later one is the gap after a tool,
        // where the letters due while the agent works are handed over (PRD
        // letters-to-busy-agents D-02).
        (HookEvent::UserPromptSubmit, None) => {
            if antigravity::Event::read(&payload, truncated).invocation == Some(0) {
                antigravity_prompt(&home, &conversation, started);
            } else {
                antigravity_tool_gap(&home, &conversation, started);
            }
        }
        (HookEvent::Stop, None) => {
            report_count(&home, event, Change::None, reporter(&conversation), true)
        }
        (
            HookEvent::PreToolUse
            | HookEvent::PostToolUse
            | HookEvent::PostToolUseFailure
            | HookEvent::SubagentStart
            | HookEvent::SubagentStop,
            None,
        ) => {}
    }
}

/// The prompt's Memory cards and the pane's waiting letters, asked at once,
/// injected as one system message; a letter is confirmed only after that
/// answer reached agy.
fn antigravity_prompt(home: &std::path::Path, conversation: &str, started: Instant) {
    let memory_deadline = started + hide_agent_hooks::memory::MEMORY_BUDGET;
    let delivery_deadline = started + INTAKE_BUDGET;
    let prompt = antigravity::last_prompt(home, conversation);
    let (cards, intake) = std::thread::scope(|scope| {
        let asking = std::thread::Builder::new()
            .name("hide-hook-memory".to_owned())
            .spawn_scoped(scope, || {
                antigravity_memory(
                    home,
                    HookEvent::UserPromptSubmit,
                    conversation,
                    prompt.clone(),
                    memory_deadline,
                )
            });
        let intake = hide_agent_hooks::delivery::pull(
            delivery_deadline,
            &hide_agent_hooks::delivery::Prompt {
                digest: None,
                session: Some(conversation.to_owned()),
            },
        )
        .unwrap_or_else(|failure| {
            hide_agent_hooks::delivery::diagnose_failure(home, &failure);
            None
        });
        let cards = match asking {
            Ok(asked) => asked.join().unwrap_or_else(|_| {
                hide_agent_hooks::memory::record_panic(home);
                None
            }),
            Err(_) => None,
        };
        (cards, intake)
    });
    let mut context = cards.unwrap_or_default();
    if let Some(intake) = &intake {
        context.push_str(&intake.context);
    }
    let Some(answer) = antigravity::inject(&context) else {
        return;
    };
    let answer = if cfg!(windows) {
        hide_agent_hooks::runtime::ascii_json(&answer)
    } else {
        answer
    };
    let mut stdout = std::io::stdout().lock();
    let flushed = writeln!(stdout, "{answer}")
        .and_then(|_| stdout.flush())
        .is_ok();
    drop(stdout);
    if let Some(intake) = intake.filter(|intake| !intake.ids.is_empty()) {
        if flushed {
            if let Err(failure) = hide_agent_hooks::delivery::confirm(&intake, delivery_deadline) {
                hide_agent_hooks::delivery::diagnose_failure(home, &failure);
            }
        } else {
            hide_agent_hooks::delivery::diagnose(home, "stdout");
        }
    }
}

/// A model call after a tool of agy's turn: the letters due while it works,
/// injected as a system message the model reads before its next step, and
/// confirmed once printed, as Grok's and Cursor's tool hook confirms. Nothing
/// due, a failed ask or a pane the helper is not in answer nothing, and agy
/// runs the call as it would.
fn antigravity_tool_gap(home: &std::path::Path, conversation: &str, started: Instant) {
    let in_pane = std::env::var("HERDR_PANE_ID").is_ok_and(|value| !value.is_empty());
    if !in_pane {
        return;
    }
    let delivery_deadline = started + INTAKE_BUDGET;
    let intake = match hide_agent_hooks::delivery::pull_at_tool(delivery_deadline, conversation) {
        Ok(Some(intake)) => intake,
        Ok(None) => return,
        Err(failure) => {
            hide_agent_hooks::delivery::diagnose_failure(home, &failure);
            return;
        }
    };
    let Some(answer) = antigravity::inject(&intake.context) else {
        return;
    };
    let answer = if cfg!(windows) {
        hide_agent_hooks::runtime::ascii_json(&answer)
    } else {
        answer
    };
    let mut stdout = std::io::stdout().lock();
    let flushed = writeln!(stdout, "{answer}")
        .and_then(|_| stdout.flush())
        .is_ok();
    drop(stdout);
    if intake.ids.is_empty() {
        return;
    }
    if flushed {
        if let Err(failure) = hide_agent_hooks::delivery::confirm(&intake, delivery_deadline) {
            hide_agent_hooks::delivery::diagnose_failure(home, &failure);
        }
    } else {
        hide_agent_hooks::delivery::diagnose(home, "stdout");
    }
}

/// Memory's context for agy's `event` in `conversation`, the session agy's
/// transcript, and so Memory's receipts, know it by.
fn antigravity_memory(
    home: &std::path::Path,
    event: HookEvent,
    conversation: &str,
    prompt: Option<String>,
    deadline: Instant,
) -> Option<String> {
    hide_agent_hooks::memory::memory_context_until(
        hide_agent_hooks::memory::MemoryRequest {
            runtime_id: GuidanceAgent::Antigravity.id(),
            event,
            prompt,
            session_id: Some(conversation.to_owned()),
        },
        home,
        deadline,
    )
    .joined()
}

/// The count a Grok or Cursor event makes. Grok's `Stop` names the
/// background subagents still running and comes from inside a subagent too,
/// which leaves the count alone; Cursor's subagents end with the turn.
fn count_basic(dialect: HookDialect, event: HookEvent, payload: Option<&(Vec<u8>, bool)>) {
    // An agent whose adapter declares no count reports none, rather than a
    // zero its hooks never changed (Cursor, issue 911).
    if dialect.adapter().subagent_counts.is_none() {
        return;
    }
    let change = match (dialect, event) {
        (_, HookEvent::SessionStart) => Change::SessionStarted,
        (_, HookEvent::SubagentStart) => Change::Started(None),
        (_, HookEvent::SubagentStop) => Change::Stopped(None),
        (HookDialect::Grok, HookEvent::Stop) => {
            let Some((payload, truncated)) = payload else {
                return;
            };
            match basic::grok_stop(payload, *truncated) {
                basic::GrokStop::Ignore => return,
                basic::GrokStop::Running { running, .. } => Change::Settled { running },
            }
        }
        (_, HookEvent::Stop) => Change::Settled { running: 0 },
        (
            _,
            HookEvent::UserPromptSubmit
            | HookEvent::PreToolUse
            | HookEvent::PostToolUse
            | HookEvent::PostToolUseFailure,
        ) => return,
    };
    let Some(home) = home_directory() else { return };
    // Neither agent's hook input is read for its session here, so the file
    // names the agent and no session, and a restore withholds the counts.
    let who = Reporter {
        agent: Some(dialect.adapter().id),
        session: None,
    };
    // No session is read, so there is none to compare with Herdr's.
    report_count(&home, event, change, who, false);
}

/// Prints one JSON answer. On Windows every agent runs its hook under
/// PowerShell, which re-encodes what a program prints in the console code
/// page, so the answer goes out as ASCII with `\u` escapes, as the runtime
/// hooks' does.
/// Prints one answer line; whether it was written and flushed.
fn print_line(json: &str) -> bool {
    let json = if cfg!(windows) {
        hide_agent_hooks::runtime::ascii_json(json)
    } else {
        json.to_owned()
    };
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{json}")
        .and_then(|_| stdout.flush())
        .is_ok()
}

/// The spawn guard (`hide_agent_hooks::spawn_guard`) of Claude Code's and
/// Codex's hook: it prints Claude Code's refusal for a shell call that starts
/// an agent through Herdr, and nothing on every other path.
fn run_spawn_guard(arguments: &[String], started: Instant) {
    let Some(runtime) =
        argument_value("--runtime", arguments).and_then(|value| AgentRuntime::parse(&value))
    else {
        return;
    };
    if runtime.dialect().adapter().spawn_guard.is_none() {
        return;
    }
    // Another agent that runs Claude Code's hooks (Grok, OpenCode, Cursor)
    // speaks through its own hook, or Hide has not verified its contract.
    if hide_agent_hooks::runtime::ForeignOrigin::detect(|name| std::env::var_os(name)).is_some() {
        return;
    }
    let Some((payload, truncated)) = read_stdin_before_deadline(started + GUARD_PAYLOAD_BUDGET)
    else {
        guard_failed(arguments, "payload_deadline");
        return;
    };
    if let Some(reason) = guard_refusal(
        runtime.dialect(),
        runtime.id(),
        started,
        &payload,
        truncated,
    ) {
        print_line(&hide_agent_hooks::spawn_guard::deny_output(&reason));
    }
}

/// The hook failed before it judged a tool call (`cause`: `panic`), so the
/// call runs as the agent's default; the guardrails' log says so (B21).
fn guard_failed(arguments: &[String], cause: &str) {
    let runtime = argument_value("--runtime", arguments).unwrap_or_default();
    if let Some(home) = home_directory() {
        hide_agent_hooks::guardrails::record_failure(&home, &runtime, cause);
    }
}

/// The guards' decision for one tool call: the reason it is refused, or
/// `None` to let it run. The project's guardrails refuse what a protection
/// that is on names; the spawn guard refuses a launch through Herdr (not a
/// launch, not in a registered checkout, the daemon unreachable and any
/// failure of its own let it run). A refusal and an unreachable daemon are
/// written to the guard's log here; the caller prints the answer in its
/// runtime's form.
fn guard_refusal(
    dialect: HookDialect,
    runtime_id: &str,
    _started: Instant,
    payload: &[u8],
    truncated: bool,
) -> Option<String> {
    use hide_agent_hooks::spawn_guard::{self as guard, Registration};

    let pane = std::env::var("HERDR_PANE_ID")
        .ok()
        .filter(|value| !value.is_empty());
    let home = home_directory();
    // The project's guardrails come first, in any session of a guarded
    // project, a Herdr pane or not (PRD agent-guardrails B22): they read only
    // local files, and a call they refuse never reaches the daemon.
    if let Some(home) = &home
        && let Some(reason) =
            hide_agent_hooks::guardrails::refusal_for(home, runtime_id, pane.as_deref(), || {
                hide_agent_hooks::guardrails::payload::read_hook(payload, truncated, dialect)
            })
    {
        return Some(reason);
    }
    // A plan waiting for review, a mission's scope and Hide's own history
    // rule, judged from the pane's record alone: no call to the daemon
    // (PRD hide-plan D-15, D-30).
    if let (Some(home), Some(pane)) = (&home, &pane)
        && let Some(reason) = plan_refusal(home, pane, runtime_id, dialect, payload, truncated)
    {
        return Some(reason);
    }
    // Outside a Herdr pane there is nothing to redirect.
    let pane = pane?;
    // Every shell call comes through here: this one search decides the rest.
    if !guard::may_hold_launch(payload) {
        return None;
    }
    let call = guard::read_call(payload, truncated, dialect)?;
    let launch = guard::find_launch(&call.command, &|name| std::env::var(name).ok())?;
    let home = home_directory()?;
    let program = workspace_context::cli_program()?;
    match guard::registration(&program, Instant::now() + GUARD_BUDGET) {
        Registration::Registered => {}
        Registration::NotRegistered => return None,
        Registration::Unreachable(cause) => {
            guard::unreachable(&home, runtime_id, cause);
            return None;
        }
    }
    let cwd = call
        .cwd
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    let (repo, branch) = guard::checkout_facts(&cwd);
    let delegation = guard::spawn_command(&launch, repo.as_deref(), branch.as_deref(), true);
    let handoff = guard::spawn_command(&launch, repo.as_deref(), branch.as_deref(), false);
    // The refusal is logged before it is printed, and the refusal stands even
    // when the log cannot be written.
    guard::record_refusal(&home, runtime_id, &pane, &launch);
    Some(guard::refusal_reason(&delegation, &handoff))
}

/// The refusal the pane's plan record names for this call, logged like
/// the guardrails' refusals. A pane with no record, or a record of another
/// session than the call's, refuses nothing.
fn plan_refusal(
    home: &std::path::Path,
    pane: &str,
    runtime_id: &str,
    dialect: HookDialect,
    payload: &[u8],
    truncated: bool,
) -> Option<String> {
    let session = basic::session(payload, truncated, dialect).ok().flatten();
    let context = hide_agent_hooks::plan_record::read(home, pane, session.as_deref())?;
    let call = hide_agent_hooks::guardrails::payload::read_hook(payload, truncated, dialect)?;
    let refused = hide_agent_hooks::plan_record::refusal(&context, &call.action)?;
    hide_agent_hooks::plan_record::record_refusal(home, runtime_id, pane, refused);
    Some(refused.reason().to_owned())
}

fn run_prompt_hook(arguments: &[String], deadline: Instant) {
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    let mut command = Command::new(executable);
    command
        .arg("hook-inner")
        .args(&arguments[1..])
        .stdin(Stdio::inherit())
        // The inner flush must reach the runtime before it confirms intake.
        // Buffering and re-emitting here would acknowledge undelivered output.
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    if let Ok(mut child) = OwnedChild::spawn_guarded(command, deadline) {
        let _ = child.capture_until(deadline, 64 * 1024);
    }
    // No filesystem or stream write follows the deadline. Failure keeps the
    // manual-inbox fallback; diagnostics run inside the supervised operation.
}

fn run_hook(arguments: &[String], started: Instant) {
    let Some(event) =
        argument_value("--event", arguments).and_then(|value| HookEvent::parse(&value))
    else {
        return;
    };
    let runtime =
        argument_value("--runtime", arguments).and_then(|value| AgentRuntime::parse(&value));
    let Some(home) = home_directory() else { return };
    // Memory and the letters are asked of the core at once, each by its own
    // `hide`, and both answers are in before the one write; Memory ends where
    // the letter pull does, so it never makes the turn wait longer.
    let memory_deadline = started + hide_agent_hooks::memory::MEMORY_BUDGET;
    let delivery_deadline = started + INTAKE_BUDGET;
    let memory_injection = arguments
        .iter()
        .any(|argument| argument == "--memory-injection")
        && runtime.is_some_and(|runtime| runtime.dialect().adapter().memory.is_some());
    let prompt_hook =
        runtime.is_some_and(|runtime| runtime.dialect().adapter().prompt_hook.is_some());
    // The prompt event reads its payload for the prompt digest even without
    // Memory; a Memory read works from the same bytes.
    // A count reads the same input: the session it came from, the subagent it
    // is about and the subagents a turn end lists.
    let counts = event != HookEvent::UserPromptSubmit
        && !runtime.is_some_and(|runtime| runtime.dialect().adapter().subagent_counts.is_none());
    let payload = if (event == HookEvent::UserPromptSubmit && runtime.is_some())
        || memory_injection
        || counts
    {
        read_stdin_before_deadline(started + PROMPT_PAYLOAD_BUDGET)
    } else {
        None
    };
    let prompt = match (&payload, event) {
        (Some((bytes, truncated)), HookEvent::UserPromptSubmit) => {
            match hide_agent_hooks::delivery::read_prompt(bytes, *truncated) {
                Ok(prompt) => prompt,
                Err(failure) => {
                    hide_agent_hooks::delivery::diagnose_failure(&home, &failure);
                    return;
                }
            }
        }
        _ => hide_agent_hooks::delivery::Prompt::default(),
    };
    // Claude Code's hook inside Grok leaves the letters where they are: the
    // session that runs it is not the pane's Claude Code.
    let takes_letters = runtime != Some(AgentRuntime::ClaudeCode)
        || hide_agent_hooks::runtime::takes_letters(|name| std::env::var_os(name));
    let pulls = event == HookEvent::UserPromptSubmit && prompt_hook && takes_letters;
    let memory_runtime = runtime.filter(|_| memory_injection);
    let memory = |runtime| {
        memory_output_before_deadline(runtime, event, &home, memory_deadline, payload.as_ref())
    };
    let (memory_output, intake, workspace) = std::thread::scope(|scope| {
        let asking = memory_runtime.map(|runtime| {
            std::thread::Builder::new()
                .name("hide-hook-memory".to_owned())
                .spawn_scoped(scope, move || memory(runtime))
                .map_err(|_| runtime)
        });
        // SessionStart's Workspace probe is asked beside Memory, not after
        // it: neither answer depends on the other, and each `hide` it runs
        // keeps its own deadline. The probe is not gated on a pane id: hided
        // binds a caller outside any pane (a Codex shared daemon, a plain
        // terminal) to the registered checkout holding its cwd, and refuses
        // everything else itself.
        let probing = (event == HookEvent::SessionStart).then(|| {
            std::thread::Builder::new()
                .name("hide-hook-workspace".to_owned())
                .spawn_scoped(scope, workspace_context::live_context)
        });
        let intake = if pulls {
            match hide_agent_hooks::delivery::pull(delivery_deadline, &prompt) {
                Ok(intake) => intake,
                Err(failure) => {
                    hide_agent_hooks::delivery::diagnose_failure(&home, &failure);
                    None
                }
            }
        } else {
            None
        };
        let memory_output = match asking {
            Some(Ok(asked)) => asked.join().unwrap_or_else(|_| {
                hide_agent_hooks::memory::record_panic(&home);
                None
            }),
            // No thread to ask on: the pull has had its turn, and Memory's
            // deadline still bounds this one.
            Some(Err(runtime)) => memory(runtime),
            None => None,
        };
        let workspace = match probing {
            // A probe that panicked has printed why and leaves the session
            // without the guidance, as a probe that failed does.
            Some(Ok(probe)) => probe.join().ok().flatten(),
            // No thread to ask on: the probe runs after Memory, as it did
            // before it had one.
            Some(Err(_)) => workspace_context::live_context(),
            None => None,
        };
        (memory_output, intake, workspace)
    });
    let mut output =
        memory_output.or_else(|| runtime.and_then(|runtime| hook_stdout(runtime, event)));
    if let Some(context) = workspace {
        output = output.map(|value| {
            hide_agent_hooks::runtime::append_session_context(&value, &context).unwrap_or(value)
        });
    }
    if let Some(intake) = &intake {
        output = match output {
            Some(existing) => {
                hide_agent_hooks::runtime::append_session_context(&existing, &intake.context)
            }
            None => runtime.and_then(|runtime| {
                hide_agent_hooks::runtime::hook_stdout_with_context(
                    runtime,
                    event,
                    Some(&intake.context),
                )
            }),
        };
    }
    let mut flushed = false;
    if let Some(output) = output {
        // PowerShell runs the hook on Windows and re-encodes what it prints.
        let output = if cfg!(windows) {
            hide_agent_hooks::runtime::ascii_json(&output)
        } else {
            output
        };
        let mut stdout = std::io::stdout().lock();
        flushed = writeln!(stdout, "{output}")
            .and_then(|_| stdout.flush())
            .is_ok();
    }
    if event == HookEvent::UserPromptSubmit {
        // A count-only answer has no letter to confirm.
        if let Some(intake) = intake.filter(|intake| !intake.ids.is_empty()) {
            if flushed {
                if let Err(failure) =
                    hide_agent_hooks::delivery::confirm(&intake, delivery_deadline)
                {
                    hide_agent_hooks::delivery::diagnose_failure(&home, &failure);
                }
            } else {
                hide_agent_hooks::delivery::diagnose(&home, "stdout");
            }
        }
        return;
    }
    if !counts {
        return;
    }
    // The session the event came from, when its input was read whole: what
    // tells the pane's own session from another one's, and a restore this
    // session's counts from another one's.
    let input = payload
        .as_ref()
        .map(|(bytes, truncated)| SubagentInput::read(bytes, *truncated))
        .unwrap_or_default();
    let who = Reporter {
        agent: runtime.map(AgentRuntime::id),
        session: input.session.as_deref(),
    };
    report_count(&home, event, Change::of(runtime, event, &input), who, true);
}

/// Applies `change` to this pane's count and reports the pane's totals to
/// Herdr. Outside a Herdr pane there is no pane to describe.
///
/// With `compare_session`, the event's session is compared with the one Herdr
/// reports for the pane's agent, and an event of another session (a nested
/// `claude -p` that a user's own hook starts runs these hooks with the pane's
/// environment) changes and reports nothing. A Herdr that names no session,
/// or cannot say, leaves every event the pane's.
fn report_count(
    home: &std::path::Path,
    event: HookEvent,
    change: Change<'_>,
    who: Reporter<'_>,
    compare_session: bool,
) {
    let Some(pane_id) = std::env::var("HERDR_PANE_ID")
        .ok()
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    // `--source` on the command line is the install marker the diagnosis
    // reads out of the hook file; the report's own source is fixed in
    // `report::metadata_source`, because Herdr refuses the marker's `@`.
    let socket_path = report::socket_path(home);
    // Herdr is asked only when the change needs the answer, and a Herdr that
    // cannot say leaves every event the pane's: that fallback is written down
    // below, where `doctor` and Settings read the last report's outcome, so a
    // pane that counts every session is not silent about it.
    let mut session_failure = None;
    let applied = counters::change_for_pane(home, &pane_id, change, who, || {
        let path = socket_path.as_ref().ok().filter(|_| compare_session)?;
        match report::pane_session(path, &pane_id) {
            Ok(session) => session,
            Err(error) => {
                session_failure = Some(error);
                None
            }
        }
    });
    if applied.as_ref().is_ok_and(|applied| applied.foreign) {
        return;
    }
    let mut outcome = match (applied.map(|applied| applied.counters), &socket_path) {
        (Ok(counters), Ok(path)) => report_latest(home, path, &pane_id, counters),
        (Err(error), _) => Err(hide_herdr_client::ApiError::Transport(format!(
            "the pane's count could not be changed: {error}"
        ))),
        (_, Err(error)) => Err(error.clone()),
    };
    if let (Ok(()), Some(error)) = (&outcome, session_failure) {
        outcome = Err(hide_herdr_client::ApiError::Transport(format!(
            "Herdr did not say which session is the pane's agent, so every session was counted as the pane's: {error}"
        )));
    }
    // The outcome is written down rather than surfaced here: a hook's
    // stderr reaches nobody, and the record is what `doctor` and Settings
    // show (engineering rule 10).
    let socket_path = socket_path.unwrap_or_default();
    let _ = report::record_outcome(home, &pane_id, event.name(), &socket_path, &outcome);
}

/// Reports `counters` and, when another event of the pane changed the record
/// while this report was on its way, the record as it is now: two events
/// that report in the other order than they counted would otherwise leave
/// Herdr with the older count until the pane's next event.
fn report_latest(
    home: &std::path::Path,
    socket_path: &std::path::Path,
    pane_id: &str,
    counters: counters::PaneCounters,
) -> Result<(), hide_herdr_client::ApiError> {
    report::report(socket_path, pane_id, counters)?;
    // Only a record actually read can be newer; one that cannot be read now
    // is left to the event that is writing it, which reports it itself.
    match counters::read_settled(home, pane_id) {
        Ok(now) if now != counters => report::report(socket_path, pane_id, now),
        _ => Ok(()),
    }
}

fn memory_output_before_deadline(
    runtime: AgentRuntime,
    event: HookEvent,
    home: &std::path::Path,
    deadline: Instant,
    payload: Option<&(Vec<u8>, bool)>,
) -> Option<String> {
    let Some((payload, exceeded)) = payload else {
        return hook_stdout(runtime, event);
    };
    hide_agent_hooks::memory::project_memory_output_until(
        runtime, event, payload, *exceeded, home, deadline,
    )
    .stdout
    .or_else(|| hook_stdout(runtime, event))
}

/// The hook's event from stdin, read until EOF, the size cap or `deadline`.
/// Leading UTF-8 byte order marks are dropped: the Cursor CLI on Windows
/// pipes the event through PowerShell, which writes one before the JSON
/// (Cursor's forum, threads 166794 and 168407), and Windows PowerShell 5.1
/// writes one more at each pipe the event crosses, so Hide's own `$input |`
/// inside Cursor's wrapper makes two; JSON allows none.
fn read_stdin_before_deadline(deadline: Instant) -> Option<(Vec<u8>, bool)> {
    const MARK: &[u8] = b"\xEF\xBB\xBF";
    let (mut payload, truncated) = read_raw_stdin_before_deadline(deadline)?;
    let marks = payload
        .chunks(MARK.len())
        .take_while(|chunk| *chunk == MARK)
        .count();
    payload.drain(..marks * MARK.len());
    Some((payload, truncated))
}

#[cfg(unix)]
#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn read_raw_stdin_before_deadline(deadline: Instant) -> Option<(Vec<u8>, bool)> {
    use std::os::fd::AsRawFd;

    let stdin = std::io::stdin();
    let input = stdin.lock();
    let descriptor_number = input.as_raw_fd();
    // SAFETY: F_GETFL does not dereference a pointer and the descriptor is
    // borrowed from the live stdin lock.
    let flags = unsafe { libc::fcntl(descriptor_number, libc::F_GETFL) };
    if flags < 0 {
        return Some((Vec::new(), false));
    }
    // SAFETY: F_SETFL changes only this process's descriptor flags. The hook
    // owns stdin for its whole short lifetime, so no other reader observes it.
    if unsafe { libc::fcntl(descriptor_number, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Some((Vec::new(), false));
    }
    let mut retained = Vec::with_capacity(hide_memory::HOOK_INPUT_LIMIT_BYTES.min(16 * 1024));
    let mut buffer = [0_u8; 8192];
    loop {
        let remaining = deadline.checked_duration_since(Instant::now())?;
        // SAFETY: `buffer` is a writable byte array and `descriptor_number`
        // is the live stdin descriptor held by `input`.
        let read = unsafe {
            libc::read(
                descriptor_number,
                buffer.as_mut_ptr().cast::<libc::c_void>(),
                buffer.len(),
            )
        };
        if read < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::WouldBlock {
                std::thread::sleep(remaining.min(Duration::from_millis(2)));
                continue;
            }
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Some((Vec::new(), false));
        }
        let read = read as usize;
        if read == 0 {
            return Some((retained, false));
        }
        let remaining_capacity = hide_memory::HOOK_INPUT_LIMIT_BYTES.saturating_sub(retained.len());
        let keep = remaining_capacity.min(read);
        retained.extend_from_slice(&buffer[..keep]);
        if keep < read {
            return Some((retained, true));
        }
    }
}

/// Windows has no nonblocking read of an anonymous pipe, so a thread reads
/// and this waits for it until the deadline; a producer that never closes
/// stdin leaves that thread blocked, and the process exits without it.
#[cfg(not(unix))]
fn read_raw_stdin_before_deadline(deadline: Instant) -> Option<(Vec<u8>, bool)> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(hide_agent_hooks::memory::read_bounded(std::io::stdin()));
    });
    match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(Ok(read)) => Some(read),
        Ok(Err(_)) => Some((Vec::new(), false)),
        Err(_) => None,
    }
}

fn run_doctor(arguments: &[String]) -> Result<String, String> {
    let home = home_directory().ok_or_else(|| "HOME is not set".to_owned())?;
    let search = hide_platform::programs::account_path()
        .ok_or_else(|| "this account's program search could not be read".to_owned())?;
    let diagnosis = Diagnosis::read(&home, &search);
    if arguments.iter().any(|argument| argument == "--json") {
        serde_json::to_string_pretty(&diagnosis)
            .map_err(|error| format!("diagnosis could not be encoded: {error}"))
    } else {
        Ok(diagnosis.render())
    }
}

fn home_directory() -> Option<PathBuf> {
    hide_platform::host::home_dir().ok()
}

fn argument_value(flag: &str, arguments: &[String]) -> Option<String> {
    let index = arguments.iter().position(|argument| argument == flag)?;
    arguments.get(index + 1).cloned()
}
