//! The 제안 tab's Memory rows (PRD hide-ai-suggestions D-02, D-42, B11,
//! B12): a Memory card's way into the repository's documents, which the
//! tab took over from the Memory tab. Memory still decides which card is
//! suggested and follows its promotion (`memory_promotion`); this module
//! draws what the Memory section already holds as rows, and hands a row's
//! press back to Memory's own action, holding the press on its row until a
//! read of the store shows what it did.

use super::*;
use crate::model::{
    HomeTurn, MemoryAction, MemoryPromotionSnapshot, MemoryPromotionState, MemoryQuestionSnapshot,
    MemoryRowSnapshot, MemoryStartFailureCode, ProjectSuggestionsSnapshot, SuggestionAttemptPhase,
    SuggestionAttemptSnapshot, SuggestionGroup, SuggestionMemorySnapshot, SuggestionRowSnapshot,
    SuggestionSource, SuggestionState,
};
use crate::suggestions::notebook::Outcome;
use crate::suggestions::policy::APPLIED_ROWS;
use std::collections::BTreeSet;

/// What the Memory rows are drawn from, compared on each delta so the tab
/// is composed again only when one of them moved.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct MemoryFacts {
    workspace_id: String,
    questions: Vec<MemoryQuestionSnapshot>,
    promoting: Vec<Promoting>,
    /// The turn the sidebar laid on each promoting card's agent, in the
    /// order of `promoting`.
    turns: Vec<Option<HomeTurn>>,
    /// What the tracker knows of each promoting card's batch branch, in the
    /// order of `promoting` (PRD hide-ai-branch-first-apply D-13).
    batches: Vec<Option<super::memory_branch::BatchBranchFacts>>,
    /// The text of each card offered for the documents, which its row shows
    /// when it opens, by card.
    bodies: Vec<(u64, Option<String>)>,
    /// The rows' presses on cards that no read has answered yet, by card.
    pressed: Vec<(u64, Pressed)>,
}

/// What a row draws of a press on its card before a read of the store shows
/// what the press did.
#[derive(Clone, Debug, PartialEq)]
enum Pressed {
    /// The card is on its way into the documents from the press on.
    Promoting { pressed_at_unix_ms: u64 },
    /// The card's dismissal is underway; its row is out of the tab.
    Dismissing,
    /// The promotion did not start, and why.
    Refused {
        refusal: Refusal,
        pressed_at_unix_ms: u64,
    },
}

/// Why a press did not start: the code the tab words, and the core's
/// sentence, which the tab shows only under a code it has no words for.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Refusal {
    code: &'static str,
    reason: String,
}

impl Refusal {
    /// A refusal Memory answered with `kind` (`memory.branches_full`).
    pub(super) fn of(kind: &str, reason: String) -> Self {
        let code = match kind {
            "memory.branches_full" => "branches_full",
            "memory.not_git" => "not_git",
            "memory.promotion_unavailable" => "promotion_unavailable",
            "memory.action_busy" => "action_busy",
            "memory.presses_full" => "presses_full",
            "memory.worker_unavailable" | "memory.worker_failed" => "store_unavailable",
            "memory.project_unavailable" => "project_unavailable",
            "memory.project_unreachable" => "project_unreachable",
            // The store's own refusals each say something different
            // (`m3 is no longer in Memory`), so their sentence is the reason.
            _ => "not_started",
        };
        Self { code, reason }
    }
}

/// The most row presses held at once; settled ones give way, oldest first.
const ROW_PRESSES: usize = 16;
/// The most presses waiting or running at once; a press past it is refused
/// on its row.
const PRESSES_UNDERWAY: usize = 8;

/// The 제안 rows' presses on Memory cards. Memory runs one action at a
/// time and a promotion's action lasts until its agent started, so a press
/// meanwhile waits here for its turn rather than being refused; each press
/// is kept until the read after its write lands, so its row hands over to
/// what the store says without going back to its button.
#[derive(Default)]
pub(super) struct RowPresses(Vec<RowPress>);

struct RowPress {
    workspace_id: String,
    number: u64,
    action: MemoryAction,
    pressed_at_unix_ms: u64,
    stage: PressStage,
}

