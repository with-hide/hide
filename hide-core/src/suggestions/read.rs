//! A Project's 제안 tab as the worker reads it from the store (PRD
//! hide-ai-suggestions B10-B17, D-07, D-37, D-44): every group, state and
//! status decided here, off the lock, from the notebook, each line's story
//! and the records. The runtime lays only the facts the lock has on it.

use super::ledger::{RecordEntry, RecordId, RecordIndex, SessionRef, Signal};
use super::lifecycle::{AttemptPhase, LineLife, LocalFailure};
use super::notebook::{Line, LineId, Notebook, Outcome, Stage, StruckBy};
use super::policy;
use super::settings::{ApplyMode, Settings};
use super::store::ProjectState;
use crate::model::{
    ProjectSuggestionsSnapshot, SuggestionAttemptPhase, SuggestionAttemptSnapshot,
    SuggestionBasisSnapshot, SuggestionDismissedSnapshot, SuggestionEscalationSnapshot,
    SuggestionEvidenceKind, SuggestionEvidenceSnapshot, SuggestionGroup, SuggestionHold,
    SuggestionRowSnapshot, SuggestionSettingsSnapshot, SuggestionSignalSnapshot, SuggestionSource,
    SuggestionState, SuggestionWatchSnapshot, SuggestionsStatus,
};
use std::collections::{BTreeMap, BTreeSet};

/// The row id of a notebook line, as every `suggestion_action` names it.
pub fn row_id(line: LineId) -> String {
    format!("s{}", line.0)
}

/// The line a row id names, if it names one.
pub fn line_of_row(id: &str) -> Option<LineId> {
    let digits = id.strip_prefix('s')?;
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| digits.parse().ok().map(LineId))
        .flatten()
}

/// The one line a global instruction fix writes: what the operator's newest
/// words behind the line say it should be (PRD memory-self-improve D-07).
pub fn local_line(line: &Line, index: &RecordIndex) -> Option<String> {
    line.records
        .iter()
        .filter_map(|id| index.get(*id))
        .filter_map(|record| Some((record.at_unix_ms, record.id, record.said.as_ref()?)))
        .max_by_key(|(at, id, _)| (*at, *id))
        .map(|(_, _, said)| said.line.clone())
}

/// Everything one read needs.
pub struct ReadInput<'a> {
    pub workspace_id: &'a str,
    pub state: &'a ProjectState,
    pub notebook: &'a Notebook,
    pub lives: &'a BTreeMap<LineId, LineLife>,
    pub index: &'a RecordIndex,
    pub investigating: bool,
    /// The intro line was shown once on this machine (B3).
    pub intro_seen: bool,
    /// Today's local date, `YYYY-MM-DD`.
    pub today: &'a str,
    pub sources: &'a EvidenceSources,
    /// The Hide global instruction as the worker read it, with its file.
    pub global: &'a GlobalRead,
}

/// The Hide global instruction file and what a read of it found.
pub struct GlobalRead {
    pub path: std::path::PathBuf,
    pub read: Result<crate::global_instruction::Instruction, crate::global_instruction::Unusable>,
}

impl GlobalRead {
    pub fn of(state_dir: &std::path::Path) -> Self {
        Self {
            path: hide_kit::layout::global_instruction(state_dir),
            read: crate::global_instruction::read(state_dir),
        }
    }
}

