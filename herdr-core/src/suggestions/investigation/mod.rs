//! The investigation (PRD hide-ai-suggestions B7, B8, D-31, D-35, D-38):
//! the analysis that edits the notebook with Hide AI over several turns.
//! Each turn the model answers one JSON object of notebook edits and the
//! reads it wants next; Hide writes each edit through the pen, runs only
//! its own read tools ([`reads`]) and hands the results to the next turn.
//! The turns themselves are `hide_ai::run_turns`, so every turn is one
//! call of the operator's Hide AI with its selection, fallback, usage limit
//! and cancellation.
//!
//! A run that gets no usable answer writes nothing and ends `failed`, which
//! the core reads as "조사 못 함" and judges by score alone (D-35). A usage
//! limit or the daily call cap ends it `usage_paused`; an agent that cannot
//! answer ends it `failed` having written nothing new, so the records it
//! did not reach stay fresh for the next run. None of it reaches the screen;
//! the diagnostic log carries counts and the end, never content (B23).

pub mod node;
mod prompt;
pub mod reads;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use hide_ai::{
    AiRouter, CancelToken, FinalCause, Phase, Step, Turn, TurnBudget, TurnHost, TurnsEnd, run_turns,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::analysis::{
    Analysis, AnalysisEnd, AnalysisInput, NotebookPen, PartialReason, PenRefused, Written,
};
use super::notebook::NotebookEdit;
use super::policy;
use reads::{Read, ReadTools};

/// Admits one turn against the daily Hide AI call cap; `Err` names the cap.
pub type Admission = Box<dyn FnMut() -> Result<(), String> + Send>;

/// The analysis that investigates with Hide AI.
pub struct Investigation {
    router: Arc<AiRouter>,
    reads: Box<dyn ReadTools>,
    admit: Admission,
    cancel: CancelToken,
    clock: Box<dyn Fn() -> Instant + Send>,
}

impl Investigation {
    /// `reads` runs on the Project's node; `admit` counts each turn as a
    /// call against the daily cap; `cancel` ends a turn in flight when the
    /// core stops.
    pub fn new(
        router: Arc<AiRouter>,
        reads: Box<dyn ReadTools>,
        admit: Admission,
        cancel: CancelToken,
    ) -> Self {
        Self {
            router,
            reads,
            admit,
            cancel,
            clock: Box::new(Instant::now),
        }
    }

    #[cfg(test)]
    fn with_clock(mut self, clock: Box<dyn Fn() -> Instant + Send>) -> Self {
        self.clock = clock;
        self
    }
}

/// One turn's answer.
#[derive(Deserialize)]
struct Answer {
    #[serde(default)]
    notes: String,
    #[serde(default)]
    reads: Vec<Read>,
    #[serde(default)]
    edits: Vec<NotebookEdit>,
    #[serde(default)]
    done: bool,
}

/// What a run did, for its diagnostic.
#[derive(Default)]
struct Tally {
    reads: BTreeMap<&'static str, u32>,
    reads_refused: u32,
    edits: BTreeMap<&'static str, u32>,
    edits_refused: u32,
}

struct Host<'a> {
    run: String,
    input: &'a AnalysisInput,
    pen: &'a mut dyn NotebookPen,
    reads: &'a mut dyn ReadTools,
    admit: &'a mut Admission,
    started_unix_ms: u64,
    carried: prompt::Carried,
    tally: Tally,
    /// Why the pen took no more edits, when it stopped the run.
    stopped: Option<String>,
}

impl Host<'_> {
    /// Writes the answer's edits one by one, keeping each outcome for the
    /// next turn. A pen that can no longer save stops the run.
    fn write(&mut self, edits: Vec<NotebookEdit>) -> Step {
        self.carried.edits.clear();
        for edit in edits.into_iter().take(policy::EDITS_PER_TURN) {
            let kind = edit.kind();
            let outcome = match self.pen.write(edit) {
                Ok(written) => {
                    *self.tally.edits.entry(kind).or_default() += 1;
                    match written {
                        Written::Line(line) => json!({"edit": kind, "ok": true, "line": line}),
                        Written::Done => json!({"edit": kind, "ok": true}),
                    }
                }
                Err(PenRefused::Edit(refused)) => {
                    self.tally.edits_refused += 1;
                    json!({"edit": kind, "ok": false, "refused": refused})
                }
                Err(PenRefused::Store { code }) => {
                    self.stopped = Some(code);
                    return Step::Stop;
                }
                Err(PenRefused::Stopped) => {
                    self.stopped = Some("stopped".into());
                    return Step::Stop;
                }
            };
            self.carried.edits.push(outcome);
        }
        Step::Continue
    }

    fn read(&mut self, reads: Vec<Read>) {
        self.carried.reads.clear();
        let mut budget = policy::READ_TURN_BYTES;
        for read in reads.into_iter().take(policy::READS_PER_TURN) {
            let (value, outcome) = reads::fulfil(&read, &self.input.records, self.reads, budget);
            budget =
                budget.saturating_sub(serde_json::to_string(&value).map_or(budget, |s| s.len()));
            match outcome {
                Ok(()) => *self.tally.reads.entry(read.tool()).or_default() += 1,
                Err(_) => self.tally.reads_refused += 1,
            }
            self.carried.reads.push(value);
        }
    }
}

