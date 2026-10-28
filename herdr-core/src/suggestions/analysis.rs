//! The one boundary behind which problems are found and suggestions are
//! made (PRD hide-ai-suggestions D-49). Its input is the record index and
//! the notebook; its output is a sequence of notebook edits, each checked
//! and saved before the next is taken, so a run cut short keeps what it
//! wrote (D-35). How a run decides its edits, with a model and read tools or
//! with none, is the implementation's alone: the ledger, the scores, the
//! suggestion lifecycle, apply, the watch and the screen read only the
//! notebook it leaves.

use super::ledger::{RecordId, RecordIndex, Signal};
use super::notebook::{EditRefused, LineId, Notebook, NotebookEdit, Outcome};
use std::collections::BTreeMap;
use std::time::Instant;

/// What one run may spend (D-36): turns and wall time, and the flag that
/// ends it early, set when the Project is turned off or the core stops. An
/// implementation that waits on anything checks `stop` and returns soon
/// after it is set. The deterministic grouping spends none of it.
#[derive(Clone, Debug)]
pub struct AnalysisBudget {
    pub max_turns: u32,
    pub deadline: Instant,
    pub stop: StopFlag,
}

/// A run's stop: set once, read from any thread. It is a provider cancel
/// token, so a stop also ends a provider turn in flight.
#[derive(Clone, Debug, Default)]
pub struct StopFlag(hide_ai::CancelToken);

impl StopFlag {
    pub fn set(&self) {
        self.0.cancel();
    }

    pub fn is_set(&self) -> bool {
        self.0.is_cancelled()
    }

    /// The token a provider call made for this run is given.
    pub fn token(&self) -> hide_ai::CancelToken {
        self.0.clone()
    }
}

/// What a run is handed (D-32): the Project, the records with their counts
/// and the notebook as it stands. No session text, file or log is in it; an
/// investigation reads those through the read tools it is given.
#[derive(Clone, Debug)]
pub struct AnalysisInput {
    /// The Project identity (`hide_project`), which its read tools are
    /// scoped to.
    pub project_id: String,
    pub records: RecordIndex,
    pub language: hide_ai::OutputLanguage,
    pub budget: AnalysisBudget,
}

/// What an accepted edit answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Written {
    /// The line a `NewLine` or `Split` wrote.
    Line(LineId),
    Done,
}

/// The pen a run writes the notebook with. `write` checks the edit, saves it
/// to the store and only then answers, so a refused or failed edit changes
/// nothing and an accepted one survives whatever happens next.
pub trait NotebookPen {
    fn notebook(&self) -> &Notebook;
    fn write(&mut self, edit: NotebookEdit) -> Result<Written, PenRefused>;
}

/// Why the pen took no edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PenRefused {
    /// The notebook refused it; the run may go on with other edits.
    Edit(EditRefused),
    /// The edit could not be saved (`code` names the store failure); the
    /// run should end, since nothing more it writes can be kept.
    Store { code: String },
    /// The Project's suggestions were turned off or the core is stopping.
    Stopped,
}

/// Why a run ended before it was done. Records it did not attach or pass
/// stay fresh and are handed to the next day's run (D-35).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartialReason {
    TurnLimit,
    TimeLimit,
    /// The operator's usage limit came near; the run resumes tomorrow.
    UsagePaused,
    /// The provider or the run itself failed.
    Failed,
    /// The pen refused to save, or the Project was turned off.
    Stopped,
}

impl PartialReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::TurnLimit => "turn_limit",
            Self::TimeLimit => "time_limit",
            Self::UsagePaused => "usage_paused",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

/// How a run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalysisEnd {
    Done,
    Partial(PartialReason),
}

/// A way of finding problems.
pub trait Analysis: Send {
    /// The name diagnostics carry for runs of this kind.
    fn name(&self) -> &'static str;
    /// Whether a run asks the operator's AI provider, so it waits for Hide
    /// AI to be ready and the usage limit to be clear (D-21, D-36).
    fn uses_provider(&self) -> bool {
        true
    }
    fn run(&mut self, input: &AnalysisInput, pen: &mut dyn NotebookPen) -> AnalysisEnd;
}

