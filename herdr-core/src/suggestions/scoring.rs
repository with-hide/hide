//! How a line's pain is counted and which lines are raised (PRD
//! hide-ai-suggestions D-11, D-12, D-42, B9). The core counts, never a
//! model: a line's score is read off the records attached to it, so the
//! numbers can be checked and a replay of past records gives the same ones.

use super::ledger::{RecordEntry, RecordIndex, Signal};
use super::lifecycle::Evidence;
use super::notebook::{Line, LineId};
use super::policy::{self, Quantity};
use std::collections::BTreeMap;

/// A line's counted pain.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Score {
    pub points: f64,
    pub sessions: usize,
    pub prs: usize,
    /// One of its corrections said "from now on" (D-11).
    pub always: bool,
}

/// Counts `line` over its records at `now` (D-12): a distinct session one
/// point, three when the operator corrected an agent in it, a half when the
/// session's only records here are a label's remaining problems,
/// [`policy::IMPROVEMENT_SESSION_POINTS`] when they are only improvements; a distinct
/// pull request one point while within [`policy::PR_WINDOW_MS`]; each unit
/// weighed by the age of its newest record.
pub fn score(line: &Line, index: &RecordIndex, now: u64) -> Score {
    let records = line.records.iter().filter_map(|id| index.get(*id));
    score_records(records, now)
}

pub(crate) fn score_records<'a>(records: impl Iterator<Item = &'a RecordEntry>, now: u64) -> Score {
    let mut sessions: BTreeMap<(&str, &str), Vec<&RecordEntry>> = BTreeMap::new();
    let mut prs: BTreeMap<u32, u64> = BTreeMap::new();
    let mut always = false;
    for record in records {
        always |= record.always && record.signal == Signal::Correction;
        if let Some(session) = &record.session {
            sessions
                .entry((session.agent.as_str(), session.id.as_str()))
                .or_default()
                .push(record);
        } else if let Some(pr) = record.pr {
            let newest = prs.entry(pr).or_default();
            *newest = (*newest).max(record.at_unix_ms);
        }
    }
    let age = |at: u64| now.saturating_sub(at);
    let mut points = 0.0;
    for records in sessions.values() {
        let unit = if records.iter().any(|r| r.signal == Signal::Correction) {
            policy::CORRECTION_SESSION_POINTS
        } else if records.iter().all(|r| r.signal == Signal::LabelMissed) {
            policy::LABEL_ONLY_SESSION_POINTS
        } else if records.iter().all(|r| r.signal == Signal::Improvement) {
            policy::IMPROVEMENT_SESSION_POINTS
        } else {
            policy::SESSION_POINTS
        };
        let newest = records.iter().map(|r| r.at_unix_ms).max().unwrap_or(0);
        points += unit * policy::decay(age(newest));
    }
    let mut counted_prs = 0;
    for newest in prs.values() {
        if age(*newest) <= policy::PR_WINDOW_MS {
            points += policy::PR_POINTS * policy::decay(age(*newest));
            counted_prs += 1;
        }
    }
    Score {
        points,
        sessions: sessions.len(),
        prs: counted_prs,
        always,
    }
}

/// The sessions and pull requests behind `line` among the records `keep`
/// takes: the units [`score`] counts.
pub(crate) fn evidence(
    line: &Line,
    index: &RecordIndex,
    keep: impl Fn(&RecordEntry) -> bool,
) -> Evidence {
    let mut evidence = Evidence::default();
    for record in line.records.iter().filter_map(|id| index.get(*id)) {
        if !keep(record) {
            continue;
        }
        if let Some(session) = &record.session {
            evidence.sessions.insert(session.clone());
        } else if let Some(pr) = record.pr {
            evidence.prs.insert(pr);
        }
    }
    evidence
}

/// Whether a line the operator skipped at `skipped_at`, standing on
/// `skipped_with`, may rise once more (D-43): the sessions and pull requests
/// seen after the skip that it did not stand on reach
/// [`policy::SKIP_RETURN_SHARE`] of those it did. A session that goes on
/// past the skip, or a pull request failing again on a newer head, is not
/// new.
pub(crate) fn outgrew_skip(
    line: &Line,
    index: &RecordIndex,
    skipped_at: u64,
    skipped_with: &Evidence,
) -> bool {
    let since = evidence(line, index, |record| record.at_unix_ms > skipped_at);
    let new = since.sessions.difference(&skipped_with.sessions).count()
        + since.prs.difference(&skipped_with.prs).count();
    new as f64 >= (skipped_with.units() as f64 * policy::SKIP_RETURN_SHARE).max(1.0)
}

/// D-11's three questions, as the core can answer them: the line names one
/// cause, it actually hurt (something counted), and it is one pull request
/// or one issue.
pub fn answers_the_questions(line: &Line, score: &Score) -> bool {
    line.struck.is_none() && line.has_cause() && line.outcome.is_some() && score.points > 0.0
}

