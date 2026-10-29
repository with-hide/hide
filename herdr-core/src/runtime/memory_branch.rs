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
use hide_node_link::branch::{BranchFacts, MergeOutcome};
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

/// Reads each checked batch's branch on its machine and moves its cards
/// (D-08, B5, B13, B20): a stopped agent's branch with commits makes the
/// placed cards ready and returns the rest with their reason, one with none
/// fails every card (`바뀐 것 없이 끝났어요`); a ready branch whose commit
/// reached the base merged its cards, one gone unmerged was discarded.
pub(super) fn read_batches(
    checks: Vec<BatchCheck>,
    reach: &HashMap<NodeId, (Arc<dyn hide_herdr_client::ApiConnector>, Arc<dyn crate::node_access::NodeLink>)>,
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
    let Some(summary) = summary.filter(|_| placed) else {
        for (number, promotion) in &check.cards {
            if promotion.state == PromotionState::Running {
                moved |= store
                    .move_promotion(&check.project_id, *number, promotion, PromotionState::Failed, None, now)
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
                .move_promotion(&check.project_id, *number, promotion, PromotionState::Ready, None, now)
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
        .set_promotion_batch_branch(&check.project_id, &check.branch, BatchPurpose::Branch, Some(&summary.tip))
        .map_err(|error| error.to_string())?;
    log_batch(check, "promotion.batch_ready", "ready");
    read.facts
        .push(((check.project_id.clone(), check.branch.clone()), summary));
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
                .set_promotion_batch_branch(&check.project_id, &check.branch, check.purpose, Some(&summary.tip))
                .map_err(|error| error.to_string())?;
        }
        read.facts
            .push(((check.project_id.clone(), check.branch.clone()), summary));
    }
    Ok(false)
}

/// What a branch holds, when it holds anything.
fn summary(base: &str, facts: &BranchFacts) -> Option<BranchSummary> {
    let tip = facts.tip.clone().filter(|_| facts.exists && facts.ahead > 0)?;
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
    Merge,
    OpenPr,
    Discard,
    Rebase,
}

impl Runtime {
    /// The ready batch card `number` is in, in the opened Project: its
    /// branch and every ready card on it, with the Project's machine and
    /// folder.
    fn ready_batch(&self, workspace_id: &str, number: u64) -> Option<(NodeId, String, String, Vec<u64>)> {
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
    pub(super) fn decide_memory_batch(
        &mut self,
        workspace_id: &str,
        number: u64,
        decision: BatchDecision,
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
        let connector = self.delivery_connector(node.as_str());
        let facts = self.project_memory.promotions.branches.get(&key).cloned();
        let base = facts
            .as_ref()
            .and_then(|facts| facts.summary.as_ref().map(|summary| summary.base.clone()))
            .or_else(|| self.project_base(&node, &root));
        let Some(base) = base else {
            return refused("no_base");
        };
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
            BatchDecision::Merge | BatchDecision::Discard => None,
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
        let spawned = self.spawn_issue_worker(
            "memory-decide",
            move || {
                decide(
                    decision,
                    connector,
                    link,
                    &place,
                    &own,
                    tip.as_deref(),
                    &database,
                    &cards,
                    start,
                )
            },
            move |runtime, unseen| {
                runtime.project_memory.promotions.deciding.remove(&carried);
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
    /// the row Hide is merging now, if any (PRD hide-ai-branch-first-apply).
    pub(super) fn memory_branch_overlay(
        &self,
        node: &NodeId,
        root: &str,
    ) -> Vec<(String, Option<String>)> {
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
            let merging = self
                .project_memory
                .promotions
                .deciding
                .get(&(project_id.clone(), branch.clone()))
                == Some(&BatchDecision::Merge);
            let entry = branches.entry(branch).or_insert(None);
            if merging && entry.is_none() {
                *entry = Some(format!("m{}", row.number));
            }
        }
        branches.into_iter().collect()
    }

    /// The ready Memory batches waiting for the operator in the Project at
    /// `root` on `node` (D-17).
    pub(super) fn memory_branches_waiting(&self, node: &NodeId, root: &str) -> usize {
        self.memory_branch_overlay(node, root).len()
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

/// The purpose a batch's agent was asked, as a 제안 row words it.
pub(super) fn attempt_purpose(purpose: Option<BatchPurpose>) -> AttemptPurpose {
    match purpose {
        Some(BatchPurpose::Branch) => AttemptPurpose::Branch,
        Some(BatchPurpose::Rebase) => AttemptPurpose::Rebase,
        Some(BatchPurpose::Pr) | None => AttemptPurpose::Pr,
    }
}

/// Carries out a decision off the lock; answers a restart whose agent has
/// not shown yet, for the tracker to ask again.
#[allow(clippy::too_many_arguments)]
fn decide(
    decision: BatchDecision,
    connector: Option<Arc<dyn hide_herdr_client::ApiConnector>>,
    link: Arc<dyn crate::node_access::NodeLink>,
    place: &BranchPlace,
    own: &[String],
    tip: Option<&str>,
    database: &Path,
    cards: &[u64],
    start: Option<PromotionStart>,
) -> Option<((String, u64, u32), PromotionStart)> {
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
        BatchDecision::Merge => {
            match branch_decision::merge(connector.as_deref(), link.as_ref(), place, own) {
                Ok(MergeOutcome::Merged { .. } | MergeOutcome::AlreadyMerged { .. }) => {
                    branch_decision::log(place, &subject, "merge", "merged");
                    move_all(&mut store, PromotionState::Merged);
                }
                Ok(MergeOutcome::BranchMissing) => {
                    branch_decision::log(place, &subject, "merge", "branch_missing");
                    move_all(&mut store, PromotionState::Dismissed);
                }
                Ok(outcome) => {
                    let code = branch_decision::refusal_code(&outcome).unwrap_or("merge_failed");
                    branch_decision::log(place, &subject, "merge", code);
                    refuse_all(&mut store, code);
                }
                Err(_) => {
                    branch_decision::log(place, &subject, "merge", "merge_failed");
                    refuse_all(&mut store, "merge_failed");
                }
            }
            None
        }
        BatchDecision::Discard => {
            let discarded = match connector.as_deref() {
                Some(connector) => branch_decision::discard(connector, link.as_ref(), place, own, tip),
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
        BatchDecision::OpenPr | BatchDecision::Rebase => {
            let mut start = start?;
            if let Some(connector) = connector.as_deref() {
                for pane in own {
                    if branch_decision::end_agent(connector, link.as_ref(), pane).is_err() {
                        crate::diagnostic!(json!({
                            "component": "memory", "kind": "promotion.previous_agent_kept",
                            "project_id": place.project_id,
                        }));
                    }
                }
            }
            if let Err(error) = store.set_promotion_batch_branch(
                &place.project_id,
                &place.branch,
                start.purpose,
                None,
            ) {
                crate::diagnostic!(json!({
                    "component": "memory", "kind": "promotion.move_failed",
                    "project_id": place.project_id, "message": error.to_string(),
                }));
                return None;
            }
            let mut restarted = Vec::new();
            for (number, promotion) in &ready {
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
                return None;
            }
            start.numbers = restarted;
            super::memory_promotion::spawn_restarted(&mut store, &place.project_id, start)
        }
    }
}
