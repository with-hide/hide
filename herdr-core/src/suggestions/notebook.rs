//! The notebook (PRD hide-ai-suggestions D-32): one list of problems per
//! Project that each day's investigation goes on editing rather than writing
//! anew, so the same problem stays the same line across days and a watch
//! after a merge finds it again. The notebook is never shown (non-goal); its
//! lines become the suggestion tab's rows only through the lifecycle.
//!
//! [`Notebook::apply`] is the one place an edit is checked: whatever writes
//! the notebook, an investigation or the deterministic grouping, goes
//! through it, and the store saves exactly what it answers.

use super::ledger::{RecordId, RecordIndex, Signal};
use super::policy;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// A line's number inside its Project, never reused (`s12` on screen).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LineId(pub u64);

/// What a line's fix is delivered as (D-15, D-50): a pull request when the
/// place to fix is a file in the repository, an issue when the cause is
/// outside it or a CI problem's place is not known yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Pr,
    Issue,
}

/// Where a line stands for the operator; only the lifecycle moves it, an
/// edit never does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// A problem on the list, not raised.
    #[default]
    Candidate,
    /// Raised as a new suggestion.
    Suggested,
    /// Being applied, or its pull request or issue waits on the operator.
    Applying,
    /// Merged, and watched for a recurrence.
    Watching,
    /// The watch ended with no recurrence (`효과 있었음`).
    Effective,
    /// Applied and done with: the watch ended unknown, or nothing is watched.
    Settled,
}

impl Stage {
    /// Work is running on the line or it is being watched, so no edit may
    /// take it away.
    pub(crate) fn busy(self) -> bool {
        matches!(self, Self::Applying | Self::Watching)
    }

    /// The line's story is over; with a struck line, the first to make room.
    pub(crate) fn ended(self) -> bool {
        matches!(self, Self::Effective | Self::Settled)
    }
}

/// Who struck a line: the operator's `넘어가기`, which may bring it back
/// once and which ⚙ restores (D-43); an investigation's discard is not
/// offered back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StruckBy {
    Operator,
    Analysis,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Strike {
    pub by: StruckBy,
    pub reason: Option<String>,
    pub at_unix_ms: u64,
}

/// One problem.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub id: LineId,
    /// The discomfort in a person's words (`불편`).
    pub discomfort: Option<String>,
    /// What was missed, one sentence (`놓친 것`, D-05).
    pub cause: Option<String>,
    /// What to change.
    pub fix: Option<String>,
    pub outcome: Option<Outcome>,
    /// The signal and tag a line the deterministic grouping wrote rests on;
    /// the screen words a line with no text from it.
    pub basis: Option<(Signal, String)>,
    pub records: Vec<RecordId>,
    pub struck: Option<Strike>,
    pub stage: Stage,
    /// How strong the fix is (D-13): 0 a document, 1 a script or check,
    /// 2 a hook. A recurrence after a merge raises it by one.
    pub level: u8,
    pub created_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
}

impl Line {
    fn new(id: LineId, now: u64) -> Self {
        Self {
            id,
            discomfort: None,
            cause: None,
            fix: None,
            outcome: None,
            basis: None,
            records: Vec::new(),
            struck: None,
            stage: Stage::Candidate,
            level: 0,
            created_at_unix_ms: now,
            updated_at_unix_ms: now,
        }
    }

    /// The first of D-11's three questions: the line names one cause, in
    /// words or by the tag it rests on.
    pub(crate) fn has_cause(&self) -> bool {
        self.cause.is_some() || self.basis.is_some()
    }
}

/// A Project's notebook.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Notebook {
    pub lines: Vec<Line>,
    /// The number the next line takes.
    pub next_line: u64,
}

/// One change to the notebook (D-32): write a new line, attach records to
/// one, merge two, split one, strike one, write its cause; `Pass` says the
/// records were looked at and belong to no line, so the next run is not
/// handed them again.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "edit", rename_all = "snake_case")]
pub enum NotebookEdit {
    NewLine {
        discomfort: Option<String>,
        records: Vec<RecordId>,
        #[serde(default)]
        basis: Option<(Signal, String)>,
        #[serde(default)]
        outcome: Option<Outcome>,
    },
    Attach {
        line: LineId,
        records: Vec<RecordId>,
    },
    Merge {
        into: LineId,
        from: LineId,
    },
    Split {
        line: LineId,
        records: Vec<RecordId>,
        discomfort: Option<String>,
    },
    Strike {
        line: LineId,
        reason: Option<String>,
    },
    WriteCause {
        line: LineId,
        cause: String,
        fix: String,
        outcome: Outcome,
    },
    Pass {
        records: Vec<RecordId>,
    },
}