/// A global instruction row's state: its line, the preview a press is
/// checked against and the last press's failure (PRD memory-self-improve
/// B6-B11).
fn local(
    line: &Line,
    life: &LineLife,
    index: &RecordIndex,
    global: &GlobalRead,
) -> Option<crate::model::SuggestionLocalSnapshot> {
    if line.outcome != Some(Outcome::Local) {
        return None;
    }
    let applied = life.local.as_ref();
    let text = applied
        .and_then(|local| local.line.clone())
        .filter(|_| line.stage == Stage::Settled)
        .or_else(|| local_line(line, index))?;
    let present = global.read.as_ref().is_ok_and(|current| {
        current
            .text
            .lines()
            .any(|candidate| candidate.trim() == text.trim())
    });
    // The token a press sends back names this line in this file (B8); a
    // line that cannot go in now says why before any press.
    let (preview, cannot) = match &global.read {
        Ok(current) if !present => {
            match crate::global_instruction::preview_against(current, &text) {
                Ok(preview) => (Some(preview.token()), None),
                Err(refusal) => (None, Some(LocalFailure::of(&refusal))),
            }
        }
        _ => (None, None),
    };
    Some(crate::model::SuggestionLocalSnapshot {
        line: text,
        preview,
        present,
        applied_at_unix_ms: applied.and_then(|local| local.applied_at_unix_ms),
        recurred: applied.is_some_and(|local| local.recurred_at_unix_ms.is_some())
            && line.stage == Stage::Suggested,
        failure: applied.and_then(|local| local.failure).or(cannot),
    })
}

/// What the store's index lacks about the evidence a read shows (B10).
#[derive(Debug, Default)]
pub struct EvidenceSources {
    /// The shown sessions whose record can still be opened; `None` when
    /// that could not be read, which leaves every session openable.
    pub kept: Option<BTreeSet<SessionRef>>,
    /// The address of each shown record's pull request.
    pub pr_urls: BTreeMap<RecordId, String>,
}

/// The records a line shows as its evidence: one per session or pull
/// request, newest first, at most [`policy::EVIDENCE_ROWS`].
fn shown<'a>(line: &Line, index: &'a RecordIndex) -> Vec<&'a RecordEntry> {
    let mut seen = BTreeSet::new();
    let mut shown = line
        .records
        .iter()
        .filter_map(|id| index.get(*id))
        .filter(|record| {
            let unit = record
                .session
                .as_ref()
                .map(|session| (session.agent.clone(), session.id.clone()))
                .or_else(|| record.pr.map(|pr| ("pr".to_owned(), pr.to_string())));
            seen.insert(unit)
        })
        .collect::<Vec<_>>();
    shown.sort_by_key(|record| std::cmp::Reverse(record.at_unix_ms));
    shown.truncate(policy::EVIDENCE_ROWS);
    shown
}

/// Every record a read of `notebook` shows as evidence, so its sources can
/// be asked before the read.
pub fn shown_evidence<'a>(
    notebook: &Notebook,
    lives: &BTreeMap<LineId, LineLife>,
    index: &'a RecordIndex,
) -> Vec<&'a RecordEntry> {
    let empty = LineLife::default();
    notebook
        .lines
        .iter()
        .filter(|line| group_and_state(line, lives.get(&line.id).unwrap_or(&empty)).is_some())
        .flat_map(|line| shown(line, index))
        .collect()
}

fn group_and_state(line: &Line, life: &LineLife) -> Option<(SuggestionGroup, SuggestionState)> {
    if line.struck.is_some() {
        return None;
    }
    Some(match line.stage {
        Stage::Candidate => return None,
        // Offered only with what its press changes; a line raised before
        // that was asked waits for the investigation (`lower_unwritten`).
        Stage::Suggested if !line.names_its_fix() => return None,
        Stage::Suggested => (SuggestionGroup::Choose, SuggestionState::New),
        Stage::Applying => match life.attempt.as_ref().map(|attempt| phase(attempt.phase)) {
            Some(SuggestionAttemptPhase::Starting | SuggestionAttemptPhase::Working) | None => {
                (SuggestionGroup::Applying, SuggestionState::HideAiWorking)
            }
            Some(phase) => (SuggestionGroup::of_attempt(phase), SuggestionState::MyTurn),
        },
        Stage::Watching => (SuggestionGroup::Applied, SuggestionState::Watching),
        Stage::Effective => (SuggestionGroup::Applied, SuggestionState::Effective),
        Stage::Settled => (SuggestionGroup::Applied, SuggestionState::Applied),
    })
}