/// The finding that needs no model: each fresh record joins the line that
/// rests on the same signal and tag, or starts one. A CI check's line is an
/// issue, since where to fix it is not known (D-50); every other is a pull
/// request. It is the shipped analysis until an investigation replaces it,
/// and the "조사 못 함" path when an investigation fails having written
/// nothing (D-35).
#[derive(Clone, Copy, Debug, Default)]
pub struct TagGrouping;

impl TagGrouping {
    /// A CI line is an issue; the operator's personal way of working goes
    /// into the Hide global instruction (PRD memory-self-improve D-06).
    fn outcome(entry: &super::ledger::RecordEntry) -> Outcome {
        match entry.signal {
            Signal::CheckFailed => Outcome::Issue,
            _ if entry.said.as_ref().is_some_and(|said| said.personal) => Outcome::Local,
            _ => Outcome::Pr,
        }
    }
}

impl Analysis for TagGrouping {
    fn name(&self) -> &'static str {
        "tag_grouping"
    }

    fn uses_provider(&self) -> bool {
        false
    }

    fn run(&mut self, input: &AnalysisInput, pen: &mut dyn NotebookPen) -> AnalysisEnd {
        let mut groups: BTreeMap<(Signal, &str), Vec<RecordId>> = BTreeMap::new();
        let mut outcomes = BTreeMap::new();
        let mut passed = Vec::new();
        for entry in input.records.fresh() {
            // A correction is named by its Memory card, which Memory itself
            // offers into the documents (D-02); with no investigation to join
            // it to the problem it corrected, it is passed, not lined.
            if entry.signal == Signal::Correction {
                passed.push(entry.id);
                continue;
            }
            outcomes
                .entry((entry.signal, entry.tag.as_str()))
                .or_insert_with(|| Self::outcome(entry));
            groups
                .entry((entry.signal, entry.tag.as_str()))
                .or_default()
                .push(entry.id);
        }
        for ((signal, tag), records) in groups {
            for chunk in records.chunks(crate::suggestions::policy::EDIT_RECORDS) {
                let home = pen
                    .notebook()
                    .lines
                    .iter()
                    .find(|line| {
                        line.basis
                            .as_ref()
                            .is_some_and(|(s, t)| *s == signal && t == tag)
                    })
                    .map(|line| line.id);
                let edit = match home {
                    Some(line) => NotebookEdit::Attach {
                        line,
                        records: chunk.to_vec(),
                    },
                    None => NotebookEdit::NewLine {
                        discomfort: None,
                        records: chunk.to_vec(),
                        basis: Some((signal, tag.to_owned())),
                        outcome: outcomes.get(&(signal, tag)).copied(),
                    },
                };
                match pen.write(edit) {
                    Ok(_) => {}
                    // A group the notebook will not take stays fresh for
                    // the next run; the others go on.
                    Err(PenRefused::Edit(_)) => {}
                    Err(PenRefused::Store { .. } | PenRefused::Stopped) => {
                        return AnalysisEnd::Partial(PartialReason::Stopped);
                    }
                }
            }
        }
        for chunk in passed.chunks(crate::suggestions::policy::EDIT_RECORDS) {
            let edit = NotebookEdit::Pass {
                records: chunk.to_vec(),
            };
            if let Err(PenRefused::Store { .. } | PenRefused::Stopped) = pen.write(edit) {
                return AnalysisEnd::Partial(PartialReason::Stopped);
            }
        }
        AnalysisEnd::Done
    }
}

/// A pen over a notebook in memory, for a replay and for tests: it checks
/// each edit exactly as the store's pen does and keeps what it took.
pub struct MemoryPen<'a> {
    pub notebook: Notebook,
    pub index: &'a RecordIndex,
    pub now: u64,
    pub written: Vec<NotebookEdit>,
}

impl<'a> MemoryPen<'a> {
    pub fn new(notebook: Notebook, index: &'a RecordIndex, now: u64) -> Self {
        Self {
            notebook,
            index,
            now,
            written: Vec::new(),
        }
    }
}