impl TurnHost for Host<'_> {
    fn admit(&mut self) -> Result<(), String> {
        (self.admit)()
    }

    fn request(&mut self, turn: &Turn) -> hide_ai::AiRequest {
        let input = prompt::input(
            turn,
            &self.carried,
            &self.input.records,
            self.pen.notebook(),
            self.started_unix_ms,
        );
        prompt::request(
            &self.run,
            &self.input.project_id,
            self.input.language,
            turn,
            input,
        )
    }

    fn answer(&mut self, turn: &Turn, answer: Value) -> Result<Step, String> {
        let answer: Answer =
            serde_json::from_value(answer).map_err(|_| "answer_unreadable".to_owned())?;
        self.carried.notes = answer
            .notes
            .chars()
            .take(policy::NOTES_CHARS)
            .collect::<String>();
        if self.write(answer.edits) == Step::Stop {
            return Ok(Step::Stop);
        }
        if answer.done {
            return Ok(Step::Finish);
        }
        if turn.phase != Phase::Final {
            self.read(answer.reads);
        }
        Ok(Step::Continue)
    }
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

impl Analysis for Investigation {
    fn name(&self) -> &'static str {
        "investigation"
    }

    fn run(&mut self, input: &AnalysisInput, pen: &mut dyn NotebookPen) -> AnalysisEnd {
        let started = (self.clock)();
        let started_unix_ms = now_unix_ms();
        let budget = TurnBudget {
            max_turns: input.budget.max_turns,
            wrap_up_turns: policy::WRAP_UP_TURNS,
            deadline: input.budget.deadline,
            turn_deadline: std::time::Duration::from_millis(policy::TURN_SPAN_MS),
        };
        let mut host = Host {
            run: format!("suggest-{started_unix_ms}"),
            input,
            pen,
            reads: &mut *self.reads,
            admit: &mut self.admit,
            started_unix_ms,
            carried: prompt::Carried::default(),
            tally: Tally::default(),
            stopped: None,
        };
        let clock = &self.clock;
        let run = run_turns(&self.router, &budget, &mut host, &self.cancel, &|| clock());
        let end = match &run.end {
            TurnsEnd::Finished => AnalysisEnd::Done,
            TurnsEnd::Final(FinalCause::TurnLimit) => {
                AnalysisEnd::Partial(PartialReason::TurnLimit)
            }
            TurnsEnd::Final(FinalCause::TimeLimit) | TurnsEnd::OutOfTime => {
                AnalysisEnd::Partial(PartialReason::TimeLimit)
            }
            TurnsEnd::Paused(_) => AnalysisEnd::Partial(PartialReason::UsagePaused),
            TurnsEnd::Final(FinalCause::Unusable)
            | TurnsEnd::Unavailable(_)
            | TurnsEnd::Failed(_) => AnalysisEnd::Partial(PartialReason::Failed),
            TurnsEnd::Cancelled | TurnsEnd::Stopped => AnalysisEnd::Partial(PartialReason::Stopped),
        };
        let cause = match &run.end {
            TurnsEnd::Paused(cap) => Some(cap.clone()),
            TurnsEnd::Unavailable(error) | TurnsEnd::Failed(error) => Some(error.to_string()),
            TurnsEnd::Stopped => host.stopped.clone(),
            _ => None,
        };
        crate::diagnostic!(json!({
            "component": "suggestions",
            "kind": "investigation.finished",
            "project_id": input.project_id,
            "run": host.run,
            "end": match end {
                AnalysisEnd::Done => "done",
                AnalysisEnd::Partial(reason) => reason.as_str(),
            },
            "cause": cause,
            "turns": run.asked,
            "answered": run.answered,
            "reads": host.tally.reads,
            "reads_refused": host.tally.reads_refused,
            "edits": host.tally.edits,
            "edits_refused": host.tally.edits_refused,
            "fresh_records": input.records.fresh().count(),
            "duration_ms": clock().saturating_duration_since(started).as_millis() as u64,
        }));
        end
    }
}

#[cfg(test)]
mod tests;