/// An attempt's phase as the row draws it; a closed pull request reads as a
/// failed attempt.
fn phase(phase: AttemptPhase) -> SuggestionAttemptPhase {
    match phase {
        AttemptPhase::Starting => SuggestionAttemptPhase::Starting,
        AttemptPhase::Working => SuggestionAttemptPhase::Working,
        AttemptPhase::Ready => SuggestionAttemptPhase::Ready,
        AttemptPhase::PrOpen => SuggestionAttemptPhase::PrOpen,
        AttemptPhase::CiFailed => SuggestionAttemptPhase::CiFailed,
        AttemptPhase::Failed | AttemptPhase::Closed => SuggestionAttemptPhase::Failed,
        AttemptPhase::IssueOpen => SuggestionAttemptPhase::IssueOpen,
        AttemptPhase::Merged => SuggestionAttemptPhase::Merged,
    }
}

fn attempt(life: &LineLife) -> Option<SuggestionAttemptSnapshot> {
    let attempt = life.attempt.as_ref()?;
    Some(SuggestionAttemptSnapshot {
        phase: phase(attempt.phase),
        started_at_unix_ms: attempt.started_at_unix_ms,
        pr_number: attempt.pr.as_ref().map(|pr| pr.number),
        url: attempt
            .pr
            .as_ref()
            .map(|pr| pr.url.clone())
            .or_else(|| attempt.issue_url.clone()),
        agent_id: attempt.agent_id.clone(),
        agent_pane: None,
        failure: attempt.failure.clone(),
        branch: on_branch(attempt).then(|| branch(attempt)),
        purpose: attempt.purpose,
    })
}

/// Whether an attempt works on a branch of its own: every pull request
/// line's, never an issue's.
fn on_branch(attempt: &crate::suggestions::lifecycle::Attempt) -> bool {
    attempt.issue_title.is_none() && attempt.issue_url.is_none()
}

/// The attempt's branch as the row and its card draw it; the worktree it is
/// checked out in is laid by the runtime, which knows the node's facts.
pub(crate) fn branch(
    attempt: &crate::suggestions::lifecycle::Attempt,
) -> crate::model::SuggestionBranchSnapshot {
    let facts = attempt.branch_facts.as_ref();
    crate::model::SuggestionBranchSnapshot {
        name: attempt.branch.clone(),
        base: facts.map(|facts| facts.base.clone()),
        checkout: None,
        files: facts.map_or_else(Vec::new, |facts| {
            facts
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
        files_total: facts.map_or(0, |facts| facts.files_total),
        added: facts.map_or(0, |facts| facts.added),
        removed: facts.map_or(0, |facts| facts.removed),
        tip: facts.map(|facts| facts.tip.clone()),
        push: crate::model::SuggestionBranchPush::Unknown,
        base_unpushed: None,
        moved_to: None,
        base_unsaved: Vec::new(),
    }
}

fn evidence(line: &Line, input: &ReadInput<'_>) -> Vec<SuggestionEvidenceSnapshot> {
    let sources = input.sources;
    shown(line, input.index)
        .into_iter()
        .map(|record| SuggestionEvidenceSnapshot {
            kind: if record.session.is_some() {
                SuggestionEvidenceKind::Session
            } else {
                SuggestionEvidenceKind::Pr
            },
            signal: record.signal,
            improvement: record.improvement,
            agent: record.session.as_ref().map(|s| s.agent.clone()),
            session_id: record.session.as_ref().map(|s| s.id.clone()),
            pr_number: record.pr,
            url: record
                .pr
                .and_then(|_| sources.pr_urls.get(&record.id).cloned()),
            at_unix_ms: record.at_unix_ms,
            gone: record.session.as_ref().is_some_and(|session| {
                sources
                    .kept
                    .as_ref()
                    .is_some_and(|kept| !kept.contains(session))
            }),
            offset: record.said.as_ref().map(|said| said.offset),
            given: record.said.as_ref().and_then(|said| said.given),
            excerpt: None,
            unreadable: false,
        })
        .collect()
}

fn basis(line: &Line) -> Option<SuggestionBasisSnapshot> {
    line.basis
        .as_ref()
        .map(|(signal, tag)| SuggestionBasisSnapshot {
            signal: *signal,
            tag: tag.clone(),
        })
}

fn settings(
    settings: &Settings,
    notebook: &Notebook,
    lives: &BTreeMap<LineId, LineLife>,
) -> SuggestionSettingsSnapshot {
    let mut dismissed = notebook
        .lines
        .iter()
        .filter_map(|line| {
            let strike = line.struck.as_ref()?;
            (strike.by == StruckBy::Operator).then_some((strike.at_unix_ms, line))
        })
        .collect::<Vec<_>>();
    dismissed.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.id.cmp(&b.1.id)));
    SuggestionSettingsSnapshot {
        suggestions_enabled: settings.suggestions_enabled,
        quantity: settings.quantity,
        apply_mode: settings.apply_mode,
        signals: Signal::ALL
            .into_iter()
            .map(|signal| SuggestionSignalSnapshot {
                signal,
                enabled: settings.reads(signal),
                unavailable: None,
            })
            .collect(),
        dismissed: dismissed
            .into_iter()
            .take(super::policy::DISMISSED_ROWS)
            .map(|(_, line)| SuggestionDismissedSnapshot {
                id: row_id(line.id),
                discomfort: line.discomfort.clone(),
                basis: basis(line),
                may_return: lives.get(&line.id).is_some_and(LineLife::skip_may_return),
            })
            .collect(),
    }
}

