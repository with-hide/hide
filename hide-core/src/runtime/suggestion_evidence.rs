//! The words behind a 제안 row, read only when the operator opens its
//! evidence (PRD memory-self-improve D-34, D-43, B7): nothing of them is
//! stored, the read runs off the lock through the node that holds the
//! sessions, and each excerpt is masked and cut before it reaches the
//! snapshot. A row none of whose words can be read again cannot be applied.
//! One read runs at a time; a row opened while it runs is read when it ends.

use super::*;
use crate::model::ProjectSuggestionsSnapshot;
use crate::suggestions::policy::{EXCERPT_CHARS, EXCERPTS};
use serde::Deserialize;
use serde_json::json;

/// `suggestion_evidence`: the row whose evidence the operator opened, or none
/// when it was closed.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SuggestionEvidencePayload {
    workspace_id: String,
    #[serde(default)]
    id: Option<String>,
}

/// The excerpts of the one row whose evidence is open.
#[derive(Default)]
pub(super) struct EvidenceWork {
    /// The Project and row open: row ids repeat across Projects.
    open: Option<(String, String)>,
    generation: u64,
    /// Each evidence place read, and its masked excerpt or none.
    read: Option<Vec<Excerpt>>,
    /// A read thread is running; it reads the open row again when it ends
    /// on an older one.
    running: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Excerpt {
    agent: String,
    session_id: String,
    text: Option<String>,
}

impl EvidenceWork {
    /// Whether the open row's words were read and none could be (B7).
    pub(super) fn unconfirmed(&self, workspace_id: &str, row: &str) -> bool {
        self.open
            .as_ref()
            .is_some_and(|(workspace, open)| workspace == workspace_id && open == row)
            && self
                .read
                .as_ref()
                .is_some_and(|read| !read.is_empty() && read.iter().all(|e| e.text.is_none()))
    }

    /// Forgets the open row, so a read still running is dropped when it ends.
    pub(super) fn close(&mut self) {
        self.generation += 1;
        self.open = None;
        self.read = None;
    }
}

impl Runtime {
    pub(super) fn open_suggestion_evidence(&mut self, payload: SuggestionEvidencePayload) -> bool {
        let work = &mut self.suggestions_work.evidence;
        work.close();
        let Some(id) = payload.id else {
            return self.compose_suggestions();
        };
        if self
            .suggestions_work
            .opened
            .as_ref()
            .is_none_or(|opened| opened.workspace_id != payload.workspace_id)
        {
            return self.compose_suggestions();
        }
        self.suggestions_work.evidence.open = Some((payload.workspace_id, id));
        self.read_suggestion_evidence();
        self.compose_suggestions()
    }

    /// Starts reading the open row's words, unless a read is running, which
    /// starts this one when it ends.
    fn read_suggestion_evidence(&mut self) {
        let work = &self.suggestions_work.evidence;
        if work.running || work.read.is_some() {
            return;
        }
        let Some((workspace_id, id)) = work.open.clone() else {
            return;
        };
        let Some(opened) = self
            .suggestions_work
            .opened
            .clone()
            .filter(|opened| opened.workspace_id == workspace_id)
        else {
            return;
        };
        let places = self
            .snapshot
            .suggestions
            .as_ref()
            .and_then(|section| section.project.as_ref())
            .and_then(|project| project.rows.iter().find(|row| row.id == id))
            .map(|row| {
                row.evidence
                    .iter()
                    .filter_map(|evidence| {
                        Some((
                            evidence.agent.clone()?,
                            evidence.session_id.clone()?,
                            evidence.offset?,
                        ))
                    })
                    .take(EXCERPTS)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if places.is_empty() {
            return;
        }
        let (Some(context), Ok((_, link))) = (
            self.worker_context.clone(),
            self.memory_node(opened.node.as_str()),
        ) else {
            self.suggestions_work.evidence.read = Some(unread(&places));
            return;
        };
        let database = self.memory_database_path();
        let generation = self.suggestions_work.evidence.generation;
        self.suggestions_work.evidence.running = true;
        let spawned = thread::Builder::new()
            .name("hide-suggestion-evidence".to_owned())
            .spawn(move || {
                let fallback = unread(&places);
                // A read that panics still ends this one: its places show as
                // unreadable and the next row is read.
                let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    read_excerpts(&database, link.as_ref(), &opened, places)
                }))
                .unwrap_or_else(|_| {
                    crate::diagnostic!(json!({
                        "component": "suggestions", "kind": "evidence.read_panicked",
                    }));
                    fallback
                });
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                let changed = match runtime.lock() {
                    Ok(mut guard) => {
                        let work = &mut guard.suggestions_work.evidence;
                        work.running = false;
                        if work.generation == generation {
                            work.read = Some(read);
                        }
                        // A row opened while this ran is read now.
                        guard.read_suggestion_evidence();
                        guard.compose_suggestions()
                    }
                    Err(_) => return,
                };
                drop(runtime);
                if changed {
                    context.notifier.notify();
                }
            });
        if let Err(error) = spawned {
            self.suggestions_work.evidence.running = false;
            crate::diagnostic!(json!({
                "component": "suggestions", "kind": "evidence.read_not_started",
                "message": error.to_string(),
            }));
        }
    }
}

