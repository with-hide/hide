//! `hide-agent-hooks <agent> <operation>`: what Hide's script file in an
//! agent's own folder asks (`hide_agent_hooks::plugin`): OpenCode's plugin
//! (`opencode`), and Pi's and omp's extension (`pi`, `omp`). Each operation
//! reads one JSON object on stdin, prints one on stdout and exits 0; a failure
//! answers `{}`, the script then changes nothing, and the cause goes to the
//! same private diagnostic the other hooks use.
//!
//! The operations reuse the Claude Code and Codex hook's parts: the letter
//! intake (`delivery`), the guards (`guardrails`, `plan_record`,
//! `opener_guard`), the pane counts (`counters`, `report`) and Project Memory
//! (`memory`). What differs is only the transport: these agents have no
//! command hook, so the script hands over the facts a hook payload would
//! carry. What each agent gets follows its adapter declaration.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hide_agent_adapter::{AgentAdapter, HookDialect, PluginDialect};
use hide_agent_hooks::delivery::{self, Intake, Prompt};
use hide_agent_hooks::memory::{self, HookMemoryOutcome, MemoryRequest, memory_context_until};
use hide_agent_hooks::runtime::HookEvent;
use hide_agent_hooks::{counters, guardrails, guidance, report};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::workspace_context;

/// Inside the script's 1.85 s prompt budget, which also pays for this process
/// starting.
const PROMPT_BUDGET: Duration = Duration::from_millis(1_650);
/// Inside the script's 2.5 s tool budget.
const TOOL_BUDGET: Duration = Duration::from_millis(2_300);
/// Inside the script's 1.5 s budget for the letters at a tool result.
const LETTERS_BUDGET: Duration = Duration::from_millis(1_300);
const CONFIRM_BUDGET: Duration = Duration::from_millis(1_800);
const INPUT_LIMIT: u64 = 256 * 1024;

#[derive(Default, Deserialize)]
#[serde(default)]
struct Input {
    session_id: Option<String>,
    /// The agent's own id for the session, which Memory and its receipts know
    /// it by; Pi's and omp's extension names the session by its file for
    /// letters and sends this beside it.
    native_session: Option<String>,
    prompt: Option<String>,
    cwd: Option<PathBuf>,
    first: bool,
    /// Whether Memory's session-start capsule is still to be given; Pi's and
    /// omp's extension tracks it apart from the guidance, and OpenCode's
    /// plugin, which sends none, gives both on its first prompt.
    memory_first: Option<bool>,
    letters: Vec<String>,
    tool: Option<String>,
    command: Option<String>,
    /// A file tool's own arguments, as the host hands them to the script,
    /// for the guardrails (`hide_agent_hooks::guardrails::payload`).
    args: Option<Value>,
    working: u32,
    done: u32,
    /// The turn the script saw the session's own events start, end or wait
    /// in (the `turn` operation).
    turn: Option<report::HookTurn>,
    /// Helper calls the script skipped at its cap or gave up on since its last
    /// call: recorded here, since the script has no log of its own.
    lost: u32,
    /// The version of the script that asks; Pi's and omp's extension sends it,
    /// and one this build does not write is answered with nothing (B13).
    version: Option<u32>,
    /// The script only guards this session: it runs outside a pane Hide manages, or
    /// is an agent started in another agent's shell, which a pane variable
    /// it inherited does not make the pane's own (PRD agent-guardrails B22).
    guard_only: bool,
}

/// The agent a script speaks for: its adapter row and the runtime id the
/// guard log and Memory know it by.
struct Agent {
    adapter: &'static AgentAdapter,
    dialect: PluginDialect,
}

impl Agent {
    fn parse(name: &str) -> Option<Self> {
        let adapter = hide_agent_adapter::ADAPTERS
            .iter()
            .find(|row| row.id == name)?;
        match adapter.hook {
            hide_agent_adapter::HookInstall::Plugin(dialect) => Some(Self { adapter, dialect }),
            _ => None,
        }
    }

