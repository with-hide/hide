//! `hide memory show|search|add|forget|undo|placed|outdated|stale|release`
//! (PRD project-memory-v2 B7, B8, B14-B16, B27, B30, D-03, D-11, D-25; PRD
//! memory-stale-cards).
//!
//! hided names the caller from its pane capability; the core checks the
//! caller under its lock and hands back a [`PreparedMemory`] holding what the
//! command needs, and the command itself (store reads, the Project's identity,
//! an `add`'s provider call) runs on hided's blocking task, off the lock.
//! A caller on any linked device is answered from the core's one store;
//! the caller's node reads its Project and session files.

use crate::node::NodeId;
use crate::node_access::NodeLink;
use crate::runtime::RuntimeWorkerContext;
use crate::runtime::memory_analysis::{
    AnalysisFailure, BatchSource, MemoryChange, admit_call, batch_identity, report_memory_change,
    write_cards,
};
use hide_ai::{AiError, AiRouter, AiSettings, CancelToken};
use hide_memory::{CandidateOutcome, MemoryItem, MemoryLifecycle, MemoryStore, card_name};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The most characters `hide memory add` takes (B8).
pub const ADD_LIMIT_CHARS: usize = hide_memory::MEMORY_BODY_LIMIT_CHARS;
/// The most bytes a search's words may take; the matcher reads only the
/// first words anyway.
pub const SEARCH_LIMIT_BYTES: usize = 1024;
/// How long an `add` may spend asking the provider, kept inside the minute
/// B8 gives the whole command so the store is never written after the caller
/// stopped waiting.
pub const ADD_PROVIDER_DEADLINE: Duration = Duration::from_secs(50);
/// How long a `stale` waits for the node to say where the reporting
/// session's conversation ends, listing then file size, so the whole report
/// answers well inside the caller's ten seconds; past it the hold has no
/// place and lasts to its deadline.
const REPORT_OFFSET_BUDGET: Duration = Duration::from_secs(5);
const REPORT_LISTING_TIMEOUT: Duration = Duration::from_secs(3);
/// How long the caller waits for an `add` (B8).
pub const ADD_TIMEOUT: Duration = Duration::from_secs(60);
/// How long before the caller stops waiting an `add` stops writing, for its
/// answer to get back.
const ADD_ANSWER_MARGIN: Duration = Duration::from_secs(5);

/// The provider and session `add` writes its batch under: one per Project
/// and text, so the same text added again is the same batch and never a
/// second card. Once every card of that batch is gone (forgotten, undone,
/// or none was kept), the same text starts the next generation's batch.
const ADD_PROVIDER: &str = "hide";
const ADD_SESSION: &str = "memory-add";
/// The most generations one text is looked up through.
const ADD_GENERATIONS: u32 = 32;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "verb", rename_all = "snake_case")]
pub enum MemoryCommand {
    Show {
        number: u64,
    },
    Search {
        words: String,
    },
    Add {
        text: String,
    },
    /// The operator asked the agent to drop a card; Undo brings it back.
    Forget {
        number: u64,
    },
    /// Steps card `number` back one change.
    Undo {
        number: u64,
    },
    /// The calling agent, given card `number` by a prompt, found it
    /// contradicted by what the operator says now: the card is held out of
    /// Memory until this session is analyzed (PRD memory-stale-cards B1).
    /// The reason stays in the store, never in a provider request.
    Stale {
        number: u64,
        reason: String,
    },
    /// Ends card `number`'s hold at once (B11).
    Release {
        number: u64,
    },
    /// The agent promoting card `number` names the repository file it put
    /// the card in, which the card points at once the pull request merges.
    Placed {
        number: u64,
        path: String,
    },
    /// The agent promoting card `number` found its text outdated: the
    /// attempt ends without a pull request and the card is verified again
    /// (D-12). The reason is what the agent saw, for that verification.
    Outdated {
        number: u64,
        reason: String,
    },
    /// One card was not processed; peers can still reach the shared PR.
    Skipped {
        number: u64,
        reason: String,
    },
    /// The agent promoting card `number` found its decision not built in the
    /// code yet and `branch` building it, with `path` there already carrying
    /// the card: that branch's pull request settles the card.
    Building {
        number: u64,
        branch: String,
        path: String,
    },
    /// The agent promoting card `number` found its decision not built and no
    /// branch building it; the reason is what it saw, shown to the operator.
    Unbuilt {
        number: u64,
        reason: String,
    },
}

impl MemoryCommand {
    /// Whether the command's own values are ones a parser could have made.
    pub fn valid(&self) -> bool {
        match self {
            Self::Show { number }
            | Self::Forget { number }
            | Self::Undo { number }
            | Self::Release { number } => *number > 0,
            Self::Stale { number, reason } => {
                *number > 0
                    && !reason.trim().is_empty()
                    && reason.chars().count() <= hide_memory::STALE_REASON_LIMIT_CHARS
            }
            Self::Search { words } => !words.trim().is_empty() && words.len() <= SEARCH_LIMIT_BYTES,
            Self::Add { text } => {
                !text.trim().is_empty() && text.chars().count() <= ADD_LIMIT_CHARS
            }
            // Never Git's own folder: the path is shown to every agent once
            // the card's pull request merges.
            Self::Outdated { number, reason }
            | Self::Skipped { number, reason }
            | Self::Unbuilt { number, reason } => {
                *number > 0
                    && !reason.trim().is_empty()
                    && reason.chars().count() <= hide_memory::OUTDATED_REASON_LIMIT_CHARS
            }
            Self::Placed { number, path } => {
                *number > 0
                    && hide_memory::repository_path(path)
                    && !path
                        .split('/')
                        .any(|part| part.eq_ignore_ascii_case(".git"))
            }
            Self::Building {
                number,
                branch,
                path,
            } => {
                *number > 0
                    && hide_memory::branch_name(branch)
                    && hide_memory::repository_path(path)
                    && !path
                        .split('/')
                        .any(|part| part.eq_ignore_ascii_case(".git"))
            }
        }
    }
}

/// Whether `branch` is a name `hide memory building` takes: one git would
/// accept for a branch.
pub fn branch_name(branch: &str) -> bool {
    hide_memory::branch_name(branch)
}

/// The number a card name such as `m12` (or a bare `12`) gives.
pub fn parse_card(value: &str) -> Option<u64> {
    hide_memory::parse_card_name(value).filter(|number| *number > 0)
}

/// Who runs a `hide memory` command, as its credential and the core's agent
/// rows say; a release gives the revision's report back only for a person
/// (PRD memory-stale-cards D-10 as amended).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MemoryCaller {
    /// A Herdr pane no agent runs in: a person at its shell.
    Person,
    /// A Herdr pane an agent runs in, with the agent's Memory provider and
    /// session when Herdr knows them.
    Agent(Option<(String, String)>),
    /// A caller bound by its working folder: a terminal outside every pane,
    /// or a tool shell of Codex's shared app-server daemon, which Hide cannot
    /// tell apart.
    Checkout,
}

impl MemoryCaller {
    /// The calling agent's Memory provider and session, when Hide knows them.
    fn session(&self) -> Option<(&str, &str)> {
        match self {
            Self::Agent(Some((provider, session))) => Some((provider, session)),
            _ => None,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Agent(_) => "agent",
            Self::Checkout => "checkout",
        }
    }
}