/// Each place as one whose words could not be read.
fn unread(places: &[(String, String, u64)]) -> Vec<Excerpt> {
    places
        .iter()
        .map(|(agent, session_id, _)| Excerpt {
            agent: agent.clone(),
            session_id: session_id.clone(),
            text: None,
        })
        .collect()
}

/// Reads each place's words again, masked and cut; a place that cannot be
/// read is none and is logged by its session, never its words.
fn read_excerpts(
    database: &std::path::Path,
    node: &dyn crate::node_access::NodeLink,
    opened: &super::suggestions::OpenedProject,
    places: Vec<(String, String, u64)>,
) -> Vec<Excerpt> {
    let store = hide_memory::MemoryStore::open_read_only(database).ok();
    let project = super::memory::project_identity(node, &opened.node, &opened.root).ok();
    places
        .into_iter()
        .map(|(agent, session_id, offset)| {
            let text = store
                .as_ref()
                .zip(project.as_ref())
                .and_then(|(store, project)| {
                    super::memory_verify::source_event(
                        store,
                        node,
                        &project.id,
                        &agent,
                        &session_id,
                        offset,
                    )
                })
                // Only the operator's own words stand as evidence: a session
                // rewritten since may hold a letter or an answer there now.
                .filter(|event| event.from_operator())
                .map(|event| {
                    let masked = hide_memory::redact(&event.text).text;
                    masked
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .chars()
                        .take(EXCERPT_CHARS)
                        .collect::<String>()
                })
                .filter(|text| !text.is_empty());
            if text.is_none() {
                crate::diagnostic!(json!({
                    "component": "suggestions", "kind": "evidence.unreadable",
                    "agent": agent, "session_id": session_id,
                }));
            }
            Excerpt {
                agent,
                session_id,
                text,
            }
        })
        .collect()
}

/// Lays the open row's excerpts on it: each read place's words or its
/// `근거 확인 불가`, and the row's block when none could be read (B7).
pub(super) fn lay_excerpts(project: &mut ProjectSuggestionsSnapshot, work: &EvidenceWork) {
    let (Some((workspace_id, id)), Some(read)) = (work.open.as_ref(), work.read.as_ref()) else {
        return;
    };
    if *workspace_id != project.workspace_id {
        return;
    }
    let unconfirmed = work.unconfirmed(workspace_id, id);
    let Some(row) = project.rows.iter_mut().find(|row| row.id == *id) else {
        return;
    };
    for evidence in &mut row.evidence {
        let Some(excerpt) = read.iter().find(|excerpt| {
            Some(&excerpt.agent) == evidence.agent.as_ref()
                && Some(&excerpt.session_id) == evidence.session_id.as_ref()
        }) else {
            continue;
        };
        evidence.excerpt.clone_from(&excerpt.text);
        evidence.unreadable = excerpt.text.is_none();
    }
    row.evidence_unconfirmed = unconfirmed;
}
