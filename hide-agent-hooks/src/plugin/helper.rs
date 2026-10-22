//! `hide-agent-hooks <agent> <operation>`: what Hide's script file in an
//! agent's own folder asks (`hide_agent_hooks::plugin`): OpenCode's plugin
//! (`opencode`), and Pi's and omp's extension (`pi`, `omp`). Each operation
//! reads one JSON object on stdin, prints one on stdout and exits 0; a failure
//! answers `{}`, the script then changes nothing, and the cause goes to the
//! same private diagnostic the other hooks use.
//!
//! The operations reuse the Claude Code and Codex hook's parts: the letter
//! intake (`delivery`), the spawn guard
//! (`spawn_guard`), the pane counts (`counters`, `report`) and Project Memory
//! (`memory`). What differs is only the transport: these agents have no
//! command hook, so the script hands over the facts a hook payload would
//! carry. What each agent gets follows its adapter declaration.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hide_agent_adapter::{AgentAdapter, PluginDialect};
use hide_agent_hooks::delivery::{self, Intake, Prompt};
use hide_agent_hooks::memory::{self, HookMemoryOutcome, MemoryRequest, memory_context_until};
use hide_agent_hooks::runtime::HookEvent;
use hide_agent_hooks::spawn_guard::{self as guard, Registration};
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
    /// letters, as Herdr does, and sends this beside it.
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
    /// Helper calls the script skipped at its cap or gave up on since its last
    /// call: recorded here, since the script has no log of its own.
    lost: u32,
    /// The version of the script that asks; Pi's and omp's extension sends it,
    /// and one this build does not write is answered with nothing (B13).
    version: Option<u32>,
    /// The script only guards this session: it runs outside a Herdr pane, or
    /// is an agent started in another agent's shell, which a pane variable
    /// it inherited does not make the pane's own (PRD agent-guardrails B22).
    guard_only: bool,
    /// Transient Post input, consumed by the same owned helper. Never stored
    /// in the metadata admission request.
    observation: Option<Value>,
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
        Some("observation")
            if agent.adapter.observation
                == hide_agent_adapter::ObservationSupport::Plugin(agent.dialect) =>
        {
            observation(&home, &agent, &input, started)
        }
        Some("letters") if agent.adapter.tool_letters.is_some() => {
            letters(&home, input, started + LETTERS_BUDGET)
        }
        Some("subagents") if agent.adapter.subagent_counts.is_some() => {
            subagents(&home, &agent, input)
        }
        _ => json!({}),
    };
    answer(&output);
}

fn observation(home: &Path, agent: &Agent, input: &Input, started: Instant) -> Value {
    use hide_agent_hooks::observation::{self, ObservationCorrelation, ToolResult};
    let Some(post) = input.observation.as_ref() else {
        return json!({});
    };
    if input.guard_only {
        return json!({});
    }
    let Some(command) = post["command"].as_str() else {
        return json!({});
    };
    let Some(session) = input.session_id.as_deref() else {
        return json!({});
    };
    let Some(call_id) = post["call_id"].as_str() else {
        return json!({});
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    let parsed = observation::observe(
        ToolResult {
            runtime_id: agent.runtime(),
            session_id: session,
            call_id,
            command,
            correlation_kind: ObservationCorrelation::ToolCall,
            result: post.get("result"),
            failed: post["failed"] == true,
            interrupted: post["interrupted"] == true,
            truncated: post["truncated"] == true,
            asynchronous: post["asynchronous"] == true,
        },
        now,
        &|name| std::env::var(name).ok(),
    );
    let accepted = parsed
        .as_ref()
        .is_some_and(|parsed| observation::send(home, parsed, started + observation::SEND_BUDGET));
    json!({ "observed": accepted })
}

fn answer(value: &Value) {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{value}").and_then(|_| stdout.flush());
}

fn pane() -> Option<String> {
    std::env::var("HERDR_PANE_ID")
        .ok()
        .filter(|value| !value.is_empty())
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
    if input.memory_first.unwrap_or(input.first) {
        let index = memory_context_until(request(HookEvent::SessionStart), home, deadline);
        start = matches!(
            index.outcome,
            HookMemoryOutcome::Provided { .. } | HookMemoryOutcome::Empty
        );
        context.push_str(index.context.as_deref().unwrap_or_default());
    }
    let prompt = memory_context_until(request(HookEvent::UserPromptSubmit), home, deadline);
    let body = prompt.context.as_deref().unwrap_or_default();
    let (context, index_kept) = bounded_memory_capsule(&context, body);
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
    let intake = Intake {
        context: String::new(),
        ids: input.letters,
        plan_context: None,
    };
    match delivery::confirm(&intake, deadline) {
        Ok(()) => json!({ "confirmed": intake.ids }),
        Err(failure) => {
            delivery::diagnose_failure(home, &failure);
            json!({})
        }
    }
}

/// The project's guardrails for a shell or file tool, the spawn guard for
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
            action: guardrails::payload::action(tool, &args)?,
            cwd: input.cwd.clone(),
        })
    };
    // The guardrails hold in any session of a guarded project, a Herdr pane
    // or not (PRD agent-guardrails B22); the guards after them only in one.
    if let Some(reason) = guardrails::refusal_for(home, runtime, pane.as_deref(), &call) {
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
        Some("bash" | "shell") => {
            let Some(command) = input.command.as_deref() else {
                return json!({});
            };
            let Some(launch) = guard::find_launch(command, &|name| std::env::var(name).ok()) else {
                return json!({});
            };
            match guard::registration(&program, deadline) {
                Registration::Registered => {}
                Registration::NotRegistered => return json!({}),
                Registration::Unreachable(cause) => {
                    guard::unreachable(home, runtime, cause);
                    return json!({});
                }
            }
            let cwd = input
                .cwd
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_default();
            let (repo, branch) = guard::checkout_facts(&cwd);
            let delegation =
                guard::spawn_command(&launch, repo.as_deref(), branch.as_deref(), true);
            let handoff = guard::spawn_command(&launch, repo.as_deref(), branch.as_deref(), false);
            // Logged before it is answered; the refusal stands without the log.
            guard::record_refusal(home, runtime, &pane, &launch);
            json!({ "deny": guard::refusal_reason(&delegation, &handoff) })
        }
        _ => json!({}),
    }
}

/// The pane's subagent counts as the script keeps them, stored and reported to
/// Herdr the way the Claude Code and Codex hooks report theirs.
fn subagents(home: &Path, agent: &Agent, input: Input) -> Value {
    let Some(pane) = pane() else {
        return json!({});
    };
    let counts = counters::PaneCounters {
        working: input.working,
        done: input.done,
    };
    let who = counters::Reporter {
        agent: Some(agent.runtime()),
        session: input
            .session_id
            .as_deref()
            .filter(|id| delivery::valid_session(id)),
    };
    if counters::store(home, &pane, counts, who).is_err() {
        return json!({});
    }
    let socket_path = report::socket_path(home);
    let outcome = match &socket_path {
        Ok(path) => report::report(path, &pane, counts),
        Err(error) => Err(error.clone()),
    };
    let socket_path = socket_path.unwrap_or_default();
    let _ = report::record_outcome(home, &pane, "subagent count", &socket_path, &outcome);
    json!({ "reported": outcome.is_ok() })
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