/// What one command needs, taken under the core's lock.
pub struct PreparedMemory {
    pub(crate) command: MemoryCommand,
    /// The machine holding the caller's checkout, with the link that reads
    /// its files.
    pub(crate) node: NodeId,
    pub(crate) sessions_node: Arc<dyn NodeLink>,
    /// The core's own node, which asks the provider for an `add`.
    pub(crate) ai_node: Arc<dyn NodeLink>,
    /// Where the provider boundary tells what an `add` spent.
    pub(crate) ai_usage: Arc<dyn hide_ai::AiUsageSink>,
    pub(crate) database: PathBuf,
    pub(crate) checkout_path: String,
    pub(crate) caller: MemoryCaller,
    /// Hide AI's settings while it is on.
    pub(crate) settings: Option<AiSettings>,
    /// The core's output language, which an `add`'s cards are written in.
    pub(crate) language: hide_ai::OutputLanguage,
    /// The coordination ids of the live agents in the caller's pane, so only
    /// the agent a promotion started names its file (`placed`).
    pub(crate) caller_agents: Vec<String>,
    /// The caller's repository as the worktree catalog last read it: its
    /// branches, `None` while unread, and the checkout of each branch that
    /// has one, which a `building` branch and its file are checked against.
    pub(crate) branches: Option<Vec<String>>,
    pub(crate) branch_checkouts: Vec<(String, String)>,
    /// Where a change is reported, so the screen reads Memory again.
    pub(crate) context: Option<RuntimeWorkerContext>,
}

impl PreparedMemory {
    /// Runs the command: the answer's `result`, or a refusal code
    /// [`next_action`] explains.
    pub fn run(self) -> Result<Value, String> {
        let node = Arc::clone(&self.ai_node);
        let usage = Arc::clone(&self.ai_usage);
        self.run_with(|settings| crate::ai::memory_add_router(&node, settings, &usage))
    }