#[derive(PartialEq)]
enum PressStage {
    /// Behind the Memory action that runs; it starts when that one ends.
    Waiting,
    /// Its write runs.
    Running,
    /// Written; a read of this generation or later shows what it did.
    Written(u64),
    /// Refused or failed before the store took it, with why.
    Refused(Refusal),
}

impl RowPress {
    fn underway(&self) -> bool {
        matches!(self.stage, PressStage::Waiting | PressStage::Running)
    }

    fn of(&self, workspace_id: &str, number: u64) -> bool {
        self.workspace_id == workspace_id && self.number == number
    }

    fn drawn(&self) -> Option<Pressed> {
        match (&self.stage, self.action) {
            (PressStage::Refused(refusal), MemoryAction::Promote) => Some(Pressed::Refused {
                refusal: refusal.clone(),
                pressed_at_unix_ms: self.pressed_at_unix_ms,
            }),
            // A refused dismissal leaves the row as it was; Memory reports
            // why.
            (PressStage::Refused(_), _) => None,
            (_, MemoryAction::Dismiss) => Some(Pressed::Dismissing),
            (_, _) => Some(Pressed::Promoting {
                pressed_at_unix_ms: self.pressed_at_unix_ms,
            }),
        }
    }
}

impl RowPresses {
    /// A read of `generation` landed: the presses it shows are let go.
    pub(super) fn shown(&mut self, generation: u64) {
        self.0.retain(
            |press| !matches!(press.stage, PressStage::Written(written) if written <= generation),
        );
    }
}

/// A card on its way into the documents, from the Memory list.
#[derive(Clone, Debug, PartialEq)]
struct Promoting {
    number: u64,
    title: String,
    promotion: MemoryPromotionSnapshot,
}

/// The cards with a promotion in the last Memory list read, kept with the
/// list they came from: the list is thousands of cards, so it is walked
/// once per read rather than on every delta.
#[derive(Default)]
pub(super) struct PromotingCache {
    rows: Option<Arc<Vec<MemoryRowSnapshot>>>,
    promoting: Vec<Promoting>,
}

impl Runtime {
    /// The opened Project's Memory facts; none while the Memory section
    /// holds another Project or none.
    pub(super) fn suggestion_memory_facts(&mut self, workspace_id: &str) -> Option<MemoryFacts> {
        let memory = self
            .snapshot
            .memory
            .as_ref()
            .and_then(|memory| memory.project.as_ref())
            .filter(|memory| memory.workspace_id == workspace_id && memory.enabled)?;
        let questions: Vec<MemoryQuestionSnapshot> = memory
            .questions
            .iter()
            .filter(|question| !question.memory_tab_owns())
            .cloned()
            .collect();
        let cache = &mut self.suggestions_work.promoting;
        if !cache
            .rows
            .as_ref()
            .is_some_and(|rows| Arc::ptr_eq(rows, &memory.rows))
        {
            cache.promoting = memory
                .rows
                .iter()
                .filter_map(|row| {
                    Some(Promoting {
                        number: row.number,
                        title: row.title.clone(),
                        promotion: row.promotion.clone()?,
                    })
                })
                .collect();
            cache.rows = Some(Arc::clone(&memory.rows));
        }
        let turns = self
            .suggestions_work
            .promoting
            .promoting
            .iter()
            .map(|card| {
                let pane = card.promotion.agent.as_ref()?.pane_id.as_deref()?;
                self.hide_ai_turn(pane)
            })
            .collect();
        let batches = self
            .suggestions_work
            .promoting
            .promoting
            .iter()
            .map(|card| {
                let branch = card.promotion.branch.as_deref()?;
                self.memory_batch_facts(branch).cloned()
            })
            .collect();
        let bodies = questions
            .iter()
            .filter_map(|question| match *question {
                MemoryQuestionSnapshot::Promote { number, .. } => Some(number),
                _ => None,
            })
            .map(|number| {
                let body = memory
                    .rows
                    .iter()
                    .find(|row| row.number == number)
                    .and_then(|row| row.body_preview.clone());
                (number, body)
            })
            .collect();
        let pressed = self
            .project_memory
            .presses
            .0
            .iter()
            .filter(|press| press.workspace_id == workspace_id)
            .filter_map(|press| Some((press.number, press.drawn()?)))
            .collect();
        Some(MemoryFacts {
            workspace_id: workspace_id.to_owned(),
            bodies,
            pressed,
            questions,
            promoting: self.suggestions_work.promoting.promoting.clone(),
            turns,
            batches,
        })
    }

