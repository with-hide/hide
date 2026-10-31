//! A Memory promotion batch's branch (PRD hide-ai-branch-first-apply D-01,
//! D-13, B20, B21): its agent commits one card per commit on the batch's
//! branch and stops, the branch waits for the operator's decision as one
//! row of the 제안 tab, and the cards follow the branch: merged into the
//! base (by Hide, by a pull request or by hand) they are in the documents,
//! discarded they stay memories. The git side is `branch_decision`'s; the
//! tracker (`memory_promotion::track_promotions`) reads each waiting batch
//! here, off the lock, and the operator's decisions run here on a worker.

use super::branch_decision::{self, BranchPlace};
use super::memory_promotion::{PromotionStart, Retire};
use super::*;
use crate::node::NodeId;
use crate::suggestions::lifecycle::{AttemptPurpose, BranchFileSummary, BranchSummary};
use hide_memory::{BatchPurpose, MemoryStore, Promotion, PromotionState};
use hide_node_link::branch::BranchFacts;
use serde_json::json;

/// One batch whose branch the tracker reads: its agent stopped with the
/// cards running, or its branch is ready.
pub(super) struct BatchCheck {
    pub(super) node: NodeId,
    pub(super) project_id: String,
    pub(super) root: String,
    pub(super) branch: String,
    pub(super) base: Option<String>,
    pub(super) tip: Option<String>,
    pub(super) purpose: BatchPurpose,
    /// The batch's cards the tracker follows, as it read them.
    pub(super) cards: Vec<(u64, Promotion)>,
}

/// What a batch read did, for the tracker to carry on with.
#[derive(Default)]
pub(super) struct BatchesRead {
    pub(super) moved_projects: BTreeSet<String>,
    /// Each batch branch read, with what it holds.
    pub(super) facts: Vec<((String, String), BranchSummary)>,
    /// Batches that reached the base, whose agent and worktree retire as a
    /// merged pull request's do.
    pub(super) merged: Vec<(Retire, Vec<String>)>,
}

/// What the tab knows of a batch branch between tracker runs: what the
/// node read of it and what its agent was asked.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct BatchBranchFacts {
    pub(super) summary: Option<BranchSummary>,
    pub(super) purpose: Option<BatchPurpose>,
}

/// How a batch's machine is reached: its engine and its node.
pub(super) type BatchReach = (
    Arc<dyn crate::engine::Engine>,
    Arc<dyn crate::node_access::NodeLink>,
);

/// Reads each checked batch's branch on its machine and moves its cards
/// (D-08, B5, B13, B20): a stopped agent's branch with commits makes the
/// placed cards ready and returns the rest with their reason, one with none
/// merges the cards placed where their document already says them and
/// fails every card when none was placed (`바뀐 것 없이 끝났어요`); a ready
/// branch whose commit reached the base merged its cards, one gone unmerged
/// was discarded.
pub(super) fn read_batches(
    checks: Vec<BatchCheck>,
    reach: &HashMap<NodeId, BatchReach>,
    store: &mut MemoryStore,
    now: u64,
) -> BatchesRead {
    let mut read = BatchesRead::default();
    for check in checks {
        let Some((_, link)) = reach.get(&check.node) else {
            continue;
        };
        let Some(base) = check.base.clone() else {
            log_batch(&check, "batch.unread", "no_base");
            continue;
        };
        let place = BranchPlace {
            node: check.node.clone(),
            project_id: check.project_id.clone(),
            root: check.root.clone(),
            branch: check.branch.clone(),
            base,
        };
        let facts = match branch_decision::read_facts(link.as_ref(), &place, check.tip.as_deref()) {
            Ok(facts) => facts,
            // The node's words can name paths; the log keeps the code (B27).
            Err(_) => {
                crate::diagnostic!(json!({
                    "component": "memory", "kind": "promotion.batch_unread",
                    "project_id": check.project_id, "reason": "read_failed",
                }));
                continue;
            }
        };
        let ready = check
            .cards
            .iter()
            .any(|(_, promotion)| promotion.state == PromotionState::Ready);
        let moved = if ready {
            read_ready(&check, &place, facts, store, now, &mut read)
        } else {
            read_stopped(&check, &place, facts, store, now, &mut read)
        };
        match moved {
            Ok(true) => {
                read.moved_projects.insert(check.project_id.clone());
            }
            Ok(false) => {}
            Err(error) => crate::diagnostic!(json!({
                "component": "memory", "kind": "promotion.move_failed",
                "project_id": check.project_id, "message": error,
            })),
        }
    }
    read
}