    fn run_with(self, router: impl FnOnce(&AiSettings) -> AiRouter) -> Result<Value, String> {
        let deadline = Instant::now() + ADD_TIMEOUT - ADD_ANSWER_MARGIN;
        if !MemoryStore::exists(&self.database) {
            return Err("memory_off".into());
        }
        let identity = crate::runtime::memory::project_identity(
            self.sessions_node.as_ref(),
            &self.node,
            &self.checkout_path,
        )
        .map_err(|message| {
            crate::diagnostic!(json!({
                "component": "memory",
                "kind": "command.project_unavailable",
                "message": message,
            }));
            "memory_project_unavailable".to_owned()
        })?;
        let root = identity.root.to_string_lossy().into_owned();
        let mut store =
            crate::runtime::memory::open_memory_store(&self.database).map_err(|error| {
                crate::diagnostic!(json!({
                    "component": "memory",
                    "kind": "command.store_unavailable",
                    "message": error.to_string(),
                }));
                "memory_unavailable".to_owned()
            })?;
        let enabled = match store.project_state(&identity.id) {
            Ok(state) => state.enabled,
            Err(hide_memory::MemoryError::ProjectMissing) => false,
            Err(error) => return Err(store_failure(&identity.id, &error)),
        };
        if !enabled {
            return Err("memory_off".into());
        }
        let now = crate::runtime::memory::now_ms();
        match self.command.clone() {
            MemoryCommand::Show { number } => {
                let item = store
                    .show(&identity.id, number, self.caller.session(), now)
                    .map_err(|error| store_failure(&identity.id, &error))?
                    .ok_or("memory_card_not_found")?;
                if item.lifecycle == MemoryLifecycle::Tombstoned {
                    return Err("memory_card_forgotten".into());
                }
                self.report(&identity.id, MemoryChange::Other);
                let evidence = store
                    .detail(&identity.id, &item.id)
                    .map_err(|error| store_failure(&identity.id, &error))?
                    .evidence;
                let mut answer = card(&item);
                answer["evidence"] = json!(evidence);
                answer["hold"] = match store
                    .hold(&identity.id, number, now)
                    .map_err(|error| store_failure(&identity.id, &error))?
                {
                    Some(hold) => json!({
                        "reason": hold.reason,
                        "reported_at": utc(hold.reported_at_unix_ms),
                        "expires_at": utc(hold.expires_at_unix_ms),
                    }),
                    None => Value::Null,
                };
                answer["document_comparison"] = json!(
                    store
                        .document_comparison(&identity.id, number)
                        .map_err(|error| store_failure(&identity.id, &error))?
                );
                Ok(answer)
            }
            MemoryCommand::Forget { number } => {
                let item = store
                    .item_by_number(&identity.id, number)
                    .map_err(|error| store_failure(&identity.id, &error))?
                    .ok_or("memory_card_not_found")?;
                if item.lifecycle == MemoryLifecycle::Tombstoned {
                    return Err("memory_card_forgotten".into());
                }
                store
                    .forget(&identity.id, &item.id)
                    .map_err(|error| store_failure(&identity.id, &error))?;
                self.report(&identity.id, MemoryChange::Forgot(number));
                Ok(json!({"card": card_name(number), "outcome": "forgotten"}))
            }
            MemoryCommand::Undo { number } => {
                let item = store
                    .undo_card(&identity.id, number)
                    .map_err(|error| store_failure(&identity.id, &error))?
                    .ok_or("memory_nothing_to_undo")?;
                self.report(&identity.id, MemoryChange::Undid(number));
                Ok(json!({
                    "card": card_name(number),
                    "outcome": if item.lifecycle == MemoryLifecycle::Tombstoned {
                        "removed"
                    } else {
                        "restored"
                    },
                }))
            }
            MemoryCommand::Stale { number, reason } => {
                let Some((runtime, session_id)) = self.caller.session() else {
                    stale_refused(&identity.id, number, None, "memory_stale_session_required");
                    return Err("memory_stale_session_required".into());
                };
                let report_offset = self.report_offset(&identity, runtime, session_id);
                let outcome = store
                    .report_stale(
                        &identity.id,
                        number,
                        &hide_memory::StaleReporter {
                            runtime,
                            session_id,
                            report_offset,
                        },
                        &reason,
                        now,
                    )
                    .map_err(|error| store_failure(&identity.id, &error))?;
                let hold = match outcome {
                    hide_memory::StaleOutcome::Held(hold) => hold,
                    hide_memory::StaleOutcome::Refused(refusal) => {
                        stale_refused(
                            &identity.id,
                            number,
                            Some((runtime, session_id)),
                            refusal.code(),
                        );
                        return Err(refusal.code().into());
                    }
                };
                crate::diagnostic!(json!({
                    "component": "memory",
                    "kind": "stale.held",
                    "project_id": identity.id,
                    "card": number,
                    "runtime": runtime,
                    "session_id": session_id,
                    "report_offset": report_offset,
                    "reason_chars": hold.reason.chars().count(),
                }));
                self.report(&identity.id, MemoryChange::Other);
                Ok(json!({
                    "card": card_name(number),
                    "outcome": "held",
                    "expires_at": utc(hold.expires_at_unix_ms),
                }))
            }
            MemoryCommand::Release { number } => {
                let item = store
                    .item_by_number(&identity.id, number)
                    .map_err(|error| store_failure(&identity.id, &error))?
                    .ok_or("memory_card_not_found")?;
                // Only a person undoes a false report; an agent's release, and
                // one Hide cannot tell from an agent's, leaves it spent.
                let report = if self.caller == MemoryCaller::Person {
                    hide_memory::ReleasedReport::GivenBack
                } else {
                    hide_memory::ReleasedReport::Spent
                };
                let ended = store
                    .release_hold(&identity.id, item.number, now, report)
                    .map_err(|error| store_failure(&identity.id, &error))?
                    .ok_or("memory_card_not_held")?;
                crate::diagnostic!(json!({
                    "component": "memory",
                    "kind": "stale.released",
                    "project_id": identity.id,
                    "card": number,
                    "runtime": ended.runtime,
                    "session_id": ended.session_id,
                    "cause": "release",
                    "caller": self.caller.code(),
                    "caller_session": self.caller.session().map(|(runtime, session)| json!({
                        "runtime": runtime,
                        "session_id": session,
                    })),
                    "report": report.code(),
                }));
                self.report(&identity.id, MemoryChange::Other);
                Ok(json!({
                    "card": card_name(number),
                    "outcome": "released",
                    "caller": self.caller.code(),
                    "report": report.code(),
                }))
            }
            MemoryCommand::Placed { number, path } => {
                let promotion = self.own_promotion(&store, &identity.id, number)?;
                // The file must be in the caller's checkout, the worktree the
                // promotion's agent changed it in.
                let inside = crate::runtime::memory_analysis::paths_inside(
                    self.sessions_node.as_ref(),
                    &self.checkout_path,
                    std::slice::from_ref(&path),
                );
                if !inside.contains(&path) {
                    return Err("memory_path_not_found".into());
                }
                // Only the path is written, and only while this attempt and
                // agent still hold the card, so a tracker move or a retry in
                // between stands.
                if !store
                    .place_promotion(&identity.id, number, &promotion, &path, now)
                    .map_err(|error| store_failure(&identity.id, &error))?
                {
                    return Err("memory_card_not_promoting".into());
                }
                self.report(&identity.id, MemoryChange::Other);
                Ok(json!({"card": card_name(number), "path": path}))
            }
            command @ (MemoryCommand::Outdated { .. }
            | MemoryCommand::Skipped { .. }
            | MemoryCommand::Unbuilt { .. }) => {
                let (outcome, number, reason) = match command {
                    MemoryCommand::Outdated { number, reason } => ("outdated", number, reason),
                    MemoryCommand::Skipped { number, reason } => ("skipped", number, reason),
                    MemoryCommand::Unbuilt { number, reason } => ("unbuilt", number, reason),
                    _ => unreachable!(),
                };
                let promotion = self.own_promotion(&store, &identity.id, number)?;
                // Only while this attempt and agent still hold the card.
                let moved = match outcome {
                    "skipped" => {
                        store.skip_promotion(&identity.id, number, &promotion, &reason, now, None)
                    }
                    "unbuilt" => {
                        store.leave_unbuilt(&identity.id, number, &promotion, &reason, now)
                    }
                    _ => store.mark_outdated(&identity.id, number, &promotion, &reason, now),
                };
                if !moved.map_err(|error| store_failure(&identity.id, &error))? {
                    return Err("memory_card_not_promoting".into());
                }
                crate::diagnostic!(json!({
                    "component": "memory",
                    "kind": format!("promotion.{outcome}"),
                    "project_id": identity.id,
                    "card": number,
                    "reason_chars": reason.chars().count(),
                }));
                self.report(
                    &identity.id,
                    if outcome == "outdated" {
                        MemoryChange::Promotion
                    } else {
                        MemoryChange::Other
                    },
                );
                Ok(json!({"card": card_name(number), "outcome": outcome}))
            }
            MemoryCommand::Building {
                number,
                branch,
                path,
            } => {
                let promotion = self.own_promotion(&store, &identity.id, number)?;
                // The attempt's own branch is where the card would be placed,
                // never the work that builds it.
                if promotion.branch.as_deref() == Some(branch.as_str()) {
                    return Err("memory_branch_is_the_attempts".into());
                }
                if self
                    .branches
                    .as_ref()
                    .is_some_and(|branches| !branches.contains(&branch))
                {
                    return Err("memory_branch_not_found".into());
                }
                // The file must be on that branch, where its checkout shows it.
                if let Some((_, checkout)) = self
                    .branch_checkouts
                    .iter()
                    .find(|(name, _)| *name == branch)
                    && !crate::runtime::memory_analysis::paths_inside(
                        self.sessions_node.as_ref(),
                        checkout,
                        std::slice::from_ref(&path),
                    )
                    .contains(&path)
                {
                    return Err("memory_building_path_not_found".into());
                }
                if !store
                    .build_promotion(&identity.id, number, &promotion, &branch, &path, now)
                    .map_err(|error| store_failure(&identity.id, &error))?
                {
                    return Err("memory_card_not_promoting".into());
                }
                crate::diagnostic!(json!({
                    "component": "memory",
                    "kind": "promotion.building",
                    "project_id": identity.id,
                    "card": number,
                }));
                self.report(&identity.id, MemoryChange::Other);
                Ok(
                    json!({"card": card_name(number), "outcome": "building", "branch": branch, "path": path}),
                )
            }
            MemoryCommand::Search { words } => {
                let items = store
                    .search(&identity.id, &words, hide_memory::SEARCH_RESULT_LIMIT)
                    .map_err(|error| store_failure(&identity.id, &error))?;
                Ok(json!({
                    "cards": items.iter().map(|item| json!({
                        "card": card_name(item.number),
                        "title": item.title,
                        "kind": item.kind.as_str(),
                    })).collect::<Vec<_>>(),
                }))
            }
            MemoryCommand::Add { text } => {
                let router = router(self.settings.as_ref().ok_or("memory_ai_off")?);
                let group = [json!({
                    "offset": 0,
                    "role": "assistant",
                    "kind": "agent_note",
                    "text": text.trim(),
                })];
                let serialized = serde_json::to_string(&group).map_err(|error| {
                    crate::diagnostic!(json!({
                        "component": "memory", "kind": "command.add_unencodable",
                        "message": error.to_string(),
                    }));
                    "memory_unavailable".to_owned()
                })?;
                let mut generation = 0;
                let session_id = loop {
                    if generation == ADD_GENERATIONS {
                        crate::diagnostic!(json!({
                            "component": "memory", "kind": "command.add_generations_spent",
                            "project_id": identity.id,
                        }));
                        return Err("memory_unavailable".into());
                    }
                    let session_id = match generation {
                        0 => ADD_SESSION.to_owned(),
                        n => format!("{ADD_SESSION}-{n}"),
                    };
                    let source = self.add_source(&identity.id, &root, &session_id, deadline);
                    let (_, batch_id) = batch_identity(&source, &serialized);
                    if !store
                        .has_batch(&batch_id)
                        .map_err(|error| store_failure(&identity.id, &error))?
                    {
                        break session_id;
                    }
                    let live = live_cards(&store, &identity.id, &batch_id)?;
                    if !live.is_empty() {
                        return Ok(json!({
                            "outcomes": live.into_iter().map(|number| json!({
                                "outcome": "already_added",
                                "card": card_name(number),
                            })).collect::<Vec<_>>(),
                            "batch_id": batch_id,
                        }));
                    }
                    generation += 1;
                };
                let written = write_cards(
                    &mut store,
                    self.sessions_node.as_ref(),
                    &router,
                    self.language,
                    &self.add_source(&identity.id, &root, &session_id, deadline),
                    &group,
                    &CancelToken::new(),
                    &mut |store| admit_call(store),
                )
                .map_err(|failure| add_failure(&identity.id, failure))?;
                self.report(&identity.id, MemoryChange::Other);
                Ok(json!({
                    "outcomes": written.summary.outcomes.iter().map(outcome).collect::<Vec<_>>(),
                    "batch_id": written.batch_id,
                }))
            }
        }
    }