    /// The turn the sidebar laid on the Hide-started agent in `pane_id`, on
    /// this machine or a linked device.
    fn hide_ai_turn(&self, pane_id: &str) -> Option<HomeTurn> {
        self.snapshot
            .navigator
            .agents
            .iter()
            .chain(
                self.snapshot
                    .status
                    .remote
                    .iter()
                    .filter_map(|remote| remote.session.as_ref())
                    .flat_map(|session| session.agents.iter()),
            )
            .find(|agent| agent.pane_id == pane_id)
            .and_then(|agent| agent.hide_ai.as_ref()?.turn)
    }

    /// A press on a Memory row is Memory's own action on its card: apply and
    /// retry promote it, dismiss is `안 올림` (B29). The row shows the press
    /// at once; behind another Memory action it waits for that one to end.
    pub(super) fn apply_memory_row_action(
        &mut self,
        workspace_id: String,
        number: u64,
        action: MemoryAction,
    ) -> bool {
        let presses = &mut self.project_memory.presses.0;
        // A second press on a card underway is the same intent (rule 11).
        if presses
            .iter()
            .any(|press| press.of(&workspace_id, number) && press.underway())
        {
            return false;
        }
        presses.retain(|press| !press.of(&workspace_id, number));
        let stage = if presses.iter().filter(|press| press.underway()).count() < PRESSES_UNDERWAY {
            PressStage::Waiting
        } else {
            crate::diagnostic!(serde_json::json!({
                "component": "memory", "kind": "memory.presses_full",
                "workspace_id": workspace_id, "card": hide_memory::card_name(number),
            }));
            PressStage::Refused(Refusal::of(
                "memory.presses_full",
                "Too many Memory changes are waiting; press again when one finishes".into(),
            ))
        };
        presses.push(RowPress {
            workspace_id,
            number,
            action,
            pressed_at_unix_ms: super::memory::now_ms(),
            stage,
        });
        while presses.len() > ROW_PRESSES {
            let Some(settled) = presses.iter().position(|press| !press.underway()) else {
                break;
            };
            presses.remove(settled);
        }
        self.run_row_presses();
        true
    }

    /// Starts the waiting presses in the order they came while no Memory
    /// action runs; a refused one is drawn refused on its row.
    pub(super) fn run_row_presses(&mut self) {
        while self.project_memory.action.is_none() {
            let Some(press) = self
                .project_memory
                .presses
                .0
                .iter_mut()
                .find(|press| press.stage == PressStage::Waiting)
            else {
                return;
            };
            press.stage = PressStage::Running;
            let workspace_id = press.workspace_id.clone();
            let number = press.number;
            let action = press.action;
            let refused = self.begin_memory_action(events::MemoryActionPayload {
                numbers: Vec::new(),
                workspace_id: workspace_id.clone(),
                action,
                number: Some(number),
                change: None,
            });
            if let Err(refusal) = refused {
                self.settle_row_press(
                    &workspace_id,
                    number,
                    Err(Refusal::of(refusal.kind, refusal.message.clone())),
                );
                self.memory_action_failed(
                    &workspace_id,
                    action,
                    refusal.kind,
                    refusal.message,
                    refusal.retryable,
                );
            }
        }
    }

    /// The running press on card `number` was written, to be shown by a
    /// read of the given generation, or refused with why.
    pub(super) fn settle_row_press(
        &mut self,
        workspace_id: &str,
        number: u64,
        outcome: Result<u64, Refusal>,
    ) {
        if let Some(press) = self
            .project_memory
            .presses
            .0
            .iter_mut()
            .find(|press| press.of(workspace_id, number) && press.stage == PressStage::Running)
        {
            press.stage = match outcome {
                Ok(generation) => PressStage::Written(generation),
                Err(refusal) => PressStage::Refused(refusal),
            };
        }
    }
}