impl NotebookEdit {
    /// The edit's name in a diagnostic, which carries no text (B23).
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::NewLine { .. } => "new_line",
            Self::Attach { .. } => "attach",
            Self::Merge { .. } => "merge",
            Self::Split { .. } => "split",
            Self::Strike { .. } => "strike",
            Self::WriteCause { .. } => "write_cause",
            Self::Pass { .. } => "pass",
        }
    }
}

/// Why an edit was not taken; the notebook is left as it was.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "refused", rename_all = "snake_case")]
pub enum EditRefused {
    UnknownLine {
        line: LineId,
    },
    UnknownRecord {
        record: RecordId,
    },
    /// The records are not all on the line a split takes them from.
    NotOnLine {
        record: RecordId,
    },
    NoRecords,
    TooManyRecords,
    SameLine,
    /// Work is running on the line or it is watched.
    LineBusy {
        line: LineId,
    },
    EmptyText {
        field: &'static str,
    },
    TooLong {
        field: &'static str,
    },
    /// At the line cap with no struck or ended line to make room (D-46).
    NotebookFull,
}

/// What an edit changed, which is exactly what the store writes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Applied {
    /// A line the edit wrote; `NewLine` and `Split` answer theirs here.
    pub created: Option<LineId>,
    /// Lines whose content changed, the new one included.
    pub touched: BTreeSet<LineId>,
    /// Lines that are gone: merged away, or removed to make room.
    pub removed: BTreeSet<LineId>,
    /// Lines removed to make room (D-46), a subset of `removed`.
    pub evicted: Vec<LineId>,
    /// Records no longer handed to the next run.
    pub seen: BTreeSet<RecordId>,
}

fn text(
    value: Option<String>,
    field: &'static str,
    limit: usize,
) -> Result<Option<String>, EditRefused> {
    value.map(|value| required(value, field, limit)).transpose()
}

fn required(value: String, field: &'static str, limit: usize) -> Result<String, EditRefused> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(EditRefused::EmptyText { field });
    }
    if trimmed.chars().count() > limit {
        return Err(EditRefused::TooLong { field });
    }
    Ok(trimmed
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect())
}

impl Notebook {
    pub(crate) fn line(&self, id: LineId) -> Option<&Line> {
        self.lines.iter().find(|line| line.id == id)
    }

    fn line_mut(&mut self, id: LineId) -> Result<&mut Line, EditRefused> {
        self.lines
            .iter_mut()
            .find(|line| line.id == id)
            .ok_or(EditRefused::UnknownLine { line: id })
    }

    /// The line holding `record`, if any.
    #[cfg(test)]
    pub(crate) fn line_of(&self, record: RecordId) -> Option<LineId> {
        self.lines
            .iter()
            .find(|line| line.records.contains(&record))
            .map(|line| line.id)
    }

    fn checked_records(
        records: &[RecordId],
        index: &RecordIndex,
    ) -> Result<Vec<RecordId>, EditRefused> {
        if records.is_empty() {
            return Err(EditRefused::NoRecords);
        }
        if records.len() > policy::EDIT_RECORDS {
            return Err(EditRefused::TooManyRecords);
        }
        let mut unique = BTreeSet::new();
        for record in records {
            if index.get(*record).is_none() {
                return Err(EditRefused::UnknownRecord { record: *record });
            }
            unique.insert(*record);
        }
        Ok(unique.into_iter().collect())
    }

    /// Takes `records` off whichever lines hold them, noting those lines.
    fn detach(&mut self, records: &[RecordId], applied: &mut Applied) {
        for line in &mut self.lines {
            let before = line.records.len();
            line.records.retain(|record| !records.contains(record));
            if line.records.len() != before {
                applied.touched.insert(line.id);
            }
        }
    }