impl NotebookPen for MemoryPen<'_> {
    fn notebook(&self) -> &Notebook {
        &self.notebook
    }

    fn write(&mut self, edit: NotebookEdit) -> Result<Written, PenRefused> {
        let applied = self
            .notebook
            .apply(edit.clone(), self.index, self.now)
            .map_err(PenRefused::Edit)?;
        self.written.push(edit);
        Ok(applied.created.map_or(Written::Done, Written::Line))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suggestions::ledger::{RecordEntry, SessionRef};
    use crate::suggestions::notebook::Stage;
    use std::time::Duration;

    fn entry(id: u64, signal: Signal, tag: &str, session: &str) -> RecordEntry {
        RecordEntry {
            id: RecordId(id),
            signal,
            tag: tag.into(),
            session: Some(SessionRef {
                agent: "claude".into(),
                id: session.into(),
            }),
            pr: None,
            at_unix_ms: 1,
            always: false,
            improvement: None,
            said: None,
            line: None,
            fresh: true,
        }
    }

    fn input(entries: Vec<RecordEntry>) -> AnalysisInput {
        AnalysisInput {
            project_id: "p".into(),
            records: RecordIndex { entries },
            language: hide_ai::OutputLanguage::English,
            budget: AnalysisBudget {
                max_turns: 0,
                deadline: Instant::now() + Duration::from_secs(60),
                stop: StopFlag::default(),
            },
        }
    }

    #[test]
    fn tag_grouping_joins_records_by_signal_and_tag_across_days() {
        let monday = input(vec![
            entry(1, Signal::FailedTool, "cargo test", "a"),
            entry(2, Signal::FailedTool, "cargo test", "b"),
            entry(3, Signal::AgentWait, "question", "a"),
            entry(4, Signal::CheckFailed, "verify", "c"),
            entry(7, Signal::Correction, "m72", "a"),
        ]);
        let mut pen = MemoryPen::new(Notebook::default(), &monday.records, 1);
        assert_eq!(TagGrouping.run(&monday, &mut pen), AnalysisEnd::Done);
        let lines = &pen.notebook.lines;
        assert_eq!(lines.len(), 3, "a Memory card's correction is no line");
        assert!(matches!(
            pen.written.last(),
            Some(NotebookEdit::Pass { records }) if records == &[RecordId(7)]
        ));
        assert_eq!(lines[0].records, [RecordId(1), RecordId(2)]);
        let check = lines
            .iter()
            .find(|line| line.basis == Some((Signal::CheckFailed, "verify".into())))
            .unwrap();
        assert_eq!(check.outcome, Some(Outcome::Issue));

        // The next day's run goes on the same notebook: the same problem
        // lands on the same line, a new one gets its own.
        let mut notebook = pen.notebook.clone();
        notebook.lines[0].stage = Stage::Watching;
        let mut tuesday = monday.records.entries.clone();
        for entry in &mut tuesday {
            entry.fresh = false;
        }
        tuesday.push(entry(5, Signal::FailedTool, "cargo test", "d"));
        tuesday.push(entry(6, Signal::FailedTool, "pnpm lint", "d"));
        let tuesday = input(tuesday);
        let mut pen = MemoryPen::new(notebook, &tuesday.records, 2);
        assert_eq!(TagGrouping.run(&tuesday, &mut pen), AnalysisEnd::Done);
        assert_eq!(
            pen.notebook.lines[0].records,
            [RecordId(1), RecordId(2), RecordId(5)],
            "a recurrence joins the watched line"
        );
        assert_eq!(pen.notebook.lines.len(), 4);
        assert_eq!(
            pen.written.len(),
            2,
            "records already on a line or passed are not handed again"
        );
    }

    /// A pen that saves the first edit and then fails, as a store gone full
    /// would: the run stops and what was written stays.
    struct FailingPen<'a>(MemoryPen<'a>);

    impl NotebookPen for FailingPen<'_> {
        fn notebook(&self) -> &Notebook {
            self.0.notebook()
        }

        fn write(&mut self, edit: NotebookEdit) -> Result<Written, PenRefused> {
            if self.0.written.is_empty() {
                return self.0.write(edit);
            }
            Err(PenRefused::Store {
                code: "suggestions_store_full".into(),
            })
        }
    }

    #[test]
    fn a_run_whose_pen_stops_saving_ends_partial_with_what_it_wrote() {
        let run = input(vec![
            entry(1, Signal::FailedTool, "a", "s"),
            entry(2, Signal::FailedTool, "b", "s"),
        ]);
        let mut pen = FailingPen(MemoryPen::new(Notebook::default(), &run.records, 1));
        assert_eq!(
            TagGrouping.run(&run, &mut pen),
            AnalysisEnd::Partial(PartialReason::Stopped)
        );
        assert_eq!(pen.0.notebook.lines.len(), 1);
    }
}
