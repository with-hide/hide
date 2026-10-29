//! A hook's Project Memory, asked of the core (PRD core-host-node-move D-12,
//! B14). The core owns the one store and answers for the checkout the hook's
//! pane is attested to, whichever machine the agent runs on; the hook asks
//! through `hide workspace memory`, the same pane credential and node link
//! its letters come by, and fails open: no answer before its deadline is a
//! turn without Memory.

use crate::delivery::{self, Failure};
use crate::runtime::{AgentRuntime, HookEvent, hook_stdout_with_context};
use hide_memory::HOOK_INPUT_LIMIT_BYTES;
use hide_memory::hook::{HookContext, HookOutcome};
use serde::Deserialize;
use std::io::{self, Read};
use std::path::Path;
use std::time::{Duration, Instant};

/// How long a hook waits for Memory, counted from the hook process's start,
/// so reading its input comes out of it (a plugin helper counts from its ask,
/// within its own deadline): inside the 2 s every hook has, and never past
/// the letter pull it runs beside.
pub const MEMORY_BUDGET: Duration = Duration::from_millis(1_250);

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum HookMemoryOutcome {
    Provided { count: usize },
    Empty,
    Disabled,
    Unavailable,
    Deadline,
    InputTooLarge,
    ProjectUnresolved,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct HookMemoryResult {
    pub stdout: Option<String>,
    pub outcome: HookMemoryOutcome,
}

#[derive(Debug, Deserialize)]
struct HookInput {
    prompt: Option<String>,
    session_id: Option<String>,
}

/// Retains only the bounded stdin prefix and stops as soon as overflow is
/// observed. The installed binary additionally puts this read behind the
/// hook's absolute process deadline.
pub fn read_bounded(mut input: impl Read) -> io::Result<(Vec<u8>, bool)> {
    let mut retained = Vec::with_capacity(HOOK_INPUT_LIMIT_BYTES.min(16 * 1024));
    let mut buffer = [0_u8; 8192];
    let mut exceeded = false;
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let remaining = HOOK_INPUT_LIMIT_BYTES.saturating_sub(retained.len());
        let keep = remaining.min(read);
        retained.extend_from_slice(&buffer[..keep]);
        exceeded |= keep < read;
        if exceeded {
            break;
        }
    }
    Ok((retained, exceeded))
}

/// The hook's output for `bytes`, the runtime's payload, with the Memory the
/// core gives it before `deadline`; a failure is recorded under `home`.
pub fn project_memory_output_until(
    runtime: AgentRuntime,
    event: HookEvent,
    bytes: &[u8],
    exceeded: bool,
    home: &Path,
    deadline: Instant,
) -> HookMemoryResult {
    let projected = if exceeded {
        MemoryContext::without(HookMemoryOutcome::InputTooLarge)
    } else {
        match serde_json::from_slice::<HookInput>(bytes) {
            Ok(input) => memory_context_until(
                MemoryRequest {
                    runtime_id: runtime.memory_id(),
                    event,
                    prompt: input.prompt,
                    session_id: input.session_id,
                },
                home,
                deadline,
            ),
            Err(_) => MemoryContext::without(HookMemoryOutcome::Unavailable),
        }
    };
    HookMemoryResult {
        stdout: match &projected.joined() {
            Some(context) => hook_stdout_with_context(runtime, event, Some(context)),
            None => crate::runtime::hook_stdout(runtime, event),
        },
        outcome: projected.outcome,
    }
}

/// What one hook asks Project Memory for, whatever runtime's payload it came in.
pub struct MemoryRequest<'a> {
    /// The provider id Memory keys sessions and receipts by: `claude`,
    /// `codex`, `opencode`, `pi` or `omp`.
    pub runtime_id: &'a str,
    /// `SessionStart` for a session's first context, `UserPromptSubmit` for a
    /// later prompt; any other event asks for nothing.
    pub event: HookEvent,
    pub prompt: Option<String>,
    pub session_id: Option<String>,
}

/// The Memory context for one hook, with its receipt, or none.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MemoryContext {
    pub context: Option<String>,
    pub outcome: HookMemoryOutcome,
    /// The Hide global instruction a session start carries, in its own slot
    /// and under its own cap, whether or not Memory answered (PRD
    /// memory-self-improve D-31).
    pub global: Option<String>,
}

impl MemoryContext {
    fn without(outcome: HookMemoryOutcome) -> Self {
        Self {
            context: None,
            outcome,
            global: None,
        }
    }

