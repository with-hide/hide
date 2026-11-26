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

pub use hide_memory::hook::HookContext;
use hide_memory::hook::{HookEvent, HookOutcome, HookRequest};
use hide_node_link::protocol::Call;
use serde_json::Value;
#[cfg(test)]
use serde_json::json;

pub use hide_memory::hook::{PROMPT_LIMIT_BYTES, prefix};

/// The answer for a hook whose Memory cannot be read, carrying only the Hide
/// global instruction a session start still gets (PRD memory-self-improve
/// D-31), the open questions a prompt still gets and the Project's other
/// sessions a session start still gets.
pub fn without_memory(
    global: Option<String>,
    asks: Option<String>,
    sessions: Option<String>,
) -> HookContext {
    HookContext {
        context: None,
        outcome: HookOutcome::Unavailable,
        global,
        asks,
        sessions,
    }
}

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
    /// Whether this ask starts a session, which carries the Hide global
    /// instruction beside Memory (PRD memory-self-improve D-29).
    pub fn starts_session(&self) -> bool {
        self.event == HookEvent::SessionStart
    }

    /// The session the hook names, when it starts one: a start is when the
    /// agent is given the Project's other sessions (`links::session_brief`).
    pub fn start_session(&self) -> Option<&str> {
        self.starts_session().then_some(self.session.as_str())
    }

    /// The session the hook names, when it asks at a prompt: a prompt is
    /// when the agent is given its open questions (`links::agent_asks`).
    pub fn prompt_session(&self) -> Option<&str> {
        (self.event == HookEvent::UserPromptSubmit).then_some(self.session.as_str())
    }

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

/// The Memory context for `ask`, or none with why, before `deadline` and
/// within the hook's own 100 ms.
pub fn answer(scope: &Scope, ask: &Ask, deadline: Instant) -> HookContext {
    answer_until(
        scope,
        ask,
        hide_memory::hook::within_hook_budget(move || Instant::now() >= deadline),
    )
}