    fn runtime(&self) -> &'static str {
        self.adapter.id
    }

    /// The hook dialect its tool calls are read in.
    fn hook_dialect(&self) -> HookDialect {
        match self.dialect {
            PluginDialect::OpenCode => HookDialect::OpenCode,
            PluginDialect::Pi => HookDialect::Pi,
            PluginDialect::Omp => HookDialect::Omp,
        }
    }

    /// Whether `version` is the script this build writes. OpenCode's plugin
    /// sends none and is judged by the kit alone.
    fn current(&self, version: Option<u32>) -> bool {
        match self.dialect {
            PluginDialect::OpenCode => true,
            PluginDialect::Pi | PluginDialect::Omp => {
                version == Some(hide_agent_hooks::pi_extension::VERSION)
            }
        }
    }

    /// The session Memory knows: OpenCode's session id is already its own,
    /// while Pi's and omp's file path is not what their session reader keys a
    /// receipt by.
    fn memory_session(&self, input: &Input, session: &str) -> Option<String> {
        match self.dialect {
            PluginDialect::OpenCode => Some(session.to_owned()),
            PluginDialect::Pi | PluginDialect::Omp => input
                .native_session
                .clone()
                .filter(|id| delivery::valid_session(id)),
        }
    }
}

/// Answers one operation of the script of `agent` (`opencode`, `pi`, `omp`).
pub fn run(agent: &str, operation: Option<&str>, started: Instant) {
    let Some(home) = hide_platform::host::home_dir().ok() else {
        answer(&json!({}));
        return;
    };
    let mut bytes = Vec::new();
    let input = std::io::stdin()
        .take(INPUT_LIMIT)
        .read_to_end(&mut bytes)
        .ok()
        .and_then(|_| serde_json::from_slice::<Input>(&bytes).ok());
    let Some(input) = input else {
        delivery::diagnose(&home, "format");
        answer(&json!({}));
        return;
    };
    if input.lost > 0 {
        delivery::diagnose(&home, "plugin");
    }
    let Some(agent) = Agent::parse(agent).filter(|agent| agent.current(input.version)) else {
        answer(&json!({}));
        return;
    };
    let output = match operation {
        Some("start") => start(),
        Some("prompt") => prompt(&home, &agent, input, started + PROMPT_BUDGET),
        Some("confirm") => confirm(&home, input, started + CONFIRM_BUDGET),
        Some("tool") => tool(&home, &agent, input, started + TOOL_BUDGET),
        Some("letters") if agent.adapter.tool_letters.is_some() => {
            letters(&home, input, started + LETTERS_BUDGET)
        }
        Some("subagents") if agent.adapter.subagent_counts.is_some() => {
            subagents(&home, &agent, input, started)
        }
        Some("turn") => turn(&home, &agent, input, started),
        Some("session") => session_start(&home, &agent, input, started),
        _ => json!({}),
    };
    answer(&output);
}

fn answer(value: &Value) {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{value}").and_then(|_| stdout.flush());
}

fn pane() -> Option<String> {
    hide_agent_hooks::pane::from_env()
}

fn session(input: &Input) -> Option<String> {
    input
        .session_id
        .clone()
        .filter(|id| delivery::valid_session(id))
}

/// The guidance a session start gets, read once when the agent loads the
/// script or starts its session, and attached to each new root session's first
/// prompt.
fn start() -> Value {
    json!({
        "context": guidance::session_context(workspace_context::live_context().as_deref()),
    })
}

/// Memory and the letters waiting for this pane's agent, asked at once and
/// both in before the answer. Nothing is confirmed here: the script confirms
/// after the agent stored the prompt that carries them (`confirm`), so a
/// letter whose prompt was lost stays pending.
fn prompt(home: &Path, agent: &Agent, input: Input, deadline: Instant) -> Value {
    let Some(session) = session(&input) else {
        return json!({});
    };
    let memory_deadline = (Instant::now() + memory::MEMORY_BUDGET).min(deadline);
    let (memory, memory_start, intake) = std::thread::scope(|scope| {
        let asking = agent.adapter.memory.is_some().then(|| {
            std::thread::Builder::new()
                .name("hide-plugin-memory".to_owned())
                .spawn_scoped(scope, || {
                    prompt_memory(home, agent, &input, &session, memory_deadline)
                })
                .map_err(|_| ())
        });
        let intake = delivery::pull(
            deadline,
            &Prompt {
                digest: None,
                session: Some(session.clone()),
            },
        )
        .unwrap_or_else(|failure| {
            delivery::diagnose_failure(home, &failure);
            None
        });
        let (memory, start) = match asking {
            Some(Ok(asked)) => asked.join().unwrap_or_else(|_| {
                memory::record_panic(home);
                (None, false)
            }),
            Some(Err(())) | None => (None, false),
        };
        (memory, start, intake)
    });
    let mut context = memory.unwrap_or_default();
    let mut letters = Vec::new();
    if let Some(Intake {
        context: text, ids, ..
    }) = intake
    {
        context.push_str(&text);
        letters = ids;
    }
    json!({ "context": context, "letters": letters, "memory_start": memory_start })
}