    /// The global instruction first, then Memory's context, or none.
    pub fn joined(&self) -> Option<String> {
        match (&self.global, &self.context) {
            (None, None) => None,
            (global, context) => Some(format!(
                "{}{}",
                global.as_deref().unwrap_or_default(),
                context.as_deref().unwrap_or_default()
            )),
        }
    }
}

/// The core's outcome as the hook records it.
impl From<HookOutcome> for HookMemoryOutcome {
    fn from(outcome: HookOutcome) -> Self {
        match outcome {
            HookOutcome::Provided { count } => Self::Provided { count },
            HookOutcome::Empty => Self::Empty,
            HookOutcome::Disabled => Self::Disabled,
            HookOutcome::Unavailable => Self::Unavailable,
            HookOutcome::Deadline => Self::Deadline,
            HookOutcome::ProjectUnresolved => Self::ProjectUnresolved,
        }
    }
}

/// [`MemoryContext`] for `request` from the core, bounded by `deadline` and
/// failing open; a failure to ask is recorded under `home` (`memory_hook`
/// `memory.unavailable` or `memory.deadline`; a panic on the thread that
/// asks is the caller's to record, with [`record_panic`]).
pub fn memory_context_until(
    request: MemoryRequest<'_>,
    home: &Path,
    deadline: Instant,
) -> MemoryContext {
    let MemoryRequest {
        runtime_id,
        event,
        prompt,
        session_id,
    } = request;
    if !matches!(event, HookEvent::SessionStart | HookEvent::UserPromptSubmit) {
        return MemoryContext::without(HookMemoryOutcome::Unavailable);
    }
    let Some(session) = session_id.filter(|session| delivery::valid_session(session)) else {
        return MemoryContext::without(HookMemoryOutcome::Unavailable);
    };
    let arguments = [
        "workspace",
        "memory",
        "--event",
        event.name(),
        "--runtime",
        runtime_id,
        "--session",
        &session,
    ];
    let prompt = match event {
        HookEvent::UserPromptSubmit => prompt.as_deref().map(hide_memory::hook::prompt_prefix),
        _ => None,
    };
    let answer =
        delivery::run_cli_with_input(&arguments, prompt.unwrap_or_default().as_bytes(), deadline)
            .and_then(|value| {
                serde_json::from_value::<HookContext>(value).map_err(|_| "format".into())
            });
    let answer = match answer {
        Ok(answer) => answer,
        Err(failure) => {
            let deadline = failure.code == "deadline";
            record(home, &failure);
            return MemoryContext::without(if deadline {
                HookMemoryOutcome::Deadline
            } else {
                HookMemoryOutcome::Unavailable
            });
        }
    };
    MemoryContext {
        context: answer.context,
        outcome: answer.outcome.into(),
        global: answer.global,
    }
}

fn record(home: &Path, failure: &Failure) {
    if failure.code == "deadline" {
        note(
            home,
            "memory_deadline",
            "memory.deadline",
            failure.code,
            false,
        );
    } else {
        // Every other failure is one cause; the step it stopped at is kept
        // beside it, so the record says where.
        note(
            home,
            "memory_unavailable",
            "memory.unavailable",
            failure.code,
            true,
        );
    }
}

/// Records that the thread asking for Memory panicked, which the turn goes
/// on without: the hook's other work never waits on it.
pub fn record_panic(home: &Path) {
    note(
        home,
        "memory_panicked",
        "memory.panicked",
        "panicked",
        false,
    );
}