/// Whether an automatic attempt nobody reviewed yet is being made or open,
/// which holds the next automatic apply (D-29).
pub fn unreviewed_auto_pr(notebook: &Notebook, lives: &BTreeMap<LineId, LineLife>) -> bool {
    notebook.lines.iter().any(|line| {
        line.stage == Stage::Applying
            && lives
                .get(&line.id)
                .and_then(|life| life.attempt.as_ref())
                .is_some_and(|attempt| attempt.holds_next_auto())
    })
}

/// The tab, as the worker reads it.
pub fn project(input: &ReadInput<'_>) -> ProjectSuggestionsSnapshot {
    let empty = LineLife::default();
    let auto_held = input.state.settings.apply_mode == ApplyMode::AutoPr
        && unreviewed_auto_pr(input.notebook, input.lives);
    let mut rows = input
        .notebook
        .lines
        .iter()
        .filter_map(|line| {
            let life = input.lives.get(&line.id).unwrap_or(&empty);
            let (group, state) = group_and_state(line, life)?;
            Some(SuggestionRowSnapshot {
                id: row_id(line.id),
                source: SuggestionSource::Notebook,
                group,
                state,
                discomfort: line.discomfort.clone(),
                cause: line.cause.clone(),
                fix: line.fix.clone(),
                plan: line.plan.clone(),
                outcome: line.outcome.unwrap_or(Outcome::Pr),
                basis: basis(line),
                auto: life.attempt.as_ref().is_some_and(|attempt| attempt.auto),
                hold: (state == SuggestionState::New && auto_held)
                    .then_some(SuggestionHold::AutoPrUnreviewed),
                returned_after_skip: state == SuggestionState::New && life.rose_after_skip(),
                escalated: life
                    .escalated_from
                    .as_ref()
                    .map(|pr| SuggestionEscalationSnapshot {
                        level: line.level,
                        previous_pr_url: Some(pr.url.clone()),
                    }),
                evidence: evidence(line, input),
                attempt: attempt(life),
                watch: life
                    .watch
                    .as_ref()
                    .filter(|_| line.stage == Stage::Watching)
                    .map(|watch| SuggestionWatchSnapshot {
                        sessions_seen: watch.sessions.len().min(policy::WATCH_SESSIONS),
                        sessions_needed: policy::WATCH_SESSIONS,
                    }),
                local: local(line, life, input.index, input.global),
                evidence_unconfirmed: false,
                memory_cards: Vec::new(),
                memory: None,
            })
        })
        .collect::<Vec<_>>();
    let updated = |id: &str| {
        line_of_row(id)
            .and_then(|line| input.notebook.line(line))
            .map_or(0, |line| line.updated_at_unix_ms)
    };
    rows.sort_by(|a, b| {
        a.drawing_rank()
            .cmp(&b.drawing_rank())
            .then(updated(&b.id).cmp(&updated(&a.id)))
            .then(a.id.cmp(&b.id))
    });
    let mut applied = 0;
    rows.retain(|row| {
        if row.group != SuggestionGroup::Applied {
            return true;
        }
        applied += 1;
        applied <= super::policy::APPLIED_ROWS
    });
    let settings_on = input.state.settings.active();
    let status = if !settings_on {
        SuggestionsStatus::Off
    } else if input.investigating {
        SuggestionsStatus::Investigating
    } else if input.state.last_completed_day.is_none() {
        SuggestionsStatus::FirstRunPending
    } else if input.state.uninvestigated {
        SuggestionsStatus::NotInvestigated
    } else if input.state.last_completed_day.as_deref() == Some(input.today)
        && !rows.iter().any(|row| row.group == SuggestionGroup::Choose)
    {
        SuggestionsStatus::NoneToday
    } else {
        SuggestionsStatus::Idle
    };
    ProjectSuggestionsSnapshot {
        workspace_id: input.workspace_id.to_owned(),
        loading: false,
        unavailable: None,
        enabled: input.state.settings.hide_ai_enabled,
        intro_seen: input.intro_seen,
        status,
        settings: settings(&input.state.settings, input.notebook, input.lives),
        rows,
        global: Some(crate::model::SuggestionGlobalSnapshot {
            path: input.global.path.to_string_lossy().into_owned(),
            unusable: input.global.read.as_ref().err().copied(),
        }),
        ..ProjectSuggestionsSnapshot::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_id_names_its_line_and_nothing_else() {
        assert_eq!(row_id(LineId(12)), "s12");
        assert_eq!(line_of_row("s12"), Some(LineId(12)));
        for other in ["m12", "s", "s1x", "12", "s-1"] {
            assert_eq!(line_of_row(other), None, "{other}");
        }
    }

    /// A ready branch, a failed attempt and failed checks wait for the
    /// operator's choice, so they sit under 고를 것 as the operator's turn;
    /// an attempt that runs, or waits on a review or an open issue, stays
    /// under 반영 중.
    #[test]
    fn an_attempt_that_waits_for_the_operators_choice_is_under_choose() {
        use crate::suggestions::ledger::Signal;
        use crate::suggestions::lifecycle::{Attempt, AttemptPurpose};
        let line = Line {
            id: LineId(4),
            discomfort: Some("the same check fails".into()),
            cause: None,
            fix: None,
            plan: None,
            outcome: None,
            basis: Some((Signal::CheckFailed, "verify".into())),
            records: Vec::new(),
            struck: None,
            stage: Stage::Applying,
            level: 0,
            created_at_unix_ms: 1,
            updated_at_unix_ms: 1,
        };
        let placed = |phase| {
            let life = LineLife {
                attempt: Some(Attempt {
                    started_at_unix_ms: 1,
                    auto: false,
                    phase,
                    branch: "hide-suggestion-s4".into(),
                    agent_name: "a".into(),
                    agent_id: None,
                    repository: None,
                    pr: None,
                    issue_url: None,
                    issue_title: None,
                    issue_since: None,
                    failure: None,
                    reviewed: false,
                    purpose: AttemptPurpose::Branch,
                    branch_facts: None,
                }),
                ..LineLife::default()
            };
            group_and_state(&line, &life).unwrap()
        };
        let (choose, applying) = (SuggestionGroup::Choose, SuggestionGroup::Applying);
        let (mine, working) = (SuggestionState::MyTurn, SuggestionState::HideAiWorking);
        for (phase, expected) in [
            (AttemptPhase::Working, (applying, working)),
            (AttemptPhase::Ready, (choose, mine)),
            (AttemptPhase::Failed, (choose, mine)),
            (AttemptPhase::Closed, (choose, mine)),
            (AttemptPhase::CiFailed, (choose, mine)),
            (AttemptPhase::PrOpen, (applying, mine)),
            (AttemptPhase::IssueOpen, (applying, mine)),
        ] {
            assert_eq!(placed(phase), expected, "{phase:?}");
        }
    }
}