    /// Where the reporting session's conversation ends now, as the node
    /// holding it says (a file's length, or an OpenCode session's message
    /// count): the hold ends once analysis reads past it (D-14).
    /// `None` when the node cannot say, and the hold lasts to its deadline.
    fn report_offset(
        &self,
        identity: &hide_project::ProjectIdentity,
        runtime: &str,
        session_id: &str,
    ) -> Option<u64> {
        let deadline = Instant::now() + REPORT_OFFSET_BUDGET;
        let stat = crate::runtime::memory::project_sessions(
            self.sessions_node.as_ref(),
            identity,
            REPORT_LISTING_TIMEOUT,
        )
        .and_then(|sessions| {
            sessions
                .into_iter()
                .find(|session| session.agent.as_str() == runtime && session.id == session_id)
                .ok_or_else(|| "session_not_listed".to_owned())
        })
        .and_then(|session| {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("report_offset_budget_spent".to_owned());
            }
            crate::runtime::memory::session_stat(self.sessions_node.as_ref(), &session, remaining)
        });
        match stat {
            Ok(stat) => Some(stat.size),
            Err(message) => {
                crate::diagnostic!(json!({
                    "component": "memory",
                    "kind": "stale.offset_unavailable",
                    "project_id": identity.id,
                    "runtime": runtime,
                    "session_id": session_id,
                    "reason": match message.as_str() {
                        "session_not_listed" | "report_offset_budget_spent" => message.as_str(),
                        _ => "node_call_failed",
                    },
                    "message": message,
                }));
                None
            }
        }
    }

    fn add_source<'a>(
        &'a self,
        project_id: &'a str,
        root: &'a str,
        session_id: &'a str,
        deadline: Instant,
    ) -> BatchSource<'a> {
        BatchSource {
            project_id,
            provider: ADD_PROVIDER,
            session_id,
            checkout_path: &self.checkout_path,
            evidence_root: root,
            new_card_limit: None,
            deadline: Some(deadline),
        }
    }

    /// The promotion of card `number` that the caller's own agent holds:
    /// running or with its pull request open. An attempt whose agent Hide
    /// has not seen yet records it on a later check, and its agent asks
    /// again then.
    fn own_promotion(
        &self,
        store: &MemoryStore,
        project_id: &str,
        number: u64,
    ) -> Result<hide_memory::Promotion, String> {
        let promotions = store
            .promotions(project_id)
            .map_err(|error| store_failure(project_id, &error))?;
        if promotions.iter().any(|(card, promotion)| {
            *card == number
                && promotion.state == hide_memory::PromotionState::Running
                && promotion.agent_id.is_none()
        }) {
            return Err("memory_promotion_agent_pending".into());
        }
        promotions
            .into_iter()
            .find(|(card, _)| *card == number)
            .filter(|(_, promotion)| {
                matches!(
                    promotion.state,
                    hide_memory::PromotionState::Running | hide_memory::PromotionState::PrOpen
                ) && promotion
                    .agent_id
                    .as_ref()
                    .is_some_and(|id| self.caller_agents.contains(id))
            })
            .map(|(_, promotion)| promotion)
            .ok_or_else(|| "memory_card_not_promoting".to_owned())
    }

    fn report(&self, project_id: &str, change: MemoryChange) {
        if let Some(context) = &self.context {
            report_memory_change(context, project_id, change);
        }
    }
}