/// Whether a line has risen to a suggestion at `quantity`: the questions
/// answered and the threshold reached, or one "from now on" correction.
pub fn rises(line: &Line, score: &Score, quantity: Quantity) -> bool {
    answers_the_questions(line, score) && (score.always || score.points >= quantity.threshold())
}

/// The lines to raise now (D-10, D-42), strongest first: as many as the open
/// cap, the stored cap and the week's cap leave, of the lines that rose. A
/// Normal Project that raised nothing for [`policy::QUIET_SPAN_MS`] since
/// `quiet_since` (its first look at what rose, then its last raise) raises
/// its strongest line that answers the questions even under the threshold.
pub fn choose(
    candidates: &[(&Line, Score)],
    open: usize,
    raised_at: &[u64],
    quiet_since: u64,
    quantity: Quantity,
    now: u64,
) -> Vec<LineId> {
    let week = raised_at
        .iter()
        .filter(|at| now.saturating_sub(**at) < policy::WEEK_MS)
        .count();
    let slots = quantity
        .open_cap()
        .min(policy::STORED_NEW_CAP)
        .saturating_sub(open)
        .min(quantity.weekly_cap().saturating_sub(week));
    if slots == 0 {
        return Vec::new();
    }
    let mut ranked = candidates
        .iter()
        .filter(|(line, score)| answers_the_questions(line, score))
        .collect::<Vec<_>>();
    ranked.sort_by(|(a, a_score), (b, b_score)| {
        b_score
            .always
            .cmp(&a_score.always)
            .then(b_score.points.total_cmp(&a_score.points))
            .then(a.id.cmp(&b.id))
    });
    let risen = ranked
        .iter()
        .filter(|(line, score)| rises(line, score, quantity))
        .take(slots)
        .map(|(line, _)| line.id)
        .collect::<Vec<_>>();
    let quiet = now.saturating_sub(quiet_since) >= policy::QUIET_SPAN_MS;
    if risen.is_empty() && open == 0 && quiet && quantity.fills_a_quiet_fortnight() {
        return ranked
            .first()
            .map(|(line, _)| line.id)
            .into_iter()
            .collect();
    }
    risen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suggestions::ledger::{RecordId, SessionRef};
    use crate::suggestions::notebook::{Notebook, NotebookEdit, Outcome};
    use crate::suggestions::policy::DAY_MS;

    const NOW: u64 = 1_000 * DAY_MS;

    fn entry(
        id: u64,
        signal: Signal,
        session: Option<&str>,
        pr: Option<u32>,
        at: u64,
    ) -> RecordEntry {
        RecordEntry {
            id: RecordId(id),
            signal,
            tag: "t".into(),
            session: session.map(|id| SessionRef {
                agent: "codex".into(),
                id: id.into(),
            }),
            pr,
            at_unix_ms: at,
            always: false,
            improvement: None,
            line: None,
            fresh: true,
        }
    }

    fn line_of(index: &RecordIndex) -> Line {
        let mut notebook = Notebook::default();
        let applied = notebook
            .apply(
                NotebookEdit::NewLine {
                    discomfort: None,
                    records: index.entries.iter().map(|e| e.id).collect(),
                    basis: Some((Signal::FailedTool, "cargo test".into())),
                    outcome: Some(Outcome::Pr),
                },
                index,
                NOW,
            )
            .unwrap();
        notebook.line(applied.created.unwrap()).unwrap().clone()
    }

    /// B25: improvements alone take as many sessions as any other session
    /// signal: two do not cross Normal's threshold, three do.
    #[test]
    fn improvements_rise_only_once_three_sessions_hold_them() {
        let points = |sessions: &[&str]| {
            let index = RecordIndex {
                entries: sessions
                    .iter()
                    .enumerate()
                    .map(|(n, session)| {
                        entry(n as u64 + 1, Signal::Improvement, Some(session), None, NOW)
                    })
                    .collect(),
            };
            score(&line_of(&index), &index, NOW).points
        };
        let normal = Quantity::Normal.threshold();
        assert!(points(&["a", "a", "b"]) < normal, "one session counts once");
        assert!(points(&["a", "b", "c"]) >= normal);
    }

    #[test]
    fn a_line_counts_sessions_corrections_label_halves_and_recent_pull_requests() {
        let index = RecordIndex {
            entries: vec![
                // Two failures in one session count once.
                entry(1, Signal::FailedTool, Some("a"), None, NOW),
                entry(2, Signal::FailedTool, Some("a"), None, NOW),
                // A session with a correction counts three.
                entry(3, Signal::Correction, Some("b"), None, NOW),
                entry(4, Signal::FailedTool, Some("b"), None, NOW),
                // A session with only a label's leftover counts a half.
                entry(5, Signal::LabelMissed, Some("c"), None, NOW),
                // A pull request within fourteen days counts one; an older one none.
                entry(6, Signal::CheckFailed, None, Some(7), NOW),
                entry(7, Signal::CheckFailed, None, Some(7), NOW),
                entry(8, Signal::CheckFailed, None, Some(8), NOW - 15 * DAY_MS),
            ],
        };
        let score = score(&line_of(&index), &index, NOW);
        assert_eq!(score.sessions, 3);
        assert_eq!(score.prs, 1);
        assert!(
            (score.points - (1.0 + 3.0 + 0.5 + 1.0)).abs() < 1e-9,
            "{score:?}"
        );
        assert!(!score.always);
    }

    /// The evidence a line stood on at `skipped`, as the skip keeps it.
    fn skipped_with(index: &RecordIndex, skipped: u64) -> Evidence {
        evidence(&line_of(index), index, |record| {
            record.at_unix_ms <= skipped
        })
    }

    /// D-43: a skip is outgrown by as many new sessions and pull requests
    /// as the line had when it was skipped; one that goes on past the skip
    /// is not new.
    #[test]
    fn a_skip_is_outgrown_when_new_sessions_and_pull_requests_double_its_evidence() {
        let skipped = NOW - DAY_MS;
        let mut index = RecordIndex {
            entries: vec![
                entry(1, Signal::FailedTool, Some("a"), None, skipped - 1),
                entry(2, Signal::FailedTool, Some("b"), None, skipped),
                entry(3, Signal::CheckFailed, None, Some(7), skipped - 1),
            ],
        };
        let kept = skipped_with(&index, skipped);
        assert_eq!(kept.units(), 3);
        index.entries.extend([
            entry(4, Signal::FailedTool, Some("a"), None, NOW),
            entry(5, Signal::CheckFailed, None, Some(7), NOW),
            entry(6, Signal::FailedTool, Some("c"), None, NOW),
            entry(7, Signal::CheckFailed, None, Some(9), NOW),
        ]);
        assert!(
            !outgrew_skip(&line_of(&index), &index, skipped, &kept),
            "c and #9 are new, a and #7 went on: two of three"
        );
        index
            .entries
            .push(entry(8, Signal::FailedTool, Some("d"), None, NOW));
        assert!(outgrew_skip(&line_of(&index), &index, skipped, &kept));
    }

    /// D-43: the ledger moves a pull request's record to its newer head and
    /// time, so a pull request that fails again after the skip has a record
    /// past it; it is still one the line stood on.
    #[test]
    fn a_pull_request_failing_again_on_a_newer_head_is_not_new() {
        let skipped = NOW - DAY_MS;
        let before = RecordIndex {
            entries: vec![
                entry(1, Signal::CheckFailed, None, Some(1), skipped - 1),
                entry(2, Signal::CheckFailed, None, Some(2), skipped - 1),
                entry(3, Signal::CheckFailed, None, Some(3), skipped - 1),
            ],
        };
        let kept = skipped_with(&before, skipped);
        let mut moved = RecordIndex {
            entries: vec![
                entry(1, Signal::CheckFailed, None, Some(1), NOW),
                entry(2, Signal::CheckFailed, None, Some(2), NOW),
                entry(3, Signal::CheckFailed, None, Some(3), skipped - 1),
            ],
        };
        assert!(
            !outgrew_skip(&line_of(&moved), &moved, skipped, &kept),
            "#1 and #2 failed again: no new pull request"
        );
        moved.entries.extend([
            entry(4, Signal::CheckFailed, None, Some(4), NOW),
            entry(5, Signal::CheckFailed, None, Some(5), NOW),
            entry(6, Signal::CheckFailed, None, Some(6), NOW),
        ]);
        assert!(outgrew_skip(&line_of(&moved), &moved, skipped, &kept));
    }

    /// D-43: records leave the ledger with age, and the evidence kept at
    /// the skip does not leave with them.
    #[test]
    fn a_skips_evidence_holds_after_its_records_are_pruned() {
        let skipped = NOW - DAY_MS;
        let before = RecordIndex {
            entries: vec![
                entry(1, Signal::FailedTool, Some("a"), None, skipped - 1),
                entry(2, Signal::FailedTool, Some("b"), None, skipped - 1),
                entry(3, Signal::FailedTool, Some("c"), None, skipped - 1),
            ],
        };
        let kept = skipped_with(&before, skipped);
        let mut pruned = RecordIndex {
            entries: vec![entry(4, Signal::Correction, Some("d"), None, NOW)],
        };
        assert!(
            !outgrew_skip(&line_of(&pruned), &pruned, skipped, &kept),
            "one new session of three, though nothing from before is left"
        );
        pruned.entries.extend([
            entry(5, Signal::FailedTool, Some("e"), None, NOW),
            entry(6, Signal::FailedTool, Some("f"), None, NOW),
        ]);
        assert!(outgrew_skip(&line_of(&pruned), &pruned, skipped, &kept));
    }

    #[test]
    fn an_older_session_weighs_less() {
        let index = RecordIndex {
            entries: vec![entry(
                1,
                Signal::FailedTool,
                Some("a"),
                None,
                NOW - policy::FULL_WEIGHT_MS - policy::DECAY_HALF_LIFE_MS,
            )],
        };
        let score = score(&line_of(&index), &index, NOW);
        assert!((score.points - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_line_rises_at_its_quantitys_threshold_or_on_one_always() {
        let two = RecordIndex {
            entries: vec![
                entry(1, Signal::FailedTool, Some("a"), None, NOW),
                entry(2, Signal::FailedTool, Some("b"), None, NOW),
            ],
        };
        let line = line_of(&two);
        let counted = score(&line, &two, NOW);
        assert!(rises(&line, &counted, Quantity::Many));
        assert!(!rises(&line, &counted, Quantity::Normal));
        assert!(!rises(&line, &counted, Quantity::Few));

        let mut always = entry(3, Signal::Correction, Some("a"), None, NOW);
        always.always = true;
        let once = RecordIndex {
            entries: vec![always],
        };
        let line = line_of(&once);
        let score = score(&line, &once, NOW);
        assert!(score.always);
        assert!(rises(&line, &score, Quantity::Few));

        let mut uncaused = line.clone();
        uncaused.basis = None;
        assert!(
            !rises(&uncaused, &score, Quantity::Few),
            "no cause, no suggestion"
        );
        let mut no_outcome = line.clone();
        no_outcome.outcome = None;
        assert!(!rises(&no_outcome, &score, Quantity::Few));
    }

    fn scored(id: u64, points: f64) -> (Line, Score) {
        let mut line = line_of(&RecordIndex {
            entries: vec![entry(1, Signal::FailedTool, Some("a"), None, NOW)],
        });
        line.id = LineId(id);
        (
            line,
            Score {
                points,
                sessions: 1,
                prs: 0,
                always: false,
            },
        )
    }

    #[test]
    fn the_open_cap_and_the_weeks_cap_bound_what_is_raised_strongest_first() {
        let lines = [
            scored(1, 3.0),
            scored(2, 9.0),
            scored(3, 5.0),
            scored(4, 4.0),
        ];
        let candidates = lines.iter().map(|(l, s)| (l, *s)).collect::<Vec<_>>();
        assert_eq!(
            choose(&candidates, 0, &[], NOW, Quantity::Normal, NOW),
            [LineId(2), LineId(3), LineId(4)]
        );
        assert_eq!(
            choose(&candidates, 0, &[], NOW, Quantity::Few, NOW),
            [LineId(2)]
        );
        assert_eq!(
            choose(&candidates, 2, &[], NOW, Quantity::Normal, NOW),
            [LineId(2)],
            "two already open leave one of three"
        );
        assert_eq!(
            choose(
                &candidates,
                0,
                &[NOW - DAY_MS, NOW - 2 * DAY_MS],
                NOW - DAY_MS,
                Quantity::Normal,
                NOW
            ),
            [LineId(2)],
            "two raised this week leave one of the week's three"
        );
        assert_eq!(
            choose(
                &candidates,
                0,
                &[NOW - 8 * DAY_MS; 3],
                NOW - 8 * DAY_MS,
                Quantity::Normal,
                NOW
            )
            .len(),
            3,
            "last week's raises do not count"
        );
    }

    /// D-10: Normal raises its strongest line under the threshold only once
    /// fourteen days passed since it began looking or last raised: not on
    /// day 0 or day 13, on day 14; never for Few, never with one open.
    #[test]
    fn a_quiet_fortnight_raises_normals_strongest_line_under_the_threshold() {
        let lines = [scored(1, 1.0), scored(2, 2.5)];
        let candidates = lines.iter().map(|(l, s)| (l, *s)).collect::<Vec<_>>();
        let quiet_for = |days: u64, quantity, open| {
            choose(&candidates, open, &[], NOW - days * DAY_MS, quantity, NOW)
        };
        assert!(quiet_for(0, Quantity::Normal, 0).is_empty(), "day 0");
        assert!(quiet_for(13, Quantity::Normal, 0).is_empty(), "day 13");
        assert_eq!(quiet_for(14, Quantity::Normal, 0), [LineId(2)], "day 14");
        assert!(quiet_for(14, Quantity::Few, 0).is_empty());
        assert!(quiet_for(14, Quantity::Normal, 1).is_empty());
    }
}