    /// Room for one more line: at the cap, the oldest struck or ended line
    /// that no work holds goes first, then the oldest line that never rose
    /// (D-46); a notebook of raised and busy lines alone refuses.
    fn make_room(&mut self, applied: &mut Applied) -> Result<(), EditRefused> {
        if self.lines.len() < policy::NOTEBOOK_LINE_CAP {
            return Ok(());
        }
        let oldest = |pick: fn(&Line) -> bool| {
            self.lines
                .iter()
                .filter(|line| !line.stage.busy() && pick(line))
                .min_by_key(|line| (line.updated_at_unix_ms, line.id))
                .map(|line| line.id)
        };
        let oldest = oldest(|line| line.struck.is_some() || line.stage.ended())
            .or_else(|| oldest(|line| line.stage == Stage::Candidate))
            .ok_or(EditRefused::NotebookFull)?;
        let at = self
            .lines
            .iter()
            .position(|line| line.id == oldest)
            .expect("the oldest line was just found");
        let line = self.lines.remove(at);
        applied.seen.extend(line.records);
        applied.touched.remove(&oldest);
        applied.removed.insert(oldest);
        applied.evicted.push(oldest);
        Ok(())
    }

    fn new_line(&mut self, now: u64, applied: &mut Applied) -> Result<LineId, EditRefused> {
        self.make_room(applied)?;
        self.next_line = self.next_line.max(1);
        let id = LineId(self.next_line);
        self.next_line += 1;
        self.lines.push(Line::new(id, now));
        applied.created = Some(id);
        applied.touched.insert(id);
        Ok(id)
    }

    /// Checks `edit` against the notebook and `index` and makes it, or
    /// leaves the notebook as it was and says why.
    pub fn apply(
        &mut self,
        edit: NotebookEdit,
        index: &RecordIndex,
        now: u64,
    ) -> Result<Applied, EditRefused> {
        let mut next = self.clone();
        let applied = next.make(edit, index, now)?;
        *self = next;
        Ok(applied)
    }