fn read_stopped(
    check: &BatchCheck,
    place: &BranchPlace,
    facts: BranchFacts,
    store: &mut MemoryStore,
    now: u64,
    read: &mut BatchesRead,
) -> Result<bool, String> {
    let mut moved = false;
    let summary = summary(&place.base, &facts);
    let placed = check
        .cards
        .iter()
        .any(|(_, promotion)| promotion.target_path.is_some());
    // A placed card on a branch with nothing of its own is one the agent
    // found its document already says: it is in the documents now, with no
    // branch for the operator to decide on.
    if placed && summary.is_none() && facts.exists {
        return settle_documented(check, place, store, now, read);
    }
    let Some(summary) = summary.filter(|_| placed) else {
        for (number, promotion) in &check.cards {
            if promotion.state == PromotionState::Running {
                moved |= store
                    .move_promotion(
                        &check.project_id,
                        *number,
                        promotion,
                        PromotionState::Failed,
                        None,
                        now,
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
        log_batch(check, "promotion.batch_ended", "no_change");
        return Ok(moved);
    };
    for (number, promotion) in &check.cards {
        if promotion.state != PromotionState::Running {
            continue;
        }
        moved |= if promotion.target_path.is_some() {
            store
                .move_promotion(
                    &check.project_id,
                    *number,
                    promotion,
                    PromotionState::Ready,
                    None,
                    now,
                )
                .map_err(|error| error.to_string())?
        } else {
            store
                .skip_promotion(
                    &check.project_id,
                    *number,
                    promotion,
                    "The card was not processed before the task ended",
                    now,
                    None,
                )
                .map_err(|error| error.to_string())?
        };
    }
    store
        .set_promotion_batch_branch(
            &check.project_id,
            &check.branch,
            BatchPurpose::Branch,
            Some(&summary.tip),
        )
        .map_err(|error| error.to_string())?;
    log_batch(check, "promotion.batch_ready", "ready");
    read.facts
        .push(((check.project_id.clone(), check.branch.clone()), summary));
    Ok(moved)
}

/// Ends a stopped batch whose agent placed cards without a commit: each
/// placed card is merged where its document already says it, the rest are
/// returned with their reason, and the batch's agent and worktree retire as
/// a merged batch's do.
fn settle_documented(
    check: &BatchCheck,
    place: &BranchPlace,
    store: &mut MemoryStore,
    now: u64,
    read: &mut BatchesRead,
) -> Result<bool, String> {
    let mut moved = false;
    for (number, promotion) in &check.cards {
        if promotion.state != PromotionState::Running {
            continue;
        }
        moved |= if promotion.target_path.is_some() {
            store
                .move_promotion(
                    &check.project_id,
                    *number,
                    promotion,
                    PromotionState::Merged,
                    None,
                    now,
                )
                .map_err(|error| error.to_string())?
        } else {
            store
                .skip_promotion(
                    &check.project_id,
                    *number,
                    promotion,
                    "The card was not processed before the task ended",
                    now,
                    None,
                )
                .map_err(|error| error.to_string())?
        };
    }
    log_batch(check, "promotion.batch_ended", "already_documented");
    read.merged.push((
        retire_of(place),
        check
            .cards
            .iter()
            .filter_map(|(_, promotion)| promotion.agent_id.clone())
            .collect(),
    ));
    Ok(moved)
}

fn read_ready(
    check: &BatchCheck,
    place: &BranchPlace,
    facts: BranchFacts,
    store: &mut MemoryStore,
    now: u64,
    read: &mut BatchesRead,
) -> Result<bool, String> {
    let settle = |state: PromotionState, store: &mut MemoryStore| -> Result<bool, String> {
        let mut moved = false;
        for (number, promotion) in &check.cards {
            if promotion.state == PromotionState::Ready {
                moved |= store
                    .move_promotion(&check.project_id, *number, promotion, state, None, now)
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(moved)
    };
    if facts.merged == Some(true) {
        let moved = settle(PromotionState::Merged, store)?;
        log_batch(check, "promotion.batch_merged", "merged");
        read.merged.push((
            retire_of(place),
            check
                .cards
                .iter()
                .filter_map(|(_, promotion)| promotion.agent_id.clone())
                .collect(),
        ));
        return Ok(moved);
    }
    if !facts.exists {
        if facts.merged == Some(false) {
            log_batch(check, "promotion.batch_discarded", "deleted");
            return settle(PromotionState::Dismissed, store);
        }
        return Ok(false);
    }
    if let Some(summary) = summary(&place.base, &facts) {
        if check.tip.as_deref() != Some(summary.tip.as_str()) {
            store
                .set_promotion_batch_branch(
                    &check.project_id,
                    &check.branch,
                    check.purpose,
                    Some(&summary.tip),
                )
                .map_err(|error| error.to_string())?;
        }
        read.facts
            .push(((check.project_id.clone(), check.branch.clone()), summary));
    }
    Ok(false)
}

/// What a branch holds, when it holds anything.
fn summary(base: &str, facts: &BranchFacts) -> Option<BranchSummary> {
    let tip = facts
        .tip
        .clone()
        .filter(|_| facts.exists && facts.ahead > 0)?;
    Some(BranchSummary {
        base: base.to_owned(),
        tip,
        ahead: facts.ahead,
        files: facts
            .files
            .iter()
            .map(|file| BranchFileSummary {
                path: file.path.clone(),
                added: file.added,
                removed: file.removed,
                binary: file.binary,
            })
            .collect(),
        files_total: facts.files_total,
        added: facts.added,
        removed: facts.removed,
    })
}

fn retire_of(place: &BranchPlace) -> Retire {
    Retire {
        node: place.node.clone(),
        project_id: place.project_id.clone(),
        root: place.root.clone(),
        branch: place.branch.clone(),
    }
}

fn log_batch(check: &BatchCheck, kind: &str, outcome: &str) {
    crate::diagnostic!(json!({
        "component": "memory", "kind": kind,
        "project_id": check.project_id,
        "cards": check.cards.iter().map(|(number, _)| hide_memory::card_name(*number)).collect::<Vec<_>>(),
        "outcome": outcome,
    }));
}

/// The operator's decision on a ready batch, pressed on its row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BatchDecision {
    /// `main에 합치기`, without GitHub.
    Merge,
    /// `{base}에 바로 올리기`, a merge pushed to GitHub's base.
    MergePush,
    OpenPr,
    Discard,
    Rebase,
    /// `내 커밋을 브랜치로 옮기고 main 맞추기`, after a push found the base
    /// apart from GitHub's.
    SyncBase,
}

impl Runtime {
    /// The ready batch card `number` is in, in the opened Project: its
    /// branch and every ready card on it, with the Project's machine and
    /// folder.
    fn ready_batch(
        &self,
        workspace_id: &str,
        number: u64,
    ) -> Option<(NodeId, String, String, Vec<u64>)> {
        let memory = self
            .snapshot
            .memory
            .as_ref()?
            .project
            .as_ref()
            .filter(|memory| memory.workspace_id == workspace_id)?;
        let branch = memory
            .rows
            .iter()
            .find(|row| row.number == number)?
            .promotion
            .as_ref()
            .filter(|promotion| promotion.state == crate::model::MemoryPromotionState::Ready)?
            .branch
            .clone()?;
        let cards = memory
            .rows
            .iter()
            .filter(|row| {
                row.promotion.as_ref().is_some_and(|promotion| {
                    promotion.state == crate::model::MemoryPromotionState::Ready
                        && promotion.branch.as_deref() == Some(branch.as_str())
                })
            })
            .map(|row| row.number)
            .collect();
        let (node, root) = self.project_memory_path()?;
        Some((node.clone(), root.to_owned(), branch, cards))
    }

    /// The operator's `main에 합치기`, `PR 올리기`, `버리기` or `다시 맞추기` on
    /// a ready batch's row (PRD hide-ai-branch-first-apply D-05 to D-07,
    /// B6-B12, B20, B21). A merge or a discard runs on the Project's machine
    /// off the lock and moves every card of the batch; `PR 올리기` and `다시
    /// 맞추기` start an agent on the same branch, its old agent ended first.
    /// A merge the Project's GitHub does not allow is refused
    /// (`merge_refused`).
    pub(super) fn decide_memory_batch(
        &mut self,
        workspace_id: &str,
        number: u64,
        decision: BatchDecision,
    ) -> bool {
        self.decide_batch_as(workspace_id, number, decision, true)
    }

    /// A decision on a ready batch; with `remake`, a merge the base conflicts
    /// with starts `다시 맞추기` and is pressed again once the remade batch
    /// is ready. The merge pressed again after a remake has no `remake`: a
    /// second refusal stops for the operator.
    fn decide_batch_as(
        &mut self,
        workspace_id: &str,
        number: u64,
        decision: BatchDecision,
        remake: bool,
    ) -> bool {
        let refused = |reason: &str| {
            crate::diagnostic!(json!({
                "component": "memory", "kind": "promotion.decision_refused",
                "card": hide_memory::card_name(number), "reason": reason,
            }));
            false
        };
        let Some((node, root, branch, cards)) = self.ready_batch(workspace_id, number) else {
            return refused("not_ready");
        };
        let Some(project_id) = self.project_memory.project_id.clone() else {
            return refused("project_unavailable");
        };
        let key = (project_id.clone(), branch.clone());
        if self.project_memory.promotions.deciding.contains_key(&key) {
            return refused("deciding");
        }
        let Ok(link) = self.node_link(node.as_str()) else {
            return refused("node_unavailable");
        };
        let engine = self.delivery_engine(node.as_str());
        let facts = self.project_memory.promotions.branches.get(&key).cloned();
        let base = facts
            .as_ref()
            .and_then(|facts| facts.summary.as_ref().map(|summary| summary.base.clone()))
            .or_else(|| self.project_base(&node, &root));
        let Some(base) = base else {
            return refused("no_base");
        };
        if matches!(decision, BatchDecision::Merge | BatchDecision::MergePush)
            && let Some(reason) =
                self.merge_refused(&node, &root, &base, decision == BatchDecision::MergePush)
        {
            return refused(reason);
        }
        let start = match decision {
            BatchDecision::OpenPr | BatchDecision::Rebase => {
                match self.promotion_start(&node, &root) {
                    Ok(mut start) => {
                        start.purpose = if decision == BatchDecision::OpenPr {
                            BatchPurpose::Pr
                        } else {
                            BatchPurpose::Rebase
                        };
                        start.base = Some(base.clone());
                        Some(start)
                    }
                    Err(reason) => return refused(&reason),
                }
            }
            // A merge refused because the branch no longer applies on its
            // base starts `다시 맞추기` itself, so it carries the start a
            // rebase needs; without one the refusal and its button stay.
            BatchDecision::Merge | BatchDecision::MergePush if remake => {
                self.promotion_start(&node, &root).ok().map(|mut start| {
                    start.purpose = BatchPurpose::Rebase;
                    start.base = Some(base.clone());
                    start
                })
            }
            BatchDecision::Merge
            | BatchDecision::MergePush
            | BatchDecision::Discard
            | BatchDecision::SyncBase => None,
        };
        let own = self.memory_batch_agents(&project_id, &cards);
        let place = BranchPlace {
            node,
            project_id: project_id.clone(),
            root,
            branch,
            base,
        };
        let tip = facts.and_then(|facts| facts.summary.map(|summary| summary.tip));
        let database = self.memory_database_path();
        self.project_memory
            .promotions
            .deciding
            .insert(key.clone(), decision);
        let carried = key.clone();
        let workspace_id = workspace_id.to_owned();
        let pushed = decision == BatchDecision::MergePush;
        let based = (place.node.clone(), place.root.clone(), place.base.clone());
        let day = branch_decision::sync_day(super::memory::now_ms());
        let spawned = self.spawn_issue_worker(
            "memory-decide",
            move || {
                decide(
                    decision,
                    engine,
                    link,
                    &place,
                    &own,
                    tip.as_deref(),
                    &database,
                    &cards,
                    start,
                    &day,
                )
            },
            move |runtime, (unseen, learned, remade)| {
                runtime.project_memory.promotions.deciding.remove(&carried);
                if remade {
                    runtime.project_memory.promotions.finish.insert(
                        carried.clone(),
                        BatchFinish {
                            decision,
                            workspace_id,
                            running: false,
                        },
                    );
                }
                let (node, root, base) = based;
                // Whatever GitHub answered the push, its rules are read again
                // before the button is offered again.
                if pushed {
                    runtime.forget_base_push(&node, &root, &base);
                }
                if let Some(learned) = learned {
                    runtime.note_base(&node, &root, &base, learned);
                }
                if let Some((key, start)) = unseen {
                    runtime.project_memory.promotions.add_unseen(key, start);
                }
                runtime.note_memory_change(&carried.0, super::memory_analysis::MemoryChange::Other);
                runtime.promotions_moved();
                true
            },
        );
        if let Err(reason) = spawned {
            self.project_memory.promotions.deciding.remove(&key);
            return refused(&reason);
        }
        self.compose_suggestions()
    }

    /// The panes of the live agents a batch's cards recorded.
    fn memory_batch_agents(&self, project_id: &str, cards: &[u64]) -> Vec<String> {
        let Some(rows) = self
            .snapshot
            .memory
            .as_ref()
            .and_then(|memory| memory.project.as_ref())
        else {
            return Vec::new();
        };
        let _ = project_id;
        rows.rows
            .iter()
            .filter(|row| cards.contains(&row.number))
            .filter_map(|row| row.promotion.as_ref()?.agent.as_ref()?.agent_id.clone())
            .filter_map(|id| self.live_agent_pane(&id))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// The ready batches of the Project at `root` on `node`, by branch, and
    /// the row Hide is merging or syncing the base of now, with the phase it
    /// shows, if any (PRD hide-ai-branch-first-apply).
    pub(super) fn memory_branch_overlay(
        &self,
        node: &NodeId,
        root: &str,
    ) -> Vec<(
        String,
        Option<(String, crate::model::SuggestionAttemptPhase)>,
    )> {
        if self
            .project_memory_path()
            .is_none_or(|(memory_node, memory_root)| memory_node != node || memory_root != root)
        {
            return Vec::new();
        }
        let Some(project_id) = self.project_memory.project_id.clone() else {
            return Vec::new();
        };
        let Some(memory) = self
            .snapshot
            .memory
            .as_ref()
            .and_then(|memory| memory.project.as_ref())
        else {
            return Vec::new();
        };
        let mut branches = BTreeMap::new();
        for row in memory.rows.iter() {
            let Some(promotion) = row.promotion.as_ref() else {
                continue;
            };
            let Some(branch) = promotion.branch.clone() else {
                continue;
            };
            if promotion.state != crate::model::MemoryPromotionState::Ready {
                continue;
            }
            let shown = self
                .project_memory
                .promotions
                .deciding
                .get(&(project_id.clone(), branch.clone()))
                .and_then(|decision| match decision {
                    BatchDecision::Merge | BatchDecision::MergePush => {
                        Some(crate::model::SuggestionAttemptPhase::Merging)
                    }
                    BatchDecision::SyncBase => Some(crate::model::SuggestionAttemptPhase::Syncing),
                    BatchDecision::OpenPr | BatchDecision::Discard | BatchDecision::Rebase => None,
                });
            let entry = branches.entry(branch).or_insert(None);
            if let Some(phase) = shown
                && entry.is_none()
            {
                *entry = Some((format!("m{}", row.number), phase));
            }
        }
        branches.into_iter().collect()
    }

    /// A batch's branch facts the 제안 row draws, by Project and branch.
    pub(super) fn memory_batch_facts(&self, branch: &str) -> Option<&BatchBranchFacts> {
        let project_id = self.project_memory.project_id.clone()?;
        self.project_memory
            .promotions
            .branches
            .get(&(project_id, branch.to_owned()))
    }
}

/// A merge a remake carries out (operator decision 2026-10-31): the decision
/// the operator pressed, the Project it was pressed in, and whether the
/// remade batch was seen running yet, so the batch's old ready rows are
/// never taken for the remake's.
#[derive(Clone, Debug)]
pub(super) struct BatchFinish {
    decision: BatchDecision,
    workspace_id: String,
    running: bool,
}

impl Runtime {
    /// Presses again the merge a remake carries out once the Memory read
    /// shows the remade batch ready; a batch that ended any other way drops
    /// it, and its row offers what its state offers.
    pub(super) fn finish_remade_batches(&mut self) {
        if self.project_memory.promotions.finish.is_empty() {
            return;
        }
        let Some(project_id) = self.project_memory.project_id.clone() else {
            return;
        };
        let Some(memory) = self
            .snapshot
            .memory
            .as_ref()
            .and_then(|memory| memory.project.as_ref())
            .cloned()
        else {
            return;
        };
        let mut due = Vec::new();
        self.project_memory
            .promotions
            .finish
            .retain(|(project, branch), finish| {
                if *project != project_id || finish.workspace_id != memory.workspace_id {
                    return true;
                }
                let rows = memory
                    .rows
                    .iter()
                    .filter_map(|row| {
                        let promotion = row.promotion.as_ref()?;
                        (promotion.branch.as_deref() == Some(branch.as_str()))
                            .then_some((row.number, promotion.state))
                    })
                    .collect::<Vec<_>>();
                use crate::model::MemoryPromotionState as State;
                if rows.iter().any(|(_, state)| *state == State::Running) {
                    finish.running = true;
                    return true;
                }
                if !finish.running {
                    return rows.iter().any(|(_, state)| *state == State::Ready);
                }
                if let Some((number, _)) = rows.iter().find(|(_, state)| *state == State::Ready) {
                    due.push((finish.workspace_id.clone(), *number, finish.decision));
                }
                false
            });
        for (workspace_id, number, decision) in due {
            crate::diagnostic!(json!({
                "component": "memory", "kind": "promotion.finish",
                "card": hide_memory::card_name(number),
            }));
            self.decide_batch_as(&workspace_id, number, decision, false);
        }
    }
}

/// The purpose a batch's agent was asked, as a 제안 row words it.
pub(super) fn attempt_purpose(purpose: Option<BatchPurpose>) -> AttemptPurpose {
    match purpose {
        Some(BatchPurpose::Branch) => AttemptPurpose::Branch,
        Some(BatchPurpose::Rebase) => AttemptPurpose::Rebase,
        Some(BatchPurpose::Pr) | None => AttemptPurpose::Pr,
    }
}

/// A restart whose agent has not shown yet, for the tracker to ask again.
type Unseen = Option<((String, u64, u32), PromotionStart)>;

/// Carries out a decision off the lock; answers a restart whose agent has
/// not shown yet, and what it learned of the base.
#[allow(clippy::too_many_arguments)]
fn decide(
    decision: BatchDecision,
    engine: Option<Arc<dyn crate::engine::Engine>>,
    link: Arc<dyn crate::node_access::NodeLink>,
    place: &BranchPlace,
    own: &[String],
    tip: Option<&str>,
    database: &Path,
    cards: &[u64],
    start: Option<PromotionStart>,
    day: &str,
) -> (Unseen, Option<branch_decision::BaseChange>, bool) {
    let mut learned = None;
    let mut remade = false;
    let unseen = decide_batch(
        decision,
        engine,
        link,
        place,
        own,
        tip,
        database,
        cards,
        start,
        day,
        &mut learned,
        &mut remade,
    );
    (unseen, learned, remade)
}

#[allow(clippy::too_many_arguments)]
fn decide_batch(
    decision: BatchDecision,
    engine: Option<Arc<dyn crate::engine::Engine>>,
    link: Arc<dyn crate::node_access::NodeLink>,
    place: &BranchPlace,
    own: &[String],
    tip: Option<&str>,
    database: &Path,
    cards: &[u64],
    start: Option<PromotionStart>,
    day: &str,
    learned: &mut Option<branch_decision::BaseChange>,
    remade: &mut bool,
) -> Unseen {
    let now = super::memory::now_ms();
    let mut store = match super::memory::open_memory_store(database) {
        Ok(store) => store,
        Err(error) => {
            crate::diagnostic!(json!({
                "component": "memory", "kind": "promotion.store_unavailable",
                "message": error.to_string(),
            }));
            return None;
        }
    };
    let ready = match store.promotions(&place.project_id) {
        Ok(promotions) => promotions
            .into_iter()
            .filter(|(number, promotion)| {
                cards.contains(number)
                    && promotion.state == PromotionState::Ready
                    && promotion.branch.as_deref() == Some(place.branch.as_str())
            })
            .collect::<Vec<_>>(),
        Err(error) => {
            crate::diagnostic!(json!({
                "component": "memory", "kind": "promotion.store_unavailable",
                "message": error.to_string(),
            }));
            return None;
        }
    };
    let subject = cards
        .first()
        .map_or_else(String::new, |number| hide_memory::card_name(*number));
    let move_all = |store: &mut MemoryStore, state: PromotionState| {
        for (number, promotion) in &ready {
            if let Err(error) =
                store.move_promotion(&place.project_id, *number, promotion, state, None, now)
            {
                crate::diagnostic!(json!({
                    "component": "memory", "kind": "promotion.move_failed",
                    "project_id": place.project_id, "card": hide_memory::card_name(*number),
                    "message": error.to_string(),
                }));
            }
        }
    };
    let refuse_all = |store: &mut MemoryStore, code: &str| {
        for (number, promotion) in &ready {
            let _ = store.set_ready_failure(&place.project_id, *number, promotion, Some(code), now);
        }
    };
    match decision {
        BatchDecision::Merge | BatchDecision::MergePush => {
            let push = decision == BatchDecision::MergePush;
            let kind = if push { "merge_push" } else { "merge" };
            let (landing, base) =
                branch_decision::land(engine.as_deref(), link.as_ref(), place, own, tip, push);
            *learned = base;
            match landing {
                branch_decision::Landing::Merged(_) => {
                    branch_decision::log(place, &subject, kind, "merged");
                    move_all(&mut store, PromotionState::Merged);
                }
                branch_decision::Landing::BranchMissing => {
                    branch_decision::log(place, &subject, kind, "branch_missing");
                    move_all(&mut store, PromotionState::Dismissed);
                }
                branch_decision::Landing::Refused(code) => {
                    branch_decision::log(place, &subject, kind, code);
                    if branch_decision::remakes(code)
                        && let Some(start) = start
                        && let Ok(unseen) = restart_batch(
                            &mut store,
                            engine.as_deref(),
                            link.as_ref(),
                            place,
                            own,
                            &ready,
                            start,
                            now,
                        )
                    {
                        *remade = true;
                        return unseen;
                    }
                    refuse_all(&mut store, code);
                }
            }
            None
        }
        BatchDecision::SyncBase => {
            let (synced, base) = branch_decision::sync(link.as_ref(), place, day);
            *learned = base;
            let code = synced.err();
            branch_decision::log(place, &subject, "sync_base", code.unwrap_or("synced"));
            match code {
                Some(code) => refuse_all(&mut store, code),
                None => {
                    for (number, promotion) in &ready {
                        let _ = store.set_ready_failure(
                            &place.project_id,
                            *number,
                            promotion,
                            None,
                            now,
                        );
                    }
                }
            }
            None
        }
        BatchDecision::Discard => {
            let discarded = match engine.as_deref() {
                Some(engine) => branch_decision::discard(engine, link.as_ref(), place, own, tip),
                None => Err("herdr_unavailable".to_owned()),
            };
            match discarded {
                Ok(()) => {
                    branch_decision::log(place, &subject, "discard", "discarded");
                    move_all(&mut store, PromotionState::Dismissed);
                }
                Err(_) => {
                    branch_decision::log(place, &subject, "discard", "discard_failed");
                    refuse_all(&mut store, "discard_failed");
                }
            }
            None
        }
        BatchDecision::OpenPr | BatchDecision::Rebase => restart_batch(
            &mut store,
            engine.as_deref(),
            link.as_ref(),
            place,
            own,
            &ready,
            start?,
            now,
        )
        .unwrap_or(None),
    }
}

/// Starts an agent on a ready batch's branch for the purpose `start` names
/// (`PR 올리기`, `다시 맞추기`), its old agents ended first; `Err` when no
/// card of the batch restarted.
#[allow(clippy::too_many_arguments)]
fn restart_batch(
    store: &mut MemoryStore,
    engine: Option<&dyn crate::engine::Engine>,
    link: &dyn crate::node_access::NodeLink,
    place: &BranchPlace,
    own: &[String],
    ready: &[(u64, Promotion)],
    mut start: PromotionStart,
    now: u64,
) -> Result<Unseen, ()> {
    if let Some(engine) = engine {
        for pane in own {
            if branch_decision::end_agent(engine, link, pane).is_err() {
                crate::diagnostic!(json!({
                    "component": "memory", "kind": "promotion.previous_agent_kept",
                    "project_id": place.project_id,
                }));
            }
        }
    }
    if let Err(error) =
        store.set_promotion_batch_branch(&place.project_id, &place.branch, start.purpose, None)
    {
        crate::diagnostic!(json!({
            "component": "memory", "kind": "promotion.move_failed",
            "project_id": place.project_id, "message": error.to_string(),
        }));
        return Err(());
    }
    let mut restarted = Vec::new();
    for (number, promotion) in ready {
        match store.restart_promotion(&place.project_id, *number, promotion, now) {
            Ok(true) => restarted.push(*number),
            Ok(false) => {}
            Err(error) => crate::diagnostic!(json!({
                "component": "memory", "kind": "promotion.move_failed",
                "project_id": place.project_id, "card": hide_memory::card_name(*number),
                "message": error.to_string(),
            })),
        }
    }
    if restarted.is_empty() {
        return Err(());
    }
    start.numbers = restarted;
    Ok(super::memory_promotion::spawn_restarted(
        store,
        &place.project_id,
        start,
    ))
}

#[cfg(test)]
mod tests {
    use super::super::memory_promotion::tests::{running, seed};
    use super::*;
    use std::path::Path;

    fn check(project: &str, cards: Vec<(u64, Promotion)>) -> BatchCheck {
        BatchCheck {
            node: NodeId::parse("test-node").unwrap(),
            project_id: project.into(),
            root: format!("/checkouts/{project}"),
            branch: "hide-memory-m1-x".into(),
            base: Some("main".into()),
            tip: None,
            purpose: BatchPurpose::Branch,
            cards,
        }
    }

    /// A branch that exists and holds no commit of its own.
    fn empty_branch() -> BranchFacts {
        BranchFacts {
            exists: true,
            tip: Some("abc123".into()),
            ahead: 0,
            merged: Some(false),
            files: Vec::new(),
            files_total: 0,
            added: 0,
            removed: 0,
        }
    }

    #[test]
    fn a_card_placed_without_a_commit_is_merged_where_its_document_already_says_it() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = MemoryStore::open(&folder.path().join("memory.sqlite3")).unwrap();
        for project in ["p", "q"] {
            store
                .ensure_project(
                    project,
                    Path::new(&format!("/checkouts/{project}")),
                    "test-node",
                )
                .unwrap();
            seed(&mut store, project, "Devices lists ssh config Hosts");
        }
        let placed = Promotion {
            target_path: Some("docs/UI_BEHAVIOR.md".into()),
            ..running(Some("agent-1"), 1_000)
        };
        store.set_promotion("p", 1, &placed).unwrap();
        let unplaced = running(Some("agent-2"), 1_000);
        store.set_promotion("q", 1, &unplaced).unwrap();

        let documented = check("p", vec![(1, placed)]);
        let place = |check: &BatchCheck| BranchPlace {
            node: check.node.clone(),
            project_id: check.project_id.clone(),
            root: check.root.clone(),
            branch: check.branch.clone(),
            base: "main".into(),
        };
        let mut read = BatchesRead::default();
        let moved = read_stopped(
            &documented,
            &place(&documented),
            empty_branch(),
            &mut store,
            2_000,
            &mut read,
        )
        .unwrap();
        assert!(moved);
        let (_, promotion) = store.promotions("p").unwrap().remove(0);
        assert_eq!(promotion.state, PromotionState::Merged);
        assert_eq!(
            promotion.target_path.as_deref(),
            Some("docs/UI_BEHAVIOR.md")
        );
        // Nothing for the operator to decide; its agent and worktree retire.
        assert!(read.facts.is_empty());
        assert_eq!(read.merged.len(), 1);
        assert_eq!(read.merged[0].1, ["agent-1"]);

        // An agent that ended with nothing placed and nothing committed
        // still changed nothing.
        let nothing = check("q", vec![(1, unplaced)]);
        let mut read = BatchesRead::default();
        read_stopped(
            &nothing,
            &place(&nothing),
            empty_branch(),
            &mut store,
            2_000,
            &mut read,
        )
        .unwrap();
        let (_, promotion) = store.promotions("q").unwrap().remove(0);
        assert_eq!(promotion.state, PromotionState::Failed);
        assert!(read.merged.is_empty());
    }
}