/// The card a Memory row id (`m72`) names.
pub(super) fn card_of_row(id: &str) -> Option<u64> {
    let digits = id.strip_prefix('m')?;
    (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| digits.parse().ok())
        .flatten()
}

/// Lays the Memory rows into the tab: a suggested card is a new
/// suggestion while the notebook leaves room under the quantity's cap
/// (D-42), and every card on its way is drawn where its promotion stands.
/// Rows stay in the tab's drawing order, the folded 반영됨 at most
/// [`APPLIED_ROWS`] long.
pub(super) fn lay_memory_rows(project: &mut ProjectSuggestionsSnapshot, facts: &MemoryFacts) {
    let offered = project.enabled && project.settings.suggestions_enabled;
    let open = project
        .rows
        .iter()
        .filter(|row| row.state == SuggestionState::New)
        .count();
    let mut room = project.settings.quantity.open_cap().saturating_sub(open);
    let mut drawn = BTreeSet::new();
    for question in &facts.questions {
        let (number, title) = match question {
            MemoryQuestionSnapshot::Promote { number, title, .. }
            | MemoryQuestionSnapshot::Failed { number, title, .. }
            | MemoryQuestionSnapshot::Unbuilt { number, title, .. } => (*number, title),
            MemoryQuestionSnapshot::DocsDisagree { .. } => continue,
        };
        let pressed = facts
            .pressed
            .iter()
            .find(|(card, _)| *card == number)
            .map(|(_, pressed)| pressed);
        let row = match (question, pressed) {
            (_, Some(Pressed::Dismissing)) => continue,
            (_, Some(Pressed::Promoting { pressed_at_unix_ms })) => {
                pressed_row(number, title, *pressed_at_unix_ms)
            }
            (
                _,
                Some(Pressed::Refused {
                    refusal,
                    pressed_at_unix_ms,
                }),
            ) => {
                let mut row = row(
                    number,
                    title,
                    SuggestionGroup::of_attempt(SuggestionAttemptPhase::Failed),
                    SuggestionState::MyTurn,
                );
                row.cause = Some(refusal.reason.clone());
                row.attempt = Some(ended(*pressed_at_unix_ms, refusal.code));
                row
            }
            (
                MemoryQuestionSnapshot::Promote {
                    number,
                    title,
                    human_session_count,
                    pulled_session_count,
                },
                None,
            ) => {
                if !offered || room == 0 {
                    continue;
                }
                room -= 1;
                let mut row = row(
                    *number,
                    title,
                    SuggestionGroup::Choose,
                    SuggestionState::New,
                );
                row.memory = Some(SuggestionMemorySnapshot {
                    body: facts
                        .bodies
                        .iter()
                        .find(|(card, _)| card == number)
                        .and_then(|(_, body)| body.clone()),
                    human_sessions: *human_session_count,
                    pulled_sessions: *pulled_session_count,
                });
                row
            }
            (
                MemoryQuestionSnapshot::Failed {
                    number,
                    title,
                    start_failure,
                },
                None,
            ) => {
                let mut row = row(
                    *number,
                    title,
                    SuggestionGroup::of_attempt(SuggestionAttemptPhase::Failed),
                    SuggestionState::MyTurn,
                );
                row.cause = start_failure
                    .as_ref()
                    .and_then(|failure| failure.detail.clone());
                row.attempt = Some(ended(
                    started_at(facts, *number),
                    match start_failure.as_ref().map(|failure| failure.code) {
                        None => "no_change",
                        Some(MemoryStartFailureCode::AgentNotStarted) => "agent_not_started",
                        Some(MemoryStartFailureCode::ShellStarting) => "shell_starting",
                        Some(MemoryStartFailureCode::CardNotProcessed) => "card_not_processed",
                        Some(MemoryStartFailureCode::Other) => "start_failed",
                    },
                ));
                row
            }
            (
                MemoryQuestionSnapshot::Unbuilt {
                    number,
                    title,
                    reason,
                },
                None,
            ) => {
                let mut row = row(
                    *number,
                    title,
                    SuggestionGroup::of_attempt(SuggestionAttemptPhase::Failed),
                    SuggestionState::MyTurn,
                );
                row.cause = reason.clone();
                row.attempt = Some(ended(started_at(facts, *number), "unbuilt"));
                row
            }
            (MemoryQuestionSnapshot::DocsDisagree { .. }, None) => continue,
        };
        drawn.insert(row.id.clone());
        project.rows.push(row);
    }
    // A batch is one row, named by its first card (PRD
    // hide-ai-branch-first-apply D-13, B20); a card from before batches is
    // its own.
    let mut batches: Vec<(Option<&str>, Vec<usize>)> = Vec::new();
    for (index, card) in facts.promoting.iter().enumerate() {
        let branch = card.promotion.branch.as_deref();
        match batches
            .iter_mut()
            .find(|(seen, _)| branch.is_some() && *seen == branch)
        {
            Some((_, members)) => members.push(index),
            None => batches.push((branch, vec![index])),
        }
    }
    for (_, members) in batches {
        let cards = members
            .iter()
            .map(|index| &facts.promoting[*index])
            .collect::<Vec<_>>();
        let turn = members.iter().find_map(|index| facts.turns[*index]);
        let batch = members
            .iter()
            .find_map(|index| facts.batches.get(*index).cloned().flatten());
        let Some(row) = batch_row(&cards, turn, batch.as_ref()) else {
            continue;
        };
        if drawn.insert(row.id.clone()) {
            project.rows.push(row);
        }
    }
    project
        .rows
        .sort_by_key(SuggestionRowSnapshot::drawing_rank);
    let applied = project
        .rows
        .iter()
        .position(|row| row.group == SuggestionGroup::Applied)
        .unwrap_or(project.rows.len());
    let mut folded = project.rows.split_off(applied);
    folded.sort_by_key(|row| {
        std::cmp::Reverse(row.attempt.as_ref().map_or(0, |a| a.started_at_unix_ms))
    });
    folded.truncate(APPLIED_ROWS);
    project.rows.extend(folded);
}

