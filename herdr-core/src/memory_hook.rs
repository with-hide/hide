//! `hide workspace memory`: an agent hook's Project Memory, answered by the core
//! from the one store it owns, whichever machine's agent asks (PRD
//! core-host-node-move D-12, B14).
//!
//! The Project is the one holding the checkout the caller's pane is attested
//! to, from the catalog facts already prepared by that checkout's node;
//! nothing the hook sends names it. The Project, bounded confirmed task and
//! store path are taken under the lock as owned values ([`Scope`]); the
//! read-only lookup and receipt tag run off it, inside the caller's deadline.
//! Every linked device uses this store; no Memory database is copied to it.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use hide_memory::hook::{HookContext, HookEvent, HookOutcome, HookRequest};
use hide_node_link::protocol::Call;
use serde_json::Value;
#[cfg(test)]
use serde_json::json;

pub use hide_memory::hook::{PROMPT_LIMIT_BYTES, prefix};

use crate::node::NodeId;
use crate::node_access::{NodeLink, call_as};

/// How long the core spends on one hook's Memory once the request is
/// admitted, inside the 1,250 ms the hook waits for it.
pub const WITHIN: Duration = Duration::from_millis(1_000);

/// What the core answers a hook's Memory read from.
pub struct Scope {
    pub(crate) store: PathBuf,
    pub(crate) project: hide_project::ProjectIdentity,
    pub(crate) task_context: Option<String>,
}

/// What one hook asks, as the CLI sent it.
pub struct Ask {
    runtime: String,
    event: HookEvent,
    session: String,
    prompt: Option<String>,
}

impl Ask {
    /// The ask a `memory_hook` request carries: an event Memory answers, the
    /// runtime as Memory names its sessions, the agent's own session id, and
    /// the prompt when the hook has one.
    pub fn from_wire(value: &Value) -> Option<Self> {
        let event = match value["event"].as_str()? {
            "SessionStart" => HookEvent::SessionStart,
            "UserPromptSubmit" => HookEvent::UserPromptSubmit,
            _ => return None,
        };
        let runtime = value["runtime"]
            .as_str()
            .filter(|runtime| memory_runtimes().any(|known| known == *runtime))?;
        let session = value["session"]
            .as_str()
            .filter(|session| crate::delivery::valid_session(session))?;
        let prompt = match value.get("prompt") {
            None | Some(Value::Null) => None,
            Some(Value::String(text)) => Some(text.clone()),
            Some(_) => return None,
        };
        Some(Self {
            runtime: runtime.to_owned(),
            event,
            session: session.to_owned(),
            prompt,
        })
    }
}

/// The runtimes whose adapter declares Memory, as Memory names their
/// sessions (`claude` for Claude Code).
fn memory_runtimes() -> impl Iterator<Item = String> {
    hide_agent_adapter::ADAPTERS
        .iter()
        .filter(|row| row.memory.is_some())
        .filter_map(|row| row.session)
        .filter_map(|format| {
            serde_json::to_value(hide_session::Agent::from_format(format))
                .ok()
                .and_then(|name| name.as_str().map(str::to_owned))
        })
}

/// The Memory context for `ask`, or none with why, before `deadline`.
pub fn answer(scope: &Scope, ask: &Ask, deadline: Instant) -> HookContext {
    let without = |outcome| HookContext {
        context: None,
        outcome,
    };
    // A store that does not exist yet costs no node call.
    if !scope.store.is_file() {
        return without(HookOutcome::Unavailable);
    }
    if Instant::now() >= deadline {
        return without(HookOutcome::Deadline);
    }
    let prompt = ask.prompt.as_deref().unwrap_or_default();
    let ranked_prompt = scope
        .task_context
        .as_deref()
        .map(|context| format!("{}\n{}", prompt, context));
    hide_memory::hook::context_for(
        &scope.store,
        &scope.project,
        HookRequest {
            runtime_id: &ask.runtime,
            event: ask.event,
            session_id: &ask.session,
            prompt: ranked_prompt.as_deref().or(ask.prompt.as_deref()),
        },
        move || Instant::now() >= deadline,
    )
}