/// The Memory capsule for this prompt, signed for the session Memory knows,
/// and whether the session-start index was answered, which tells the script
/// to stop asking for it once the message carrying this prompt is written.
/// The first prompt carries the index and the cards its own words match,
/// since these hosts have no hook of their own at session start.
fn prompt_memory(
    home: &Path,
    agent: &Agent,
    input: &Input,
    session: &str,
    deadline: Instant,
) -> (Option<String>, bool) {
    let Some(memory_session) = agent.memory_session(input, session) else {
        // A script of this build always sends the host's id; Memory without
        // it would be signed for a session no reader knows.
        delivery::diagnose(home, "plugin");
        return (None, false);
    };
    let request = |event| MemoryRequest {
        runtime_id: agent.runtime(),
        event,
        prompt: input.prompt.clone(),
        session_id: Some(memory_session.clone()),
    };
    let mut start = false;
    let mut context = String::new();
    // The Hide global instruction comes with the session-start ask, in its
    // own slot outside Memory's quota (PRD memory-self-improve D-31).
    let mut global = None;
    if input.memory_first.unwrap_or(input.first) {
        let index = memory_context_until(request(HookEvent::SessionStart), home, deadline);
        start = matches!(
            index.outcome,
            HookMemoryOutcome::Provided { .. } | HookMemoryOutcome::Empty
        );
        context.push_str(index.context.as_deref().unwrap_or_default());
        // Once a session: with the start guidance, or with the session-start
        // capsule when the guidance came first; Memory off answers no
        // capsule, so a later prompt does not carry it again.
        global = index.global.filter(|_| input.first || start);
    }
    let prompt = memory_context_until(request(HookEvent::UserPromptSubmit), home, deadline);
    let body = prompt.context.as_deref().unwrap_or_default();
    let (context, index_kept) = bounded_memory_capsule(&context, body);
    let context = match (global, context) {
        (Some(global), Some(context)) => Some(format!("{global}{context}")),
        (global, context) => global.or(context),
    };
    (context, start && index_kept)
}

/// Keep complete envelopes so a signed receipt never names a cut card.
/// Prompt bodies take priority; a deferred index is asked again next turn.
/// Letters are appended by the caller after this Memory-only quota.
fn bounded_memory_capsule(index: &str, bodies: &str) -> (Option<String>, bool) {
    use hide_memory::hook::{RENDERED_TOKEN_LIMIT, estimated_tokens};
    if estimated_tokens(bodies) > RENDERED_TOKEN_LIMIT {
        return (
            (!index.is_empty() && estimated_tokens(index) <= RENDERED_TOKEN_LIMIT)
                .then(|| index.to_owned()),
            true,
        );
    }
    let combined = format!("{index}{bodies}");
    if estimated_tokens(&combined) <= RENDERED_TOKEN_LIMIT {
        ((!combined.is_empty()).then_some(combined), true)
    } else {
        ((!bodies.is_empty()).then(|| bodies.to_owned()), false)
    }
}

/// The letters due while the pane's agent works, asked at one of its tool
/// results (PRD letters-to-busy-agents D-02): the script writes them with
/// the result and confirms them, as it does a prompt's, once the host stored
/// that message. Nothing is due for a resting agent or a letter still
/// waiting, and the answer then carries no context.
fn letters(home: &Path, input: Input, deadline: Instant) -> Value {
    let Some(session) = session(&input) else {
        return json!({});
    };
    match delivery::pull_at_tool(deadline, &session) {
        Ok(Some(Intake { context, ids, .. })) => json!({ "context": context, "letters": ids }),
        Ok(None) => json!({ "context": "", "letters": [] }),
        Err(failure) => {
            delivery::diagnose_failure(home, &failure);
            json!({})
        }
    }
}

fn confirm(home: &Path, input: Input, deadline: Instant) -> Value {
    if input.letters.is_empty() || !delivery::valid_letter_ids(&input.letters) {
        return json!({});
    }
    // One that misses is kept for the core as a prompt hook's is.
    match delivery::confirm_or_owe(home, &input.letters, input.session_id.as_deref(), deadline) {
        Ok(()) => json!({ "confirmed": input.letters }),
        Err(_) => json!({}),
    }
}