fn note(home: &Path, cause: &str, kind: &str, code: &str, step: bool) {
    if delivery::claim_at(home, cause, step.then_some(code)) {
        eprintln!(
            "{}",
            serde_json::json!({"component":"memory_hook","kind":kind,"code":code,"diagnostic_saved":true})
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B1, D-31: the global instruction rides beside Memory's context, first,
    /// and alone when Memory gave nothing.
    #[test]
    fn the_global_instruction_comes_before_memory_and_without_it() {
        let both = MemoryContext {
            context: Some("<memory/>\n".into()),
            outcome: HookMemoryOutcome::Empty,
            global: Some("<global/>\n".into()),
        };
        assert_eq!(both.joined().as_deref(), Some("<global/>\n<memory/>\n"));
        let alone = MemoryContext {
            context: None,
            outcome: HookMemoryOutcome::Unavailable,
            global: Some("<global/>\n".into()),
        };
        assert_eq!(alone.joined().as_deref(), Some("<global/>\n"));
        assert_eq!(
            MemoryContext::without(HookMemoryOutcome::Disabled).joined(),
            None
        );
    }

    #[test]
    fn oversized_input_is_drained_but_never_parsed() {
        let bytes = vec![b'x'; HOOK_INPUT_LIMIT_BYTES + 7];
        let (retained, exceeded) = read_bounded(bytes.as_slice()).unwrap();
        assert_eq!(retained.len(), HOOK_INPUT_LIMIT_BYTES);
        assert!(exceeded);
    }

    /// A Memory request that failed is one cause, with the step it stopped
    /// at kept beside it; a panic and a deadline name themselves.
    #[test]
    fn a_failed_memory_request_records_the_step_it_stopped_at() {
        let home = tempfile::tempdir().unwrap();
        record(home.path(), &Failure::from("connect"));
        record(home.path(), &Failure::from("deadline"));
        record_panic(home.path());
        let path = home
            .path()
            .join(".hide/agent-hooks/delivery-diagnostics.json");
        let recorded: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        for cause in ["memory_unavailable", "memory_deadline", "memory_panicked"] {
            assert!(recorded["last"][cause].is_u64(), "{cause}: {recorded}");
        }
        assert_eq!(
            recorded["step"],
            serde_json::json!({"memory_unavailable": "connect"})
        );
    }

    /// A panic on the Memory thread leaves its record, once per ten minutes
    /// like every other hook failure.
    #[test]
    fn a_memory_thread_panic_is_recorded_once() {
        let home = tempfile::tempdir().unwrap();
        record_panic(home.path());
        let path = home
            .path()
            .join(".hide/agent-hooks/delivery-diagnostics.json");
        let recorded: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(recorded["last"]["memory_panicked"].is_u64(), "{recorded}");
        assert!(!delivery::claim(home.path(), "memory_panicked"));
    }

    #[test]
    fn the_core_s_answer_reads_as_the_hook_s_outcome() {
        let answer = |value: serde_json::Value| {
            let answer = serde_json::from_value::<HookContext>(value).unwrap();
            (answer.context, HookMemoryOutcome::from(answer.outcome))
        };
        let (context, outcome) = answer(serde_json::json!({
            "context": "Project Memory:\n- Keep it.\n", "outcome": "provided", "count": 1,
        }));
        assert_eq!(outcome, HookMemoryOutcome::Provided { count: 1 });
        assert_eq!(context.as_deref(), Some("Project Memory:\n- Keep it.\n"));
        let (context, outcome) =
            answer(serde_json::json!({"context": null, "outcome": "project_unresolved"}));
        assert_eq!(outcome, HookMemoryOutcome::ProjectUnresolved);
        assert!(context.is_none());
    }

    #[test]
    fn current_runtime_prompt_fixtures_parse_and_use_the_verified_context_envelope() {
        for (runtime, bytes) in [
            (
                AgentRuntime::ClaudeCode,
                include_bytes!("../tests/fixtures/claude-user-prompt-submit.json").as_slice(),
            ),
            (
                AgentRuntime::Codex,
                include_bytes!("../tests/fixtures/codex-user-prompt-submit.json").as_slice(),
            ),
        ] {
            let input: HookInput = serde_json::from_slice(bytes).unwrap();
            assert_eq!(
                input.prompt.as_deref(),
                Some("Inspect the project memory boundary")
            );
            let output = hook_stdout_with_context(
                runtime,
                HookEvent::UserPromptSubmit,
                Some("Project Memory:\n- Keep the boundary local.\n"),
            )
            .unwrap();
            let value: serde_json::Value = serde_json::from_str(&output).unwrap();
            assert_eq!(
                value["hookSpecificOutput"]["hookEventName"],
                "UserPromptSubmit"
            );
            assert_eq!(
                value["hookSpecificOutput"]["additionalContext"],
                "Project Memory:\n- Keep the boundary local.\n"
            );
        }
    }

    #[test]
    fn an_expired_hook_keeps_only_its_base_context() {
        let temp = tempfile::tempdir().unwrap();
        let payload = serde_json::to_vec(&serde_json::json!({
            "cwd": temp.path(),
            "session_id": "expired-session"
        }))
        .unwrap();

        let result = project_memory_output_until(
            AgentRuntime::Codex,
            HookEvent::SessionStart,
            &payload,
            false,
            temp.path(),
            Instant::now(),
        );

        assert_eq!(result.outcome, HookMemoryOutcome::Deadline);
        let output: serde_json::Value =
            serde_json::from_str(result.stdout.as_ref().unwrap()).unwrap();
        assert_eq!(
            output["hookSpecificOutput"]["additionalContext"],
            crate::runtime::PURPOSE_CONTEXT
        );
    }
}
