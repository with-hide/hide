//! The 제안 tab's Memory rows (PRD hide-ai-suggestions D-02, D-42, B11,
//! B12): a Memory card's way into the repository's documents, which the
//! tab took over from the Memory tab. Memory still decides which card is
//! suggested and follows its promotion (`memory_promotion`); this module
//! only draws what the Memory section already holds as rows, and hands a
//! row's press back to Memory's own action.

use super::*;
use crate::model::{
    HomeTurn, MemoryAction, MemoryPromotionSnapshot, MemoryPromotionState, MemoryQuestionSnapshot,
    MemoryRowSnapshot, MemoryStartFailureCode, ProjectSuggestionsSnapshot, SuggestionAttemptPhase,
    SuggestionAttemptSnapshot, SuggestionGroup, SuggestionRowSnapshot, SuggestionSource,
    SuggestionState,
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
        let questions = memory
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
        Some(MemoryFacts {
            workspace_id: workspace_id.to_owned(),
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
    /// retry promote it, dismiss is `안 올림` (B29).
    pub(super) fn apply_memory_row_action(
        &mut self,
        workspace_id: String,
        number: u64,
        action: MemoryAction,
    ) -> bool {
        self.apply_memory_action(events::MemoryActionPayload {
            numbers: Vec::new(),
            workspace_id,
            action,
            number: Some(number),
            change: None,
        })
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
        let row = match question {
            MemoryQuestionSnapshot::Promote { number, title, .. } => {
                if !offered || room == 0 {
                    continue;
                }
                room -= 1;
                row(
                    *number,
                    title,
                    SuggestionGroup::Choose,
                    SuggestionState::New,
                )
            }
            MemoryQuestionSnapshot::Failed {
                number,
                title,
                start_failure,
            } => {
                let mut row = row(
                    *number,
                    title,
                    SuggestionGroup::Applying,
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
            MemoryQuestionSnapshot::Unbuilt {
                number,
                title,
                reason,
            } => {
                let mut row = row(
                    *number,
                    title,
                    SuggestionGroup::Applying,
                    SuggestionState::MyTurn,
                );
                row.cause = reason.clone();
                row.attempt = Some(ended(started_at(facts, *number), "unbuilt"));
                row
            }
            MemoryQuestionSnapshot::DocsDisagree { .. } => continue,
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
    let rank = |group: SuggestionGroup| match group {
        SuggestionGroup::Choose => 0,
        SuggestionGroup::Applying => 1,
        SuggestionGroup::Applied => 2,
    };
    project.rows.sort_by_key(|row| rank(row.group));
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
    }
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
    let mine = (SuggestionGroup::Applying, SuggestionState::MyTurn);
    let (phase, (group, state)) = match promotion.state {
        MemoryPromotionState::Running
        | MemoryPromotionState::Building
        | MemoryPromotionState::Unconfirmed => (SuggestionAttemptPhase::Working, working),
        MemoryPromotionState::Ready => (SuggestionAttemptPhase::Ready, mine),
        MemoryPromotionState::PrOpen => match turn {
            Some(HomeTurn::PrReview { .. }) => (SuggestionAttemptPhase::PrOpen, mine),
            Some(HomeTurn::CiFailed { .. }) => (SuggestionAttemptPhase::CiFailed, mine),
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
    /// a question and a promotion is drawn once.
    #[test]
    fn a_cards_way_into_the_documents_is_drawn_as_a_suggestion_row() {
        let mut project = tab();
        let facts = MemoryFacts {
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
                ("m1", G::Choose, S::New),
                ("m2", G::Applying, S::MyTurn),
                ("m3", G::Applying, S::MyTurn),
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

    #[test]
    fn a_memory_row_id_names_its_card() {
        assert_eq!(card_of_row("m72"), Some(72));
        assert_eq!(card_of_row("s72"), None);
        assert_eq!(card_of_row("m"), None);
        assert_eq!(card_of_row("m7x"), None);
    }
}