/// The project's guardrails for a shell or file tool, the opener guard for
/// the shell (`bash`, OpenCode 2's `shell`): `deny`
/// carries the reason the script refuses with, and every other outcome lets
/// the call run, as the Claude Code and Codex guards do.
fn tool(home: &Path, agent: &Agent, input: Input, deadline: Instant) -> Value {
    let runtime = agent.runtime();
    let pane = pane();
    let call = || {
        let tool = input.tool.as_deref()?;
        let args = input.args.clone().or_else(|| {
            input
                .command
                .as_ref()
                .map(|command| json!({ "command": command }))
        })?;
        Some(guardrails::payload::ToolCall {
            action: guardrails::payload::action(agent.hook_dialect(), tool, &args)?,
            cwd: input.cwd.clone(),
        })
    };
    // No agent writes the Hide global instruction (PRD memory-self-improve
    // D-07), in any session or project.
    if let Some(reason) =
        call().and_then(|call| hide_agent_hooks::global_guard::refusal(home, &call))
    {
        hide_agent_hooks::global_guard::record_refusal(home, runtime, pane.as_deref());
        return json!({ "deny": reason });
    }
    // The guardrails hold in any session of a guarded project, a pane or not
    // (PRD agent-guardrails B22); the guards after them only in one.
    if let Some(reason) = guardrails::refusal_for(home, runtime, pane.as_deref(), call) {
        return json!({ "deny": reason });
    }
    // The pane's plan record: a plan waiting for review, a mission's scope
    // and Hide's own history rule (PRD hide-plan D-15), read from the file
    // the prompt's pull wrote for the same session.
    if let Some(pane) = pane.as_deref().filter(|_| !input.guard_only)
        && let Some(context) =
            hide_agent_hooks::plan_record::read(home, pane, session(&input).as_deref())
        && let Some(refused) =
            call().and_then(|call| hide_agent_hooks::plan_record::refusal(&context, &call.action))
    {
        hide_agent_hooks::plan_record::record_refusal(home, runtime, pane, refused);
        return json!({ "deny": refused.reason() });
    }
    let Some(pane) = pane.filter(|_| !input.guard_only) else {
        return json!({});
    };
    let Some(program) = workspace_context::cli_program() else {
        return json!({});
    };
    match input.tool.as_deref() {
        Some(tool @ ("bash" | "shell")) => {
            let Some(command) = input.command.as_deref() else {
                return json!({});
            };
            let syntax = hide_session::call_shell::syntax(agent.hook_dialect(), tool);
            let refusal = hide_agent_hooks::opener_guard::pane_refusal(
                home, runtime, &pane, command, syntax, &program, deadline,
            );
            refusal.map_or_else(|| json!({}), |reason| json!({ "deny": reason }))
        }
        _ => json!({}),
    }
}

/// The pane's subagent counts as the script keeps them, stored and reported to
/// the core the way the Claude Code and Codex hooks report theirs.
fn subagents(home: &Path, agent: &Agent, input: Input, started: Instant) -> Value {
    let Some(pane) = pane() else {
        return json!({});
    };
    let counts = counters::PaneCounters {
        working: input.working,
        done: input.done,
    };
    let who = reporter(agent, &input);
    // A refused record write does not keep the session's identity from the
    // core: the counts it carried are left out instead (engineering rules 4
    // and 10).
    let stored = counters::store(home, &pane, counts, who);
    let outcome = report::report(
        &report::HookStatusReport {
            runtime: agent.runtime().to_owned(),
            version: hide_agent_hooks::HOOK_VERSION,
            session: who.session.map(str::to_owned),
            turn: None,
            counts: stored.is_ok().then_some(report::HookCounts {
                working: counts.working,
                done: counts.done,
            }),
            session_start: false,
            waiting: None,
            account: None,
            refused: None,
        },
        started + report::REPORT_TIMEOUT,
    );
    let record_error = stored
        .err()
        .map(|error| format!("the pane's count could not be stored: {error}"))
        .map_or(Ok(()), Err);
    let outcome = report::outcome(record_error, outcome);
    report::record_and_say(home, &pane, agent.runtime(), "subagent count", &outcome);
    json!({ "reported": outcome.is_ok() })
}