/// The Project that holds `checkout_path`, as the node that holds it reads
/// it; the id names the node.
pub(crate) fn project_identity(
    link: &dyn NodeLink,
    node: &NodeId,
    checkout_path: &str,
    within: Duration,
) -> Result<hide_project::ProjectIdentity, String> {
    let facts: hide_project::ProjectFacts = call_as(
        link,
        Call::Project {
            path: checkout_path.to_owned(),
        },
        within,
    )
    .map_err(|error| error.to_string())?;
    Ok(hide_project::ProjectIdentity {
        id: hide_project::project_id(node.as_str(), &facts.root),
        root: facts.root,
        checkout_root: facts.checkout_root,
        device_id: node.as_str().to_owned(),
        kind: facts.kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn short_continuations_use_only_the_prepared_confirmed_task() {
        let temp = tempfile::tempdir().unwrap();
        let project = hide_project::resolve(temp.path(), "local").unwrap();
        let path = hide_memory::database_path(temp.path());
        let mut store = hide_memory::MemoryStore::open(&path).unwrap();
        store
            .ensure_project(&project.id, &project.root, "local")
            .unwrap();
        store.set_enabled(&project.id, true, true).unwrap();
        store
            .apply_candidates(
                &hide_memory::AnalysisBatch {
                    id: "seed".into(),
                    project_id: project.id.clone(),
                    provider: "claude".into(),
                    analysis_provider: "claude".into(),
                    session_id: "source".into(),
                    content_hash: "seed".into(),
                    created_at_unix_ms: 1,
                },
                &[hide_memory::Candidate {
                    text: "Use nextest for test execution".into(),
                    title: "Repository test runner".into(),
                    aliases: vec!["테스트 실행기".into()],
                    kind: hide_memory::CandidateKind::Rule,
                    confidence: 0.9,
                    salience: 0.9,
                    source_offsets: vec![1],
                    human_offsets: vec![1],
                    source_times: Default::default(),
                    cited_paths: vec![],
                    relation: hide_memory::CandidateRelation::New,
                }],
            )
            .unwrap();
        drop(store);
        let mut scope = Scope {
            store: path,
            project,
            task_context: None,
        };
        let ask = Ask::from_wire(&json!({"event":"UserPromptSubmit","runtime":"claude","session":"task-session","prompt":"계속해 주세요"})).unwrap();
        assert_eq!(
            answer(&scope, &ask, Instant::now() + WITHIN).outcome,
            HookOutcome::Empty
        );
        scope.task_context = Some("테스트 실행기 점검".into());
        let result = answer(&scope, &ask, Instant::now() + WITHIN);
        assert_eq!(result.outcome, HookOutcome::Provided { count: 1 });
        assert!(
            result
                .context
                .unwrap()
                .contains("Use nextest for test execution")
        );
    }

    #[test]
    fn the_memory_wire_admits_exactly_the_runtimes_whose_adapter_declares_memory() {
        let schema: Value =
            serde_json::from_str(include_str!("../../contracts/hided-ws.schema.json")).unwrap();
        let wire: BTreeSet<String> =
            schema["$defs"]["memoryHookAsk"]["properties"]["runtime"]["enum"]
                .as_array()
                .expect("memory runtime enum")
                .iter()
                .map(|value| value.as_str().unwrap().to_owned())
                .collect();
        assert_eq!(wire, memory_runtimes().collect::<BTreeSet<_>>());
        for runtime in &wire {
            let ask = Ask::from_wire(&json!({
                "event": "UserPromptSubmit", "runtime": runtime, "session": "s-1",
            }));
            assert!(ask.is_some(), "{runtime}");
        }
        for refused in [
            json!({"event": "UserPromptSubmit", "runtime": "grok", "session": "s-1"}),
            json!({"event": "Stop", "runtime": "codex", "session": "s-1"}),
            json!({"event": "SessionStart", "runtime": "codex", "session": ""}),
            json!({"event": "SessionStart", "runtime": "codex", "session": "s-1", "prompt": 7}),
        ] {
            assert!(Ask::from_wire(&refused).is_none(), "{refused}");
        }
    }
}