    fn make(
        &mut self,
        edit: NotebookEdit,
        index: &RecordIndex,
        now: u64,
    ) -> Result<Applied, EditRefused> {
        let mut applied = Applied::default();
        match edit {
            NotebookEdit::NewLine {
                discomfort,
                records,
                basis,
                outcome,
            } => {
                let discomfort = text(discomfort, "discomfort", policy::DISCOMFORT_CHARS)?;
                let records = Self::checked_records(&records, index)?;
                let basis = basis
                    .map(|(signal, tag)| {
                        crate::suggestions::ledger::normalize_tag(&tag)
                            .map(|tag| (signal, tag))
                            .ok_or(EditRefused::EmptyText { field: "basis" })
                    })
                    .transpose()?;
                self.detach(&records, &mut applied);
                let id = self.new_line(now, &mut applied)?;
                let line = self.line_mut(id)?;
                line.discomfort = discomfort;
                line.basis = basis;
                line.outcome = outcome;
                line.records = records.clone();
                applied.seen.extend(records);
            }
            NotebookEdit::Attach { line, records } => {
                let records = Self::checked_records(&records, index)?;
                self.line_mut(line)?;
                self.detach(&records, &mut applied);
                let target = self.line_mut(line)?;
                target.records.extend(records.iter().copied());
                target.records.sort();
                target.updated_at_unix_ms = now;
                applied.touched.insert(line);
                applied.seen.extend(records);
            }
            NotebookEdit::Merge { into, from } => {
                if into == from {
                    return Err(EditRefused::SameLine);
                }
                self.line_mut(into)?;
                let source = self.line_mut(from)?;
                if source.stage.busy() {
                    return Err(EditRefused::LineBusy { line: from });
                }
                let moved = std::mem::take(&mut source.records);
                self.lines.retain(|line| line.id != from);
                let target = self.line_mut(into)?;
                target.records.extend(moved);
                target.records.sort();
                target.records.dedup();
                target.updated_at_unix_ms = now;
                applied.touched.insert(into);
                applied.removed.insert(from);
            }
            NotebookEdit::Split {
                line,
                records,
                discomfort,
            } => {
                let discomfort = text(discomfort, "discomfort", policy::DISCOMFORT_CHARS)?;
                let records = Self::checked_records(&records, index)?;
                let source = self.line_mut(line)?;
                if let Some(record) = records.iter().find(|r| !source.records.contains(r)) {
                    return Err(EditRefused::NotOnLine { record: *record });
                }
                source.records.retain(|record| !records.contains(record));
                source.updated_at_unix_ms = now;
                applied.touched.insert(line);
                let id = self.new_line(now, &mut applied)?;
                let created = self.line_mut(id)?;
                created.discomfort = discomfort;
                created.records = records.clone();
                applied.seen.extend(records);
            }
            NotebookEdit::Strike { line, reason } => {
                let reason = text(reason, "reason", policy::REASON_CHARS)?;
                let target = self.line_mut(line)?;
                if target.stage == Stage::Applying {
                    return Err(EditRefused::LineBusy { line });
                }
                target.struck = Some(Strike {
                    by: StruckBy::Analysis,
                    reason,
                    at_unix_ms: now,
                });
                if target.stage == Stage::Suggested {
                    target.stage = Stage::Candidate;
                }
                target.updated_at_unix_ms = now;
                applied.touched.insert(line);
            }
            NotebookEdit::WriteCause {
                line,
                cause,
                fix,
                outcome,
            } => {
                let cause = required(cause, "cause", policy::CAUSE_CHARS)?;
                let fix = required(fix, "fix", policy::FIX_CHARS)?;
                let target = self.line_mut(line)?;
                target.cause = Some(cause);
                target.fix = Some(fix);
                target.outcome = Some(outcome);
                target.updated_at_unix_ms = now;
                applied.touched.insert(line);
            }
            NotebookEdit::Pass { records } => {
                let records = Self::checked_records(&records, index)?;
                applied.seen.extend(records);
            }
        }
        Ok(applied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suggestions::ledger::RecordEntry;

    fn index(ids: impl IntoIterator<Item = u64>) -> RecordIndex {
        RecordIndex {
            entries: ids
                .into_iter()
                .map(|id| RecordEntry {
                    id: RecordId(id),
                    signal: Signal::FailedTool,
                    tag: "cargo test".into(),
                    session: None,
                    pr: None,
                    at_unix_ms: 1,
                    always: false,
                    improvement: None,
                    line: None,
                    fresh: true,
                })
                .collect(),
        }
    }

    fn new_line(records: &[u64]) -> NotebookEdit {
        NotebookEdit::NewLine {
            discomfort: Some("tests fail on a clean checkout".into()),
            records: records.iter().copied().map(RecordId).collect(),
            basis: None,
            outcome: None,
        }
    }

    #[test]
    fn lines_are_numbered_once_and_records_move_between_them() {
        let index = index(1..=4);
        let mut notebook = Notebook::default();
        let first = notebook.apply(new_line(&[1, 2]), &index, 10).unwrap();
        assert_eq!(first.created, Some(LineId(1)));
        let second = notebook.apply(new_line(&[2, 3]), &index, 11).unwrap();
        assert_eq!(second.created, Some(LineId(2)));
        assert!(second.touched.contains(&LineId(1)), "record 2 left line 1");
        assert_eq!(notebook.line(LineId(1)).unwrap().records, [RecordId(1)]);
        notebook
            .apply(
                NotebookEdit::Merge {
                    into: LineId(1),
                    from: LineId(2),
                },
                &index,
                12,
            )
            .unwrap();
        assert_eq!(
            notebook.line(LineId(1)).unwrap().records,
            [RecordId(1), RecordId(2), RecordId(3)]
        );
        assert!(notebook.line(LineId(2)).is_none());
        let split = notebook
            .apply(
                NotebookEdit::Split {
                    line: LineId(1),
                    records: vec![RecordId(3)],
                    discomfort: None,
                },
                &index,
                13,
            )
            .unwrap();
        assert_eq!(
            split.created,
            Some(LineId(3)),
            "a merged-away number is not reused"
        );
        assert_eq!(notebook.line_of(RecordId(3)), Some(LineId(3)));
    }

    #[test]
    fn a_refused_edit_leaves_the_notebook_as_it_was() {
        let index = index(1..=2);
        let mut notebook = Notebook::default();
        notebook.apply(new_line(&[1]), &index, 1).unwrap();
        let before = notebook.clone();
        for (edit, refusal) in [
            (
                new_line(&[9]),
                EditRefused::UnknownRecord {
                    record: RecordId(9),
                },
            ),
            (new_line(&[]), EditRefused::NoRecords),
            (
                NotebookEdit::Attach {
                    line: LineId(7),
                    records: vec![RecordId(2)],
                },
                EditRefused::UnknownLine { line: LineId(7) },
            ),
            (
                NotebookEdit::Split {
                    line: LineId(1),
                    records: vec![RecordId(2)],
                    discomfort: None,
                },
                EditRefused::NotOnLine {
                    record: RecordId(2),
                },
            ),
            (
                NotebookEdit::WriteCause {
                    line: LineId(1),
                    cause: " ".into(),
                    fix: "x".into(),
                    outcome: Outcome::Pr,
                },
                EditRefused::EmptyText { field: "cause" },
            ),
            (
                NotebookEdit::WriteCause {
                    line: LineId(1),
                    cause: "x".repeat(policy::CAUSE_CHARS + 1),
                    fix: "x".into(),
                    outcome: Outcome::Pr,
                },
                EditRefused::TooLong { field: "cause" },
            ),
            (
                NotebookEdit::Merge {
                    into: LineId(1),
                    from: LineId(1),
                },
                EditRefused::SameLine,
            ),
        ] {
            assert_eq!(notebook.apply(edit, &index, 2), Err(refusal));
            assert_eq!(notebook, before);
        }
    }

    #[test]
    fn work_on_a_line_keeps_it_from_being_merged_away_or_struck() {
        let index = index(1..=2);
        let mut notebook = Notebook::default();
        notebook.apply(new_line(&[1]), &index, 1).unwrap();
        notebook.apply(new_line(&[2]), &index, 1).unwrap();
        notebook.lines[1].stage = Stage::Applying;
        assert_eq!(
            notebook.apply(
                NotebookEdit::Merge {
                    into: LineId(1),
                    from: LineId(2)
                },
                &index,
                2
            ),
            Err(EditRefused::LineBusy { line: LineId(2) })
        );
        assert_eq!(
            notebook.apply(
                NotebookEdit::Strike {
                    line: LineId(2),
                    reason: None
                },
                &index,
                2
            ),
            Err(EditRefused::LineBusy { line: LineId(2) })
        );
        // A watched line still takes a recurrence.
        notebook.lines[1].stage = Stage::Watching;
        notebook
            .apply(
                NotebookEdit::Attach {
                    line: LineId(2),
                    records: vec![RecordId(1)],
                },
                &index,
                3,
            )
            .unwrap();
        assert_eq!(notebook.line_of(RecordId(1)), Some(LineId(2)));
    }

    /// D-46: at the cap the oldest struck or ended line goes first, then
    /// the oldest line that never rose; raised and busy lines stay, and a
    /// notebook of only those refuses.
    #[test]
    fn a_full_notebook_drops_ended_lines_first_then_the_oldest_candidate() {
        let index = index(1..=1);
        let mut notebook = Notebook::default();
        for at in 0..policy::NOTEBOOK_LINE_CAP as u64 {
            notebook.apply(new_line(&[1]), &index, at).unwrap();
        }
        notebook.lines[5].struck = Some(Strike {
            by: StruckBy::Analysis,
            reason: None,
            at_unix_ms: 0,
        });
        notebook.lines[9].stage = Stage::Effective;
        notebook.lines[9].updated_at_unix_ms = 7;
        notebook.lines[0].stage = Stage::Suggested;
        notebook.lines[1].stage = Stage::Applying;
        let mut evicted = Vec::new();
        for at in 500..503 {
            let applied = notebook.apply(new_line(&[1]), &index, at).unwrap();
            evicted.extend(applied.evicted);
            assert_eq!(notebook.lines.len(), policy::NOTEBOOK_LINE_CAP);
        }
        assert_eq!(
            evicted,
            [LineId(6), LineId(10), LineId(3)],
            "struck, then ended, then the oldest line that never rose"
        );
        for line in &mut notebook.lines {
            line.stage = Stage::Suggested;
        }
        assert_eq!(
            notebook.apply(new_line(&[1]), &index, 600),
            Err(EditRefused::NotebookFull)
        );
    }
}