fn row(
    number: u64,
    title: &str,
    group: SuggestionGroup,
    state: SuggestionState,
) -> SuggestionRowSnapshot {
    SuggestionRowSnapshot {
        id: format!("m{number}"),
        source: SuggestionSource::Memory,
        group,
        state,
        discomfort: Some(title.to_owned()),
        cause: None,
        fix: None,
        plan: None,
        outcome: Outcome::Pr,
        basis: None,
        auto: false,
        hold: None,
        returned_after_skip: false,
        escalated: None,
        evidence: Vec::new(),
        attempt: None,
        watch: None,
        local: None,
        evidence_unconfirmed: false,
        memory_cards: Vec::new(),
        memory: None,
    }
}

/// A row pressed to fix the documents, drawn working from the press until
/// the store's promotion takes its place.
fn pressed_row(number: u64, title: &str, pressed_at_unix_ms: u64) -> SuggestionRowSnapshot {
    let mut row = row(
        number,
        title,
        SuggestionGroup::Applying,
        SuggestionState::HideAiWorking,
    );
    row.memory_cards = vec![number];
    row.attempt = Some(SuggestionAttemptSnapshot {
        phase: SuggestionAttemptPhase::Working,
        failure: None,
        ..ended(pressed_at_unix_ms, "")
    });
    row
}

fn started_at(facts: &MemoryFacts, number: u64) -> u64 {
    facts
        .promoting
        .iter()
        .find(|card| card.number == number)
        .map_or(0, |card| card.promotion.started_at_unix_ms)
}

fn ended(started_at_unix_ms: u64, failure: &str) -> SuggestionAttemptSnapshot {
    SuggestionAttemptSnapshot {
        phase: SuggestionAttemptPhase::Failed,
        started_at_unix_ms,
        pr_number: None,
        url: None,
        agent_id: None,
        agent_pane: None,
        failure: Some(failure.to_owned()),
        branch: None,
        purpose: crate::suggestions::lifecycle::AttemptPurpose::Branch,
    }
}