/// The turn of the script's own session: working, done, waiting on a
/// permission, or at rest after an interrupt (PRD engine-bundle-1 D-10).
fn turn(home: &Path, agent: &Agent, input: Input, started: Instant) -> Value {
    let (Some(pane), Some(turn)) = (pane(), input.turn) else {
        return json!({});
    };
    if input.guard_only {
        return json!({});
    }
    let who = reporter(agent, &input);
    let counted = agent.adapter.subagent_counts.is_some();
    // The turn comes from the script's own event, so a refused record write
    // keeps its report and leaves out only the counts the record held
    // (engineering rules 4 and 10).
    let kept = counters::apply(
        home,
        &pane,
        counters::Change::None,
        who,
        counters::Said::Turn(turn),
        None,
    );
    let reported = report::report(
        &report::HookStatusReport {
            runtime: agent.runtime().to_owned(),
            version: hide_agent_hooks::HOOK_VERSION,
            session: who.session.map(str::to_owned),
            turn: Some(turn),
            counts: kept.as_ref().ok().and_then(|applied| {
                counted.then_some(report::HookCounts {
                    working: applied.counters.working,
                    done: applied.counters.done,
                })
            }),
            session_start: false,
            waiting: None,
            account: None,
            refused: None,
        },
        started + report::REPORT_TIMEOUT,
    );
    let record_error = kept
        .err()
        .map(|error| format!("the pane's record could not be changed: {error}"))
        .map_or(Ok(()), Err);
    let outcome = report::outcome(record_error, reported);
    report::record_and_say(home, &pane, agent.runtime(), "turn", &outcome);
    json!({ "reported": outcome.is_ok() })
}

/// The session the pane's agent started or was switched to, reported as a
/// command hook's session start is, so the core knows the pane's session
/// before its first turn and letters and the inbox reach it from the start
/// (PRD engine-bundle-4-herdr-removal, mission c3): on the core's own engine
/// nothing else names it. The counts are the
/// script's to send; a session start says nothing of them.
fn session_start(home: &Path, agent: &Agent, input: Input, started: Instant) -> Value {
    let Some(pane) = pane() else {
        return json!({});
    };
    let who = reporter(agent, &input);
    if input.guard_only || who.session.is_none() {
        return json!({});
    }
    // The session's identity is what the core binds a start by, and it comes
    // from the script's own call, so a refused record write does not keep it
    // back (engineering rules 4 and 10).
    let kept = counters::apply(
        home,
        &pane,
        counters::Change::None,
        who,
        counters::Said::Nothing,
        None,
    );
    let reported = report::report(
        &report::HookStatusReport {
            runtime: agent.runtime().to_owned(),
            version: hide_agent_hooks::HOOK_VERSION,
            session: who.session.map(str::to_owned),
            turn: None,
            counts: None,
            session_start: true,
            waiting: None,
            // The plugin's helper runs in the agent's own environment.
            account: hide_agent_hooks::account::reported_variables(agent.runtime(), |name| {
                std::env::var_os(name)
            }),
            refused: None,
        },
        started + report::REPORT_TIMEOUT,
    );
    let record_error = kept
        .err()
        .map(|error| format!("the pane's record could not be changed: {error}"))
        .map_or(Ok(()), Err);
    let outcome = report::outcome(record_error, reported);
    report::record_and_say(home, &pane, agent.runtime(), "session start", &outcome);
    json!({ "reported": outcome.is_ok() })
}

/// The script's session as the pane record and the core key it: the agent's
/// own id, which is what the pane's record names for OpenCode.
fn reporter<'a>(agent: &Agent, input: &'a Input) -> counters::Reporter<'a> {
    counters::Reporter {
        agent: Some(agent.runtime()),
        session: input
            .session_id
            .as_deref()
            .filter(|id| delivery::valid_session(id)),
    }
}

#[cfg(test)]
mod memory_budget_tests {
    use super::bounded_memory_capsule;

    #[test]
    fn a_first_prompt_keeps_its_whole_signed_body_within_the_shared_quota() {
        let index = format!(
            "<hide-memory-index>{}</hide-memory-index>",
            "index ".repeat(600)
        );
        let bodies = format!(
            "<hide-memory-context>{}</hide-memory-context><hide-memory-receipt items=\"m1@2\" auth=\"synthetic\" />",
            "body ".repeat(600)
        );
        let (capsule, index_kept) = bounded_memory_capsule(&index, &bodies);
        let capsule = capsule.unwrap();
        assert!(!index_kept);
        assert_eq!(capsule, bodies);
        assert!(hide_memory::hook::estimated_tokens(&capsule) <= 1200);
        let (capsule, index_kept) = bounded_memory_capsule(
            "<hide-memory-index>one</hide-memory-index>",
            "<hide-memory-context>two</hide-memory-context>",
        );
        assert!(index_kept);
        assert!(
            capsule
                .unwrap()
                .contains("one</hide-memory-index><hide-memory-context>two")
        );
    }
}