/// [`answer`] bounded by `expired` alone, which a test that is not about the
/// budget holds still.
fn answer_until<E>(scope: &Scope, ask: &Ask, expired: E) -> HookContext
where
    E: Fn() -> bool + Clone + Send + 'static,
{
    let without = |outcome| HookContext {
        context: None,
        outcome,
        global: None,
        asks: None,
        sessions: None,
    };
    // A doorbell's turn is the bell's line, not the operator's words: no card
    // answers it, and its open questions ride beside this answer.
    if ask.event == HookEvent::UserPromptSubmit
        && ask
            .prompt
            .as_deref()
            .is_some_and(hide_session::is_bell_line)
    {
        return without(HookOutcome::Empty);
    }
    // A store that does not exist yet costs no node call.
    if !scope.store.is_file() {
        return without(HookOutcome::Unavailable);
    }
    if expired() {
        return without(HookOutcome::Deadline);
    }
    let prompt = hide_memory::hook::prompt_prefix(ask.prompt.as_deref().unwrap_or_default());
    hide_memory::hook::context_until(
        &scope.store,
        &scope.project,
        HookRequest {
            runtime_id: &ask.runtime,
            event: ask.event,
            session_id: &ask.session,
            prompt: Some(prompt),
            task_context: scope.task_context.as_deref(),
        },
        expired,
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

    /// The hook's 100 ms is `hide_memory::hook`'s tests' to hold; these read
    /// which cards an ask gets, with a clock that never ends.
    fn never() -> bool {
        false
    }

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
                &[
                    hide_memory::Candidate {
                        text: "Use nextest for test execution".into(),
                        title: "Repository test runner".into(),
                        aliases: vec!["테스트 실행기".into()],
                        kind: hide_memory::CandidateKind::Rule,
                        confidence: 0.9,
                        salience: 0.9,
                        source_offsets: vec![1],
                        human_offsets: vec![1],
                        source_times: Default::default(),
                        source_words: Default::default(),
                        cited_paths: vec![],
                        relation: hide_memory::CandidateRelation::New,
                    },
                    hide_memory::Candidate {
                        text: "Granite slabs are catalogued by quarry".into(),
                        title: "Granite catalogue".into(),
                        aliases: vec!["화강암".into()],
                        kind: hide_memory::CandidateKind::Rule,
                        confidence: 0.9,
                        salience: 0.9,
                        source_offsets: vec![2],
                        human_offsets: vec![2],
                        source_times: Default::default(),
                        source_words: Default::default(),
                        cited_paths: vec![],
                        relation: hide_memory::CandidateRelation::New,
                    },
                ],
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
            answer_until(&scope, &ask, never).outcome,
            HookOutcome::Empty
        );
        scope.task_context = Some("테스트 실행기 점검".into());
        let unrelated = Ask::from_wire(&json!({"event":"UserPromptSubmit","runtime":"claude","session":"other-task","prompt":"Investigate the basalt specimen archive and document the geological history of the volcanic rocks collected last year."})).unwrap();
        assert_eq!(
            answer_until(&scope, &unrelated, never).outcome,
            HookOutcome::Empty
        );

        let own_topic = Ask::from_wire(&json!({"event":"UserPromptSubmit","runtime":"claude","session":"short-own-topic","prompt":"granite 목록 정리해줘"})).unwrap();
        let result = answer_until(&scope, &own_topic, never);
        assert_eq!(
            result.outcome,
            HookOutcome::Provided { count: 1 },
            "a short prompt that finds its own card spends nothing on the task's"
        );
        assert!(result.context.unwrap().contains("Granite slabs"));
        hide_memory::MemoryStore::open(&scope.store)
            .unwrap()
            .record_receipt(
                &scope.project.id,
                "claude",
                "short-own-topic",
                "event:1",
                "m2@1",
                5,
            )
            .unwrap();
        assert_eq!(
            answer_until(&scope, &own_topic, never).outcome,
            HookOutcome::Empty,
            "a prompt whose own card was already given gets nothing, never the task's"
        );

        let result = answer_until(&scope, &ask, never);
        assert_eq!(result.outcome, HookOutcome::Provided { count: 1 });
        assert!(
            result
                .context
                .unwrap()
                .contains("Use nextest for test execution")
        );
        scope.task_context = Some(format!(
            "{}테스트 실행기",
            "basalt ".repeat(hide_memory::hook::PROMPT_LIMIT_BYTES / 7 + 1)
        ));
        assert_eq!(
            answer_until(&scope, &ask, never).outcome,
            HookOutcome::Empty,
            "ranking context beyond the combined 8 KiB limit is never searched"
        );
    }

    /// A store holding one card that matches every bell below and the
    /// ordinary prompt beside them.
    fn store_with_a_card_for_every_word(temp: &std::path::Path) -> Scope {
        let project = hide_project::resolve(temp, "local").unwrap();
        let path = hide_memory::database_path(temp);
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
                    text: "Granite reviews run nextest before the pull request opens".into(),
                    title: "Granite review runner".into(),
                    aliases: vec!["화강암".into()],
                    kind: hide_memory::CandidateKind::Rule,
                    confidence: 0.9,
                    salience: 0.9,
                    source_offsets: vec![1],
                    human_offsets: vec![1],
                    source_times: Default::default(),
                    source_words: Default::default(),
                    cited_paths: vec![],
                    relation: hide_memory::CandidateRelation::New,
                }],
            )
            .unwrap();
        Scope {
            store: path,
            project,
            task_context: Some("granite review".into()),
        }
    }

    fn prompt(runtime: &str, session: &str, prompt: &str) -> Ask {
        Ask::from_wire(&json!({
            "event": "UserPromptSubmit", "runtime": runtime, "session": session, "prompt": prompt,
        }))
        .unwrap()
    }

    #[test]
    fn a_doorbells_turn_gets_no_card_and_reads_no_store() {
        use crate::delivery::bell::{Ring, line};
        use crate::model::InterfaceLanguage;
        let temp = tempfile::tempdir().unwrap();
        let scope = store_with_a_card_for_every_word(temp.path());
        let ring = |inbox| {
            line(&Ring {
                sender: "granite-review",
                kind: "report",
                body: "granite review nextest passed",
                others: 1,
                inbox,
                language: InterfaceLanguage::English,
            })
        };
        let claude = ring(false);
        let codex = ring(true);
        // Codex CLI on Windows records the line without the owl.
        let codex_windows: String = codex.chars().filter(|c| u32::from(*c) <= 0xFFFF).collect();
        assert!(!codex_windows.contains(hide_session::bell_line::BELL_MARK));
        let ordinary = prompt("claude", "operator", "granite review nextest passed");
        assert_eq!(
            answer_until(&scope, &ordinary, never).outcome,
            HookOutcome::Provided { count: 1 },
            "the bells' own words find the card when the operator types them"
        );

        // A store that is not there would answer `Unavailable`, so `Empty`
        // proves the bell was answered before the store was looked at.
        let missing = Scope {
            store: temp.path().join("missing.sqlite3"),
            project: scope.project.clone(),
            task_context: scope.task_context.clone(),
        };
        for (runtime, bell) in [
            ("claude", &claude),
            ("codex", &codex),
            ("codex", &codex_windows),
        ] {
            let ask = prompt(runtime, &format!("{runtime}-bell"), bell);
            let result = answer_until(&scope, &ask, never);
            assert_eq!(result.outcome, HookOutcome::Empty, "{bell}");
            assert!(result.context.is_none(), "{bell}");
            assert_eq!(
                answer_until(&missing, &ask, never).outcome,
                HookOutcome::Empty,
                "{bell}"
            );
        }
    }

    #[test]
    fn a_session_start_and_an_ordinary_prompt_still_read_the_store() {
        let temp = tempfile::tempdir().unwrap();
        let scope = store_with_a_card_for_every_word(temp.path());
        let missing = Scope {
            store: temp.path().join("missing.sqlite3"),
            project: scope.project.clone(),
            task_context: None,
        };
        let start = Ask::from_wire(&json!({
            "event": "SessionStart", "runtime": "claude", "session": "starting",
        }))
        .unwrap();
        let ordinary = prompt("claude", "operator", "화강암 리뷰 규칙 알려줘");
        for ask in [&start, &ordinary] {
            assert_eq!(
                answer_until(&missing, ask, never).outcome,
                HookOutcome::Unavailable
            );
        }
        assert_eq!(
            answer_until(&scope, &ordinary, never).outcome,
            HookOutcome::Provided { count: 1 }
        );
        assert_ne!(
            answer_until(&scope, &start, never).outcome,
            HookOutcome::Unavailable
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