/// A batch whose promotion runs, waits on its branch or its pull request,
/// or merged, drawn as one row named by its first card (B20); a failed or
/// unbuilt card is drawn from its question, and none without one.
fn batch_row(
    cards: &[&Promoting],
    turn: Option<HomeTurn>,
    batch: Option<&super::memory_branch::BatchBranchFacts>,
) -> Option<SuggestionRowSnapshot> {
    // The batch stands where its least settled card stands: a card still
    // running keeps the row working.
    let card = cards
        .iter()
        .min_by_key(|card| state_rank(card.promotion.state))?;
    let promotion = &card.promotion;
    let working = (SuggestionGroup::Applying, SuggestionState::HideAiWorking);
    let mine = |phase| (SuggestionGroup::of_attempt(phase), SuggestionState::MyTurn);
    let (phase, (group, state)) = match promotion.state {
        MemoryPromotionState::Running
        | MemoryPromotionState::Building
        | MemoryPromotionState::Unconfirmed => (SuggestionAttemptPhase::Working, working),
        MemoryPromotionState::Ready => (
            SuggestionAttemptPhase::Ready,
            mine(SuggestionAttemptPhase::Ready),
        ),
        MemoryPromotionState::PrOpen => match turn {
            Some(HomeTurn::PrReview { .. }) => (
                SuggestionAttemptPhase::PrOpen,
                mine(SuggestionAttemptPhase::PrOpen),
            ),
            Some(HomeTurn::CiFailed { .. }) => (
                SuggestionAttemptPhase::CiFailed,
                mine(SuggestionAttemptPhase::CiFailed),
            ),
            _ => (SuggestionAttemptPhase::PrOpen, working),
        },
        MemoryPromotionState::Merged => (
            SuggestionAttemptPhase::Merged,
            (SuggestionGroup::Applied, SuggestionState::Applied),
        ),
        MemoryPromotionState::Failed | MemoryPromotionState::Unbuilt => return None,
    };
    let first = cards.iter().map(|card| card.number).min()?;
    let mut row = row(first, &card.title, group, state);
    row.memory_cards = cards.iter().map(|card| card.number).collect();
    row.memory_cards.sort_unstable();
    row.attempt = Some(SuggestionAttemptSnapshot {
        phase,
        started_at_unix_ms: promotion.started_at_unix_ms,
        pr_number: promotion
            .pr_number
            .and_then(|number| u32::try_from(number).ok()),
        url: promotion.pr_url.clone(),
        agent_id: cards
            .iter()
            .find_map(|card| card.promotion.agent.as_ref()?.agent_id.clone()),
        // The promotion names its agent's pane as the agent lists do, for
        // as long as the agent lives.
        agent_pane: cards
            .iter()
            .find_map(|card| card.promotion.agent.as_ref()?.pane_id.clone()),
        failure: promotion.failure.clone(),
        branch: promotion.branch.as_ref().map(|name| {
            let summary = batch.and_then(|batch| batch.summary.as_ref());
            crate::model::SuggestionBranchSnapshot {
                name: name.clone(),
                base: summary.map(|summary| summary.base.clone()),
                checkout: None,
                files: summary.map_or_else(Vec::new, |summary| {
                    summary
                        .files
                        .iter()
                        .map(|file| crate::model::SuggestionBranchFileSnapshot {
                            path: file.path.clone(),
                            added: file.added,
                            removed: file.removed,
                            binary: file.binary,
                        })
                        .collect()
                }),
                files_total: summary.map_or(0, |summary| summary.files_total),
                added: summary.map_or(0, |summary| summary.added),
                removed: summary.map_or(0, |summary| summary.removed),
                tip: summary.map(|summary| summary.tip.clone()),
                push: crate::model::SuggestionBranchPush::Unknown,
                base_unpushed: None,
                moved_to: None,
                base_unsaved: Vec::new(),
            }
        }),
        purpose: super::memory_branch::attempt_purpose(batch.and_then(|batch| batch.purpose)),
    });
    Some(row)
}