/// A time as `hide memory show` prints it, in UTC.
fn utc(unix_ms: u64) -> String {
    jiff::Timestamp::from_millisecond(unix_ms as i64)
        .map(|time| time.strftime("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_default()
}

/// A refused report, by card, session and refusal; never the reason given.
fn stale_refused(project_id: &str, number: u64, reporter: Option<(&str, &str)>, code: &str) {
    crate::diagnostic!(json!({
        "component": "memory",
        "kind": "stale.refused",
        "project_id": project_id,
        "card": number,
        "runtime": reporter.map(|(runtime, _)| runtime),
        "session_id": reporter.map(|(_, session)| session),
        "reason": code,
    }));
}

/// The cards of a written batch that are still in use.
fn live_cards(store: &MemoryStore, project_id: &str, batch_id: &str) -> Result<Vec<u64>, String> {
    let mut live = Vec::new();
    for number in store
        .batch_card_numbers(batch_id)
        .map_err(|error| store_failure(project_id, &error))?
    {
        let item = store
            .item_by_number(project_id, number)
            .map_err(|error| store_failure(project_id, &error))?;
        if item.is_some_and(|item| item.lifecycle == MemoryLifecycle::Active) {
            live.push(number);
        }
    }
    Ok(live)
}

fn card(item: &MemoryItem) -> Value {
    json!({
        "card": card_name(item.number),
        "title": item.title,
        "kind": item.kind.as_str(),
        "state": item.lifecycle.as_str(),
        "body": item.body,
        "sources": item.source_count,
        "operator_sources": item.human_source_count,
        "cited_paths": item.cited_paths,
        "verifying": item.verifying,
    })
}

fn outcome(outcome: &CandidateOutcome) -> Value {
    match outcome {
        CandidateOutcome::Created { number } => {
            json!({"outcome": "added", "card": card_name(*number)})
        }
        CandidateOutcome::Merged { number } => {
            json!({"outcome": "same_as", "card": card_name(*number)})
        }
        CandidateOutcome::Superseded { number } => {
            json!({"outcome": "replaced", "card": card_name(*number)})
        }
        // The card rests on the operator's own words; the added text is
        // kept in its history and never given (D-14, B7).
        CandidateOutcome::Kept { number } => {
            json!({"outcome": "outranked", "card": card_name(*number)})
        }
        // `hide memory add` sets no new-card limit, so the limit cannot be
        // reached here; it is the analysis pass's alone (D-02).
        CandidateOutcome::Discarded | CandidateOutcome::SessionLimit => {
            json!({"outcome": "not_kept"})
        }
        CandidateOutcome::SecretRejected => json!({"outcome": "secret"}),
        CandidateOutcome::CapacityRejected => json!({"outcome": "full"}),
    }
}

fn store_failure(project_id: &str, error: &hide_memory::MemoryError) -> String {
    crate::diagnostic!(json!({
        "component": "memory",
        "kind": "command.store_failed",
        "project_id": project_id,
        "message": error.to_string(),
    }));
    "memory_unavailable".into()
}

/// A failed `add` wrote nothing: the batch is written only after the
/// provider answered and the answer was checked.
fn add_failure(project_id: &str, failure: AnalysisFailure) -> String {
    let code = match &failure {
        AnalysisFailure::Provider(AiError::NotAuthenticated) => "memory_ai_sign_in",
        AnalysisFailure::Provider(AiError::Timeout) => "memory_add_timeout",
        AnalysisFailure::Provider(AiError::UsageLimited { .. }) => "memory_ai_usage_limited",
        AnalysisFailure::Provider(_) => "memory_ai_failed",
        AnalysisFailure::Deferred(_) => "memory_daily_cap",
        AnalysisFailure::Cancelled => "memory_add_timeout",
        AnalysisFailure::Disabled => "memory_off",
        AnalysisFailure::Local(_) => "memory_unavailable",
    };
    crate::diagnostic!(json!({
        "component": "memory",
        "kind": "command.add_failed",
        "project_id": project_id,
        "reason": code,
        "message": match &failure {
            AnalysisFailure::Provider(error) => error.to_string(),
            AnalysisFailure::Local(message) => message.clone(),
            AnalysisFailure::Deferred(reason) => (*reason).to_owned(),
            AnalysisFailure::Cancelled => "cancelled".to_owned(),
            AnalysisFailure::Disabled => "disabled".to_owned(),
        },
    }));
    code.into()
}

/// What the caller can do about a refusal.
pub fn next_action(code: &str) -> &'static str {
    match code {
        "link_down" => "Reconnect this device in Hide, then retry",
        "unregistered" => "Register this device in Hide, then retry",
        "session_reader_support_unavailable" | "session_reader_memory_unsupported" => {
            "Update this device's Hide helper to one that supports Project Memory, then reconnect"
        }
        "memory_off" => "Memory is off for this Project; only the operator can turn it on in Hide",
        "memory_ai_off" => "Turn Hide AI on and choose an agent in Hide's Settings",
        "memory_ai_sign_in" => "Sign in to the agent Hide AI uses, then retry",
        "memory_ai_usage_limited" => "Retry after the agent's usage limit resets",
        "memory_daily_cap" => "Retry tomorrow; this machine reached its daily Memory calls",
        "memory_add_timeout" => "Nothing was saved; retry with shorter text",
        "memory_card_not_found" => "Find the card with hide memory search <words>",
        "memory_card_forgotten" => {
            "The card was forgotten; find another with hide memory search <words>"
        }
        "memory_text_too_long" => "Shorten the text to 4,000 characters and retry",
        "memory_stale_reason_invalid" => {
            "Give a reason of at most 500 characters saying what the operator said that contradicts the card"
        }
        "memory_stale_session_required" => {
            "Only an agent whose session Hide knows can report a card; run it from the agent's own pane"
        }
        "memory_stale_already_reported" => {
            "This card's text has its one report already; tell the operator instead, and hide memory show <card> says whether it is held"
        }
        "memory_stale_not_received" => {
            "Only a session a prompt gave this card to can report it; if one did just now, retry after Hide reads this conversation, within a minute"
        }
        "memory_stale_shown_only" => {
            "Reading a card with hide memory show does not let a session report it; only a card a prompt gave this session can be reported"
        }
        "memory_stale_receipts_full" => {
            "Hide cannot tell which cards this session was given because this Project's delivery records are full; tell the operator instead"
        }
        "memory_stale_session_limit" => {
            "This session already holds 3 cards; tell the operator about this one instead"
        }
        "memory_stale_project_limit" => {
            "This Project already holds 10 cards; tell the operator about this one instead"
        }
        "memory_card_not_held" => "The card is not held; hide memory show <card> shows it",
        "memory_card_not_promoting" => {
            "Only the agent Hide started to put this card in the repository names its file or says where it stands"
        }
        "memory_promotion_agent_pending" => {
            "Hide has not confirmed this agent yet; run the same command again in a minute"
        }
        "memory_reason_invalid" => {
            "Give a reason of at most 500 characters saying what in the card is outdated"
        }
        "memory_path_not_found" => {
            "Name a file that exists in this checkout, relative to its root, and retry"
        }
        "memory_branch_not_found" => {
            "Name a branch of this repository that builds the card, as git branch lists it, and retry"
        }
        "memory_branch_is_the_attempts" => {
            "Name the branch that builds the card, not this checkout's own, and retry"
        }
        "memory_building_path_not_found" => {
            "Name the file on that branch's checkout that carries the card, relative to its root, and retry"
        }
        "agent_pane_required" | "caller_context_changed" => {
            "Run the command from the agent's own pane and retry"
        }
        _ => "Retry; the reason is in Hide's diagnostic log",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hide_ai::{
        AiBackend, AiRequest, AiResponse, Availability, ModelCatalog, NoopLogSink, ProviderId,
        RouterConfig,
    };
    use hide_memory::{AnalysisBatch, Candidate, CandidateKind, CandidateRelation};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Answers each request with the next scripted value and counts calls.
    struct Scripted {
        answers: Mutex<Vec<Value>>,
        calls: AtomicUsize,
    }

    impl AiBackend for Scripted {
        fn id(&self) -> ProviderId {
            ProviderId::CLAUDE
        }
        fn availability(&self) -> Availability {
            Availability::Ready
        }
        fn models(&self) -> ModelCatalog {
            ModelCatalog::Offered(vec!["fixture".to_owned()])
        }
        fn execute(&self, _: &AiRequest, _: &CancelToken) -> Result<AiResponse, AiError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let value = self.answers.lock().unwrap().pop();
            value
                .map(|value| AiResponse {
                    value,
                    usage: Default::default(),
                })
                .ok_or_else(|| AiError::Transient("no scripted answer".to_owned()))
        }
    }

    struct Fixture {
        _temp: tempfile::TempDir,
        root: String,
        database: PathBuf,
        project_id: String,
        node: Arc<dyn NodeLink>,
        backend: Arc<Scripted>,
    }

    impl Fixture {
        /// A Project with Memory on and card m1, which cites `src/lib.rs`.
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("project");
            std::fs::create_dir_all(root.join("src")).unwrap();
            let root = std::fs::canonicalize(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let node: Arc<dyn NodeLink> =
                Arc::new(hide_node::Local::new(Some(temp.path().join("home"))));
            let identity = crate::runtime::memory::project_identity(
                node.as_ref(),
                &crate::node::test_node(),
                &root,
            )
            .unwrap();
            let database = temp.path().join("memory.sqlite3");
            let mut store = MemoryStore::open(&database).unwrap();
            store
                .ensure_project(&identity.id, &identity.root, crate::node::TEST_NODE)
                .unwrap();
            store.set_enabled(&identity.id, true, true).unwrap();
            store
                .apply_candidates(
                    &AnalysisBatch {
                        id: "seed".to_owned(),
                        project_id: identity.id.clone(),
                        provider: "codex".to_owned(),
                        analysis_provider: "codex".to_owned(),
                        session_id: "seed".to_owned(),
                        content_hash: "seed".to_owned(),
                        created_at_unix_ms: 1,
                    },
                    &[Candidate {
                        text: "Run tests with nextest before a merge".to_owned(),
                        title: "Tests run with nextest".to_owned(),
                        aliases: vec!["테스트".to_owned()],
                        kind: CandidateKind::Rule,
                        confidence: 0.9,
                        salience: 0.9,
                        source_offsets: vec![1],
                        human_offsets: vec![1],
                        source_times: Default::default(),
                        cited_paths: vec!["src/lib.rs".to_owned()],
                        relation: CandidateRelation::New,
                    }],
                )
                .unwrap();
            Self {
                _temp: temp,
                root,
                database,
                project_id: identity.id,
                node,
                backend: Arc::new(Scripted {
                    answers: Mutex::new(Vec::new()),
                    calls: AtomicUsize::new(0),
                }),
            }
        }

        fn run(&self, command: MemoryCommand, ai: bool) -> Result<Value, String> {
            self.run_in_repository(command, ai, None, Vec::new())
        }

        /// Runs `command` with the caller's repository read as having
        /// `branches` and the checkouts `branch_checkouts`.
        fn run_in_repository(
            &self,
            command: MemoryCommand,
            ai: bool,
            branches: Option<Vec<String>>,
            branch_checkouts: Vec<(String, String)>,
        ) -> Result<Value, String> {
            self.run_as(
                command,
                ai,
                branches,
                branch_checkouts,
                MemoryCaller::Agent(Some(("claude".to_owned(), "caller-session".to_owned()))),
            )
        }

        /// Runs `command` for `caller`.
        fn run_as(
            &self,
            command: MemoryCommand,
            ai: bool,
            branches: Option<Vec<String>>,
            branch_checkouts: Vec<(String, String)>,
            caller: MemoryCaller,
        ) -> Result<Value, String> {
            let backend: Arc<dyn AiBackend> = self.backend.clone();
            PreparedMemory {
                command,
                node: crate::node::test_node(),
                sessions_node: Arc::clone(&self.node),
                ai_node: Arc::clone(&self.node),
                ai_usage: crate::usage_ledger::ai_usage::AiUsageQueue::new(),
                database: self.database.clone(),
                checkout_path: self.root.clone(),
                caller,
                settings: ai.then(AiSettings::default),
                language: hide_ai::OutputLanguage::English,
                caller_agents: vec!["agent-9".to_owned()],
                branches,
                branch_checkouts,
                context: None,
            }
            .run_with(|_| {
                AiRouter::new(
                    vec![backend],
                    RouterConfig {
                        priority: vec![ProviderId::CLAUDE],
                        max_transient_attempts: 1,
                        ..RouterConfig::default()
                    },
                    Arc::new(NoopLogSink),
                )
            })
        }

        fn store(&self) -> MemoryStore {
            MemoryStore::open(&self.database).unwrap()
        }

        fn calls(&self) -> usize {
            self.backend.calls.load(Ordering::SeqCst)
        }
    }

    impl Fixture {
        fn stale(&self, caller: MemoryCaller) -> Result<Value, String> {
            self.run_as(
                MemoryCommand::Stale {
                    number: 1,
                    reason: "The operator now says api_key=super-secret-value-123 is gone"
                        .to_owned(),
                },
                false,
                None,
                Vec::new(),
                caller,
            )
        }

        fn release(&self, caller: MemoryCaller) -> Result<Value, String> {
            self.run_as(
                MemoryCommand::Release { number: 1 },
                false,
                None,
                Vec::new(),
                caller,
            )
        }

        /// A prompt gave `session` card `m1` as it is now.
        fn give(&self, session: &str) {
            self.store()
                .record_receipt(
                    &self.project_id,
                    "claude",
                    session,
                    "event:40",
                    "m1@1",
                    crate::runtime::memory::now_ms(),
                )
                .unwrap();
        }
    }

    fn agent(session: &str) -> MemoryCaller {
        MemoryCaller::Agent(Some(("claude".to_owned(), session.to_owned())))
    }

    #[test]
    fn a_card_a_prompt_gave_the_caller_is_held_shown_and_an_agents_release_spends_its_report() {
        // PRD memory-stale-cards B1-B3, B5, B11, D-10 as amended.
        let fixture = Fixture::new();
        let caller = || agent("caller-session");
        assert_eq!(
            fixture.stale(caller()),
            Err("memory_stale_not_received".into())
        );
        for nobody in [
            MemoryCaller::Person,
            MemoryCaller::Agent(None),
            MemoryCaller::Checkout,
        ] {
            assert_eq!(
                fixture.stale(nobody),
                Err("memory_stale_session_required".into())
            );
        }
        fixture.give("caller-session");
        let held = fixture.stale(caller()).unwrap();
        assert_eq!(held["card"], "m1");
        assert_eq!(held["outcome"], "held");
        let shown = fixture
            .run(MemoryCommand::Show { number: 1 }, false)
            .unwrap();
        let reason = shown["hold"]["reason"].as_str().unwrap();
        assert!(reason.starts_with("The operator now says"));
        assert!(!reason.contains("super-secret"), "{reason}");
        assert_eq!(
            fixture.stale(caller()),
            Err("memory_stale_already_reported".into())
        );

        let released = fixture.release(caller()).unwrap();
        assert_eq!(
            (
                &released["outcome"],
                &released["caller"],
                &released["report"]
            ),
            (&json!("released"), &json!("agent"), &json!("spent"))
        );
        assert_eq!(
            fixture.release(MemoryCaller::Person),
            Err("memory_card_not_held".into()),
            "a released card holds nothing a person could give back"
        );
        assert_eq!(
            fixture
                .run(MemoryCommand::Show { number: 1 }, false)
                .unwrap()["hold"],
            Value::Null
        );
        assert_eq!(
            fixture.stale(caller()),
            Err("memory_stale_already_reported".into())
        );
        assert_eq!(
            fixture.run(MemoryCommand::Release { number: 7 }, false),
            Err("memory_card_not_found".into())
        );
        for code in [
            "memory_stale_session_required",
            "memory_stale_already_reported",
            "memory_stale_not_received",
            "memory_stale_shown_only",
            "memory_stale_receipts_full",
            "memory_stale_session_limit",
            "memory_stale_project_limit",
            "memory_stale_reason_invalid",
            "memory_card_not_held",
        ] {
            assert_ne!(
                next_action(code),
                next_action("unknown"),
                "{code} tells the agent what to do"
            );
        }
    }

    #[test]
    fn only_a_release_from_a_pane_with_no_agent_gives_the_report_back() {
        // D-10 as amended: a person at a Hide shell undoes a false report; a
        // terminal outside every pane may be Codex's shared daemon, so its
        // release leaves the report spent.
        let fixture = Fixture::new();
        fixture.give("first");
        fixture.give("second");
        fixture.stale(agent("first")).unwrap();
        let released = fixture.release(MemoryCaller::Person).unwrap();
        assert_eq!(
            (&released["caller"], &released["report"]),
            (&json!("person"), &json!("given_back"))
        );

        assert_eq!(fixture.stale(agent("second")).unwrap()["outcome"], "held");
        assert_eq!(
            fixture.stale(agent("first")),
            Err("memory_stale_already_reported".into()),
            "the report given back is one report again"
        );
        let released = fixture.release(MemoryCaller::Checkout).unwrap();
        assert_eq!(
            (&released["caller"], &released["report"]),
            (&json!("checkout"), &json!("spent"))
        );
        assert_eq!(
            fixture.stale(agent("first")),
            Err("memory_stale_already_reported".into())
        );
    }

    #[test]
    fn only_a_card_on_its_way_into_the_repository_takes_the_file_it_was_put_in() {
        // B30, B31: the promoting agent names the file the merged card
        // points at; it must be a file in its checkout.
        let fixture = Fixture::new();
        std::fs::write(format!("{}/src/lib.rs", fixture.root), "").unwrap();
        let placed = |path: &str| {
            fixture.run(
                MemoryCommand::Placed {
                    number: 1,
                    path: path.to_owned(),
                },
                false,
            )
        };
        assert_eq!(
            placed("src/lib.rs"),
            Err("memory_card_not_promoting".into())
        );

        // Another agent's promotion: only the agent Hide started names the file.
        let theirs = hide_memory::Promotion {
            state: hide_memory::PromotionState::Running,
            revision: 1,
            attempt: 1,
            agent_id: Some("agent-1".into()),
            branch: Some("hide-memory-m1".into()),
            pr_number: None,
            repository: None,
            target_path: None,
            started_at_unix_ms: 1,
            updated_at_unix_ms: 1,
            failure: None,
            building_branch: None,
        };
        fixture
            .store()
            .set_promotion(&fixture.project_id, 1, &theirs)
            .unwrap();
        assert_eq!(
            placed("src/lib.rs"),
            Err("memory_card_not_promoting".into())
        );

        // An attempt whose agent Hide has not confirmed yet: ask again later.
        fixture
            .store()
            .set_promotion(
                &fixture.project_id,
                1,
                &hide_memory::Promotion {
                    agent_id: None,
                    ..theirs.clone()
                },
            )
            .unwrap();
        assert_eq!(
            placed("src/lib.rs"),
            Err("memory_promotion_agent_pending".into())
        );

        let running = hide_memory::Promotion {
            state: hide_memory::PromotionState::Running,
            revision: 1,
            attempt: 1,
            agent_id: Some("agent-9".into()),
            branch: Some("hide-memory-m1".into()),
            pr_number: None,
            repository: None,
            target_path: None,
            started_at_unix_ms: 1,
            updated_at_unix_ms: 1,
            failure: None,
            building_branch: None,
        };
        fixture
            .store()
            .set_promotion(&fixture.project_id, 1, &running)
            .unwrap();
        assert_eq!(
            placed("src/missing.rs"),
            Err("memory_path_not_found".into())
        );
        assert_eq!(
            placed("src/lib.rs").unwrap(),
            json!({"card": "m1", "path": "src/lib.rs"})
        );
        let promotions = fixture.store().promotions(&fixture.project_id).unwrap();
        assert_eq!(promotions[0].1.target_path.as_deref(), Some("src/lib.rs"));
    }

    #[test]
    fn the_agent_placing_a_card_calls_it_outdated_and_the_card_waits_to_be_verified() {
        // D-12, B17: the attempt ends without a pull request.
        let fixture = Fixture::new();
        let outdated = || {
            fixture.run(
                MemoryCommand::Outdated {
                    number: 1,
                    reason: "the script it names is gone".to_owned(),
                },
                false,
            )
        };
        assert_eq!(outdated(), Err("memory_card_not_promoting".into()));
        let mut attempt = hide_memory::Promotion {
            state: hide_memory::PromotionState::Running,
            revision: 1,
            attempt: 1,
            agent_id: Some("agent-1".into()),
            branch: Some("hide-memory-m1".into()),
            pr_number: None,
            repository: None,
            target_path: None,
            started_at_unix_ms: 1,
            updated_at_unix_ms: 1,
            failure: None,
            building_branch: None,
        };
        fixture
            .store()
            .set_promotion(&fixture.project_id, 1, &attempt)
            .unwrap();
        assert_eq!(
            outdated(),
            Err("memory_card_not_promoting".into()),
            "another agent's attempt"
        );
        attempt.agent_id = Some("agent-9".into());
        fixture
            .store()
            .set_promotion(&fixture.project_id, 1, &attempt)
            .unwrap();
        assert_eq!(
            outdated().unwrap(),
            json!({"card": "m1", "outcome": "outdated"})
        );
        assert_eq!(
            fixture.store().promotions(&fixture.project_id).unwrap()[0]
                .1
                .state,
            hide_memory::PromotionState::Outdated
        );
        let card = fixture
            .run(MemoryCommand::Show { number: 1 }, false)
            .unwrap();
        assert_eq!(card["verifying"], true);
        assert!(
            !MemoryCommand::Outdated {
                number: 1,
                reason: " ".into()
            }
            .valid()
        );
    }

    #[test]
    fn the_agent_placing_an_unbuilt_card_names_the_branch_that_builds_it_or_says_none_does() {
        // A card whose decision the code does not have yet is not a document
        // to correct: the attempt ends building, on the branch that builds
        // it and the file there that carries it, or unbuilt with the reason.
        let fixture = Fixture::new();
        let builder = format!("{}/builder", fixture._temp.path().display());
        std::fs::create_dir_all(format!("{builder}/docs")).unwrap();
        std::fs::write(format!("{builder}/docs/UI.md"), "").unwrap();
        let attempt = hide_memory::Promotion {
            state: hide_memory::PromotionState::Running,
            revision: 1,
            attempt: 1,
            agent_id: Some("agent-9".into()),
            branch: Some("hide-memory-m1".into()),
            pr_number: None,
            repository: None,
            target_path: None,
            started_at_unix_ms: 1,
            updated_at_unix_ms: 1,
            failure: None,
            building_branch: None,
        };
        fixture
            .store()
            .set_promotion(&fixture.project_id, 1, &attempt)
            .unwrap();
        let building = |branch: &str, path: &str| {
            fixture.run_in_repository(
                MemoryCommand::Building {
                    number: 1,
                    branch: branch.to_owned(),
                    path: path.to_owned(),
                },
                false,
                Some(vec![
                    "main".to_owned(),
                    "hide-memory-m1".to_owned(),
                    "feat/build".to_owned(),
                ]),
                vec![("feat/build".to_owned(), builder.clone())],
            )
        };
        assert_eq!(
            building("hide-memory-m1", "docs/UI.md"),
            Err("memory_branch_is_the_attempts".into())
        );
        assert_eq!(
            building("feat/gone", "docs/UI.md"),
            Err("memory_branch_not_found".into())
        );
        assert_eq!(
            building("feat/build", "docs/missing.md"),
            Err("memory_building_path_not_found".into())
        );
        assert_eq!(
            building("feat/build", "docs/UI.md").unwrap(),
            json!({"card": "m1", "outcome": "building", "branch": "feat/build", "path": "docs/UI.md"})
        );
        let (_, stored) = fixture.store().promotions(&fixture.project_id).unwrap()[0].clone();
        assert_eq!(stored.state, hide_memory::PromotionState::Building);
        assert_eq!(stored.building_branch.as_deref(), Some("feat/build"));
        assert_eq!(stored.target_path.as_deref(), Some("docs/UI.md"));
        assert_eq!(
            stored.branch.as_deref(),
            Some("hide-memory-m1"),
            "the attempt keeps its own branch"
        );
        assert_eq!(
            building("feat/build", "docs/UI.md"),
            Err("memory_card_not_promoting".into()),
            "a building card is no longer the agent's to move"
        );

        // A branch without a checkout here is taken by name alone.
        fixture
            .store()
            .set_promotion(
                &fixture.project_id,
                1,
                &hide_memory::Promotion {
                    attempt: 2,
                    ..attempt.clone()
                },
            )
            .unwrap();
        assert_eq!(
            fixture
                .run_in_repository(
                    MemoryCommand::Building {
                        number: 1,
                        branch: "feat/elsewhere".to_owned(),
                        path: "docs/UI.md".to_owned(),
                    },
                    false,
                    Some(vec!["feat/elsewhere".to_owned()]),
                    Vec::new(),
                )
                .unwrap()["outcome"],
            "building"
        );

        fixture
            .store()
            .set_promotion(
                &fixture.project_id,
                1,
                &hide_memory::Promotion {
                    attempt: 3,
                    ..attempt.clone()
                },
            )
            .unwrap();
        assert_eq!(
            fixture
                .run(
                    MemoryCommand::Unbuilt {
                        number: 1,
                        reason: "백로그 출처 선택이 아직 없음".to_owned(),
                    },
                    false,
                )
                .unwrap(),
            json!({"card": "m1", "outcome": "unbuilt"})
        );
        let (_, stored) = fixture.store().promotions(&fixture.project_id).unwrap()[0].clone();
        assert_eq!(stored.state, hide_memory::PromotionState::Unbuilt);
        assert_eq!(
            stored.failure.as_deref(),
            Some("백로그 출처 선택이 아직 없음")
        );
        for command in [
            MemoryCommand::Unbuilt {
                number: 1,
                reason: " ".into(),
            },
            MemoryCommand::Building {
                number: 1,
                branch: "-x".into(),
                path: "docs/UI.md".into(),
            },
            MemoryCommand::Building {
                number: 1,
                branch: "feat/build".into(),
                path: ".git/config".into(),
            },
        ] {
            assert!(!command.valid(), "{command:?}");
        }
    }

    #[test]
    fn show_answers_the_card_with_its_files_and_counts_the_read_for_the_callers_session() {
        // B14, B27, D-18.
        let fixture = Fixture::new();

        let card = fixture
            .run(MemoryCommand::Show { number: 1 }, false)
            .unwrap();

        assert_eq!(card["card"], "m1");
        assert_eq!(card["body"], "Run tests with nextest before a merge");
        assert_eq!(card["state"], "active");
        assert_eq!(card["kind"], "rule");
        assert_eq!(card["sources"], 1);
        assert_eq!(card["cited_paths"], json!(["src/lib.rs"]));
        assert_eq!(
            card["evidence"],
            json!([]),
            "a card made with none rests on none"
        );
        let item = fixture
            .store()
            .item_by_number(&fixture.project_id, 1)
            .unwrap()
            .unwrap();
        assert_eq!((item.pull_count, item.provided_session_count), (1, 1));

        assert_eq!(
            fixture.run(MemoryCommand::Show { number: 9 }, false),
            Err("memory_card_not_found".to_owned())
        );
        let id = item.id;
        fixture.store().forget(&fixture.project_id, &id).unwrap();
        assert_eq!(
            fixture.run(MemoryCommand::Show { number: 1 }, false),
            Err("memory_card_forgotten".to_owned())
        );
    }

    #[test]
    fn search_lists_matching_cards_and_answers_none_without_failing() {
        // B15.
        let fixture = Fixture::new();
        let search = |words: &str| {
            fixture
                .run(
                    MemoryCommand::Search {
                        words: words.to_owned(),
                    },
                    false,
                )
                .unwrap()
        };

        assert_eq!(
            search("테스트를 돌려줘")["cards"],
            json!([{"card": "m1", "title": "Tests run with nextest", "kind": "rule"}])
        );
        assert_eq!(search("오늘 점심 메뉴")["cards"], json!([]));
    }

    #[test]
    fn add_writes_a_card_once_and_the_same_text_again_adds_nothing() {
        // B7, B8, D-11.
        let fixture = Fixture::new();
        fixture.backend.answers.lock().unwrap().push(json!({
            "candidates": [{
                "text": "Lint runs before tests.",
                "title": "Lint before tests",
                "aliases": ["lint"],
                "kind": "rule",
                "confidence": 0.8,
                "salience": 0.7,
                "source_offsets": [0],
                "cited_paths": [],
                "relation": "new",
                "target_id": null,
            }],
            "alias_backfill": [],
        }));
        let add = || {
            fixture.run(
                MemoryCommand::Add {
                    text: "Lint runs before tests".to_owned(),
                },
                true,
            )
        };

        let added = add().unwrap();
        assert_eq!(
            added["outcomes"],
            json!([{"outcome": "added", "card": "m2"}])
        );
        let card = fixture
            .store()
            .item_by_number(&fixture.project_id, 2)
            .unwrap()
            .unwrap();
        assert_eq!(
            card.human_source_count, 0,
            "add is never the operator's word"
        );

        assert_eq!(
            add().unwrap()["outcomes"],
            json!([{"outcome": "already_added", "card": "m2"}])
        );
        assert_eq!(fixture.calls(), 1);
        assert!(
            fixture
                .store()
                .item_by_number(&fixture.project_id, 3)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn forget_drops_a_card_undo_brings_it_back_and_a_forgotten_text_can_be_added_again() {
        // letter-4413: the operator tells the agent to forget a card.
        let fixture = Fixture::new();
        let run = |command| fixture.run(command, true);

        assert_eq!(
            run(MemoryCommand::Forget { number: 1 }).unwrap(),
            json!({"card": "m1", "outcome": "forgotten"})
        );
        assert_eq!(
            run(MemoryCommand::Forget { number: 1 }),
            Err("memory_card_forgotten".to_owned())
        );
        assert_eq!(
            run(MemoryCommand::Undo { number: 1 }).unwrap(),
            json!({"card": "m1", "outcome": "restored"})
        );
        assert_eq!(
            run(MemoryCommand::Show { number: 1 }).unwrap()["card"],
            "m1"
        );
        assert_eq!(
            run(MemoryCommand::Forget { number: 7 }),
            Err("memory_card_not_found".to_owned())
        );

        let answer = |title: &str| {
            json!({
                "candidates": [{
                    "text": "Lint runs before tests.",
                    "title": title,
                    "aliases": ["lint"],
                    "kind": "rule",
                    "confidence": 0.8,
                    "salience": 0.7,
                    "source_offsets": [0],
                    "cited_paths": [],
                    "relation": "new",
                    "target_id": null,
                }],
                "alias_backfill": [],
            })
        };
        let add = || {
            run(MemoryCommand::Add {
                text: "Lint runs before tests".to_owned(),
            })
        };
        fixture
            .backend
            .answers
            .lock()
            .unwrap()
            .push(answer("Lint first"));
        assert_eq!(
            add().unwrap()["outcomes"],
            json!([{"outcome": "added", "card": "m2"}])
        );
        run(MemoryCommand::Forget { number: 2 }).unwrap();

        // Every card of the first add is gone, so the same text is a new add.
        fixture
            .backend
            .answers
            .lock()
            .unwrap()
            .push(answer("Lint again"));
        assert_eq!(
            add().unwrap()["outcomes"],
            json!([{"outcome": "added", "card": "m3"}])
        );
        assert_eq!(
            add().unwrap()["outcomes"],
            json!([{"outcome": "already_added", "card": "m3"}])
        );
        assert_eq!(fixture.calls(), 2);

        // Undoing an add removes the card it made, and then there is nothing left.
        assert_eq!(
            run(MemoryCommand::Undo { number: 3 }).unwrap(),
            json!({"card": "m3", "outcome": "removed"})
        );
        assert_eq!(
            run(MemoryCommand::Undo { number: 3 }),
            Err("memory_nothing_to_undo".to_owned())
        );
    }

    #[test]
    fn a_failed_or_refused_add_writes_nothing() {
        // B8, B16.
        let fixture = Fixture::new();
        let add = |ai| {
            fixture.run(
                MemoryCommand::Add {
                    text: "Lint runs before tests".to_owned(),
                },
                ai,
            )
        };

        assert_eq!(add(false), Err("memory_ai_off".to_owned()));
        // The provider has no answer: nothing is written.
        assert_eq!(add(true), Err("memory_ai_failed".to_owned()));
        assert!(
            fixture
                .store()
                .item_by_number(&fixture.project_id, 2)
                .unwrap()
                .is_none()
        );

        fixture
            .store()
            .set_enabled(&fixture.project_id, false, true)
            .unwrap();
        for command in [
            MemoryCommand::Show { number: 1 },
            MemoryCommand::Search {
                words: "nextest".to_owned(),
            },
            MemoryCommand::Add {
                text: "Lint runs before tests".to_owned(),
            },
        ] {
            assert_eq!(fixture.run(command, true), Err("memory_off".to_owned()));
        }
        assert_eq!(fixture.calls(), 1);
    }

    #[test]
    fn a_parser_made_command_is_valid_and_an_oversized_add_is_not() {
        assert!(
            MemoryCommand::Add {
                text: "가".repeat(ADD_LIMIT_CHARS)
            }
            .valid()
        );
        assert!(
            !MemoryCommand::Add {
                text: "가".repeat(ADD_LIMIT_CHARS + 1)
            }
            .valid()
        );
        assert!(!MemoryCommand::Show { number: 0 }.valid());
        assert!(!MemoryCommand::Forget { number: 0 }.valid());
        for path in [
            "../outside.md",
            "/etc/hosts",
            "docs/./x.md",
            "",
            ".git/config",
            "sub/.GIT/hooks/x",
        ] {
            assert!(
                !MemoryCommand::Placed {
                    number: 1,
                    path: path.into()
                }
                .valid(),
                "{path}"
            );
        }
        assert!(
            MemoryCommand::Placed {
                number: 1,
                path: "docs/TESTING.md".into()
            }
            .valid()
        );
        assert_eq!(parse_card("m12"), Some(12));
        assert_eq!(parse_card("m0"), None);
    }
}