/// How far a card's promotion is from settled, least first.
fn state_rank(state: MemoryPromotionState) -> u8 {
    match state {
        MemoryPromotionState::Running
        | MemoryPromotionState::Unconfirmed
        | MemoryPromotionState::Building => 0,
        MemoryPromotionState::Ready => 1,
        MemoryPromotionState::PrOpen => 2,
        MemoryPromotionState::Failed | MemoryPromotionState::Unbuilt => 3,
        MemoryPromotionState::Merged => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MemoryPromotionAgentSnapshot, MemoryStartFailure};

    fn promoting(number: u64, state: MemoryPromotionState) -> Promoting {
        Promoting {
            number,
            title: format!("card {number}"),
            promotion: MemoryPromotionSnapshot {
                state,
                started_at_unix_ms: number * 1_000,
                agent: Some(MemoryPromotionAgentSnapshot {
                    name: format!("memory-promote-m{number}"),
                    pane_id: Some(format!("p{number}")),
                    agent_id: Some(format!("a{number}")),
                }),
                pr_number: Some(900 + number),
                pr_url: Some(format!("https://github.com/acme/app/pull/{}", 900 + number)),
                target_path: None,
                github_failure: None,
                branch: None,
                failure: None,
            },
        }
    }

    fn tab() -> ProjectSuggestionsSnapshot {
        let mut project = ProjectSuggestionsSnapshot {
            enabled: true,
            ..Default::default()
        };
        project.settings.suggestions_enabled = true;
        project
    }

    fn drawn(
        project: &ProjectSuggestionsSnapshot,
    ) -> Vec<(&str, SuggestionGroup, SuggestionState)> {
        project
            .rows
            .iter()
            .map(|row| (row.id.as_str(), row.group, row.state))
            .collect()
    }

    /// D-02, B11, B12: each Memory question and promotion becomes the row
    /// the 제안 tab draws it as, in the tab's group order; a card with both
    /// a question and a promotion is drawn once. A failed or unbuilt card
    /// waits for the operator's choice in 고를 것, ahead of new suggestions.
    #[test]
    fn a_cards_way_into_the_documents_is_drawn_as_a_suggestion_row() {
        let mut project = tab();
        let facts = MemoryFacts {
            pressed: Vec::new(),
            bodies: Vec::new(),
            workspace_id: "w".into(),
            questions: vec![
                MemoryQuestionSnapshot::Promote {
                    number: 1,
                    title: "card 1".into(),
                    human_session_count: 2,
                    pulled_session_count: 0,
                },
                MemoryQuestionSnapshot::Failed {
                    number: 2,
                    title: "card 2".into(),
                    start_failure: Some(MemoryStartFailure {
                        code: MemoryStartFailureCode::AgentNotStarted,
                        detail: None,
                    }),
                },
                MemoryQuestionSnapshot::Unbuilt {
                    number: 3,
                    title: "card 3".into(),
                    reason: Some("the switch is not built".into()),
                },
            ],
            promoting: vec![
                promoting(2, MemoryPromotionState::Failed),
                promoting(4, MemoryPromotionState::Running),
                promoting(5, MemoryPromotionState::PrOpen),
                promoting(6, MemoryPromotionState::PrOpen),
                promoting(7, MemoryPromotionState::Merged),
            ],
            turns: vec![
                None,
                None,
                Some(HomeTurn::PrReview { pr_number: 905 }),
                None,
                None,
            ],
            batches: Vec::new(),
        };
        lay_memory_rows(&mut project, &facts);
        use SuggestionGroup as G;
        use SuggestionState as S;
        assert_eq!(
            drawn(&project),
            [
                ("m2", G::Choose, S::MyTurn),
                ("m3", G::Choose, S::MyTurn),
                ("m1", G::Choose, S::New),
                ("m4", G::Applying, S::HideAiWorking),
                ("m5", G::Applying, S::MyTurn),
                ("m6", G::Applying, S::HideAiWorking),
                ("m7", G::Applied, S::Applied),
            ]
        );
        let attempt = |id: &str| {
            project
                .rows
                .iter()
                .find(|row| row.id == id)
                .and_then(|row| row.attempt.clone())
                .unwrap()
        };
        assert_eq!(attempt("m2").failure.as_deref(), Some("agent_not_started"));
        assert_eq!(attempt("m2").started_at_unix_ms, 2_000);
        assert_eq!(attempt("m3").failure.as_deref(), Some("unbuilt"));
        assert_eq!(attempt("m4").agent_id.as_deref(), Some("a4"));
        assert_eq!(attempt("m5").phase, SuggestionAttemptPhase::PrOpen);
        assert_eq!(attempt("m5").pr_number, Some(905));
        assert_eq!(attempt("m7").phase, SuggestionAttemptPhase::Merged);
        let unbuilt = project.rows.iter().find(|row| row.id == "m3").unwrap();
        assert_eq!(unbuilt.cause.as_deref(), Some("the switch is not built"));
        assert!(
            project
                .rows
                .iter()
                .all(|row| row.source == SuggestionSource::Memory)
        );
    }

    /// D-42: a suggested card is a new suggestion only while the notebook's
    /// new lines leave room under the quantity's cap, and none at all while
    /// suggestions are off; a card on its way is drawn regardless.
    #[test]
    fn suggested_cards_share_the_quantitys_cap_with_the_notebook() {
        let promote = |number: u64| MemoryQuestionSnapshot::Promote {
            number,
            title: format!("card {number}"),
            human_session_count: 2,
            pulled_session_count: 0,
        };
        let facts = MemoryFacts {
            pressed: Vec::new(),
            bodies: Vec::new(),
            workspace_id: "w".into(),
            questions: vec![promote(1), promote(2), promote(3)],
            promoting: vec![promoting(4, MemoryPromotionState::Running)],
            turns: vec![None],
            batches: Vec::new(),
        };
        let mut project = tab();
        project.rows.push(SuggestionRowSnapshot {
            source: SuggestionSource::Notebook,
            ..row(0, "line", SuggestionGroup::Choose, SuggestionState::New)
        });
        project.rows[0].id = "s1".into();
        lay_memory_rows(&mut project, &facts);
        let ids = project
            .rows
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["s1", "m1", "m2", "m4"], "Normal opens three");

        let mut off = tab();
        off.settings.suggestions_enabled = false;
        lay_memory_rows(&mut off, &facts);
        let ids = off
            .rows
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["m4"]);
    }

    /// A press is drawn on its row until a read answers it: a promotion
    /// working from the press, a refused one failed with why, a dismissal
    /// out of the tab.
    #[test]
    fn a_press_is_drawn_on_its_row_until_a_read_answers_it() {
        let promote = |number: u64| MemoryQuestionSnapshot::Promote {
            number,
            title: format!("card {number}"),
            human_session_count: 2,
            pulled_session_count: 0,
        };
        let facts = MemoryFacts {
            workspace_id: "w".into(),
            questions: vec![promote(1), promote(2), promote(3)],
            pressed: vec![
                (
                    1,
                    Pressed::Promoting {
                        pressed_at_unix_ms: 7,
                    },
                ),
                (
                    2,
                    Pressed::Refused {
                        refusal: Refusal::of("memory.branches_full", "busy".into()),
                        pressed_at_unix_ms: 8,
                    },
                ),
                (3, Pressed::Dismissing),
            ],
            ..MemoryFacts::default()
        };
        let mut project = tab();
        lay_memory_rows(&mut project, &facts);
        use SuggestionGroup as G;
        use SuggestionState as S;
        assert_eq!(
            drawn(&project),
            [
                ("m2", G::Choose, S::MyTurn),
                ("m1", G::Applying, S::HideAiWorking)
            ]
        );
        let working = project.rows[1].attempt.as_ref().unwrap();
        assert_eq!(
            (working.phase, working.started_at_unix_ms),
            (SuggestionAttemptPhase::Working, 7)
        );
        assert_eq!(project.rows[1].memory_cards, [1]);
        let refused = &project.rows[0];
        assert_eq!(refused.cause.as_deref(), Some("busy"));
        assert_eq!(
            refused.attempt.as_ref().unwrap().failure.as_deref(),
            Some("branches_full")
        );
    }

    #[test]
    fn a_memory_row_id_names_its_card() {
        assert_eq!(card_of_row("m72"), Some(72));
        assert_eq!(card_of_row("s72"), None);
        assert_eq!(card_of_row("m"), None);
        assert_eq!(card_of_row("m7x"), None);
    }
}
