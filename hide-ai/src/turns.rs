//! A request answered over several turns (PRD hide-ai-suggestions D-31,
//! D-35, D-38): the model answers one JSON object a turn, the feature acts on
//! it (runs its own read tools, keeps what the answer changed) and builds the
//! next turn's request from the result, until the feature is done or the
//! turns or the time run out.
//!
//! Every turn is one [`AiRouter::execute`], so provider selection and
//! fallback, the usage limit, duplicate suppression, cancellation, process
//! ownership and usage counting are the router's as for any request, and a
//! turn is a call like any other. No provider keeps a conversation between
//! turns: each turn's request carries everything the model needs, which is
//! what lets every provider the router can ask take part the same way.
//!
//! The model never acts itself. A turn's answer is data the feature reads;
//! what a feature runs for it is the feature's own fixed code.

use std::time::{Duration, Instant};

use serde_json::Value;

use crate::{AiError, AiRequest, AiRouter, CancelToken};

/// What one run may spend.
#[derive(Clone, Copy, Debug)]
pub struct TurnBudget {
    pub max_turns: u32,
    /// The turns before the last in which the feature is asked to wrap up.
    pub wrap_up_turns: u32,
    pub deadline: Instant,
    /// The longest one turn is given. The final turn starts while at least
    /// this much of the run is left, so the run's last edits are asked for
    /// before its deadline rather than lost to it.
    pub turn_deadline: Duration,
}

/// Where a turn stands in its run, which the feature words for the model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Explore,
    /// Few turns or little time is left: the model should finish what it
    /// started rather than open something new.
    WrapUp,
    /// The run's last turn. Its request offers no read: the model gives its
    /// final changes from what it has already seen (D-35).
    Final,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Explore => "explore",
            Self::WrapUp => "wrap_up",
            Self::Final => "final",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Turn {
    /// From one.
    pub number: u32,
    /// The turns the run may still take after this one.
    pub left: u32,
    pub phase: Phase,
}

/// What the feature made of an answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Continue,
    /// The model said it was done, or the feature has nothing left to ask.
    Finish,
    /// The feature cannot go on (what it keeps could not be saved, or it
    /// was turned off): nothing more is asked.
    Stop,
}

/// The feature side of a run.
pub trait TurnHost {
    /// Admits one more turn against the caller's own call cap, before
    /// anything is asked; `Err` names the cap and ends the run paused.
    fn admit(&mut self) -> Result<(), String>;
    /// The request for `turn`. Its deadline is cut to what the run has left.
    fn request(&mut self, turn: &Turn) -> AiRequest;
    /// Acts on a schema-valid answer. `Err` says the answer could not be
    /// used, which the run treats as a turn with no answer.
    fn answer(&mut self, turn: &Turn, answer: Value) -> Result<Step, String>;
}

/// Why the run asked for a final turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FinalCause {
    TurnLimit,
    TimeLimit,
    /// A turn before it got no usable answer.
    Unusable,
}

/// How a run ended. Whatever the feature kept from the answers it took
/// stays kept in every case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TurnsEnd {
    /// The feature said it was done.
    Finished,
    /// The final turn was answered; the cause says why it was the final.
    Final(FinalCause),
    /// The deadline passed before a final turn could be asked.
    OutOfTime,
    /// The caller's call cap or the operator's usage limit: nothing more
    /// was asked, and asking again later will be answered.
    Paused(String),
    /// No provider could take a turn (not signed in, unavailable, a
    /// process or rate cap), or Hide AI was turned off.
    Unavailable(AiError),
    /// A turn got no usable answer and so did the final turn after it.
    Failed(AiError),
    Cancelled,
    /// The feature stopped the run ([`Step::Stop`]).
    Stopped,
}

/// A run's end with what it spent, for the caller's diagnostic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnsRun {
    pub end: TurnsEnd,
    /// Turns asked, answered or not.
    pub asked: u32,
    /// Turns whose answer the feature took.
    pub answered: u32,
}

enum Failure {
    Paused(String),
    Unavailable(AiError),
    Unusable(AiError),
    Cancelled,
}

fn classify(error: AiError) -> Failure {
    match error {
        AiError::UsageLimited { .. } => Failure::Paused(error.to_string()),
        AiError::Cancelled => Failure::Cancelled,
        AiError::InvalidOutput(_)
        | AiError::Rejected(_)
        | AiError::Timeout
        | AiError::CompletionUnknown(_) => Failure::Unusable(error),
        AiError::NotAuthenticated
        | AiError::ProviderUnavailable(_)
        | AiError::Transient(_)
        | AiError::Internal(_)
        | AiError::Unsupported(_)
        | AiError::OverBudget { .. }
        | AiError::NoProvider(_)
        | AiError::Disabled => Failure::Unavailable(error),
    }
}

impl TurnBudget {
    fn final_cause(&self, number: u32, now: Instant) -> Option<FinalCause> {
        if number >= self.max_turns {
            Some(FinalCause::TurnLimit)
        } else if self.deadline.saturating_duration_since(now) <= self.turn_deadline {
            Some(FinalCause::TimeLimit)
        } else {
            None
        }
    }

    fn wrapping_up(&self, number: u32, now: Instant) -> bool {
        let left = self.deadline.saturating_duration_since(now);
        self.max_turns - number <= self.wrap_up_turns
            || left <= self.turn_deadline.saturating_mul(self.wrap_up_turns + 1)
    }
}

/// Runs turns until the host is done or the budget is spent.
pub fn run_turns(
    router: &AiRouter,
    budget: &TurnBudget,
    host: &mut dyn TurnHost,
    cancel: &CancelToken,
    now: &dyn Fn() -> Instant,
) -> TurnsRun {
    let mut asked = 0;
    let mut answered = 0;
    let mut forced: Option<FinalCause> = None;
    let end = loop {
        if cancel.is_cancelled() {
            break TurnsEnd::Cancelled;
        }
        let number = asked + 1;
        let started = now();
        if number > budget.max_turns || started >= budget.deadline {
            break TurnsEnd::OutOfTime;
        }
        let cause = forced.or_else(|| budget.final_cause(number, started));
        let turn = Turn {
            number,
            left: budget.max_turns - number,
            phase: match cause {
                Some(_) => Phase::Final,
                None if budget.wrapping_up(number, started) => Phase::WrapUp,
                None => Phase::Explore,
            },
        };
        if let Err(cap) = host.admit() {
            break TurnsEnd::Paused(cap);
        }
        let mut request = host.request(&turn);
        request.deadline = request
            .deadline
            .min(budget.deadline.saturating_duration_since(started));
        asked = number;
        let failure = match router.execute(&request, cancel) {
            Ok(result) => match host.answer(&turn, result.value) {
                Ok(Step::Finish) => {
                    answered += 1;
                    break TurnsEnd::Finished;
                }
                Ok(Step::Stop) => {
                    answered += 1;
                    break TurnsEnd::Stopped;
                }
                Ok(Step::Continue) => {
                    answered += 1;
                    match cause {
                        Some(cause) => break TurnsEnd::Final(cause),
                        None => continue,
                    }
                }
                Err(reason) => Failure::Unusable(AiError::InvalidOutput(reason)),
            },
            Err(error) => classify(error),
        };
        match failure {
            Failure::Paused(reason) => break TurnsEnd::Paused(reason),
            Failure::Unavailable(error) => break TurnsEnd::Unavailable(error),
            Failure::Cancelled => break TurnsEnd::Cancelled,
            // The final turn had no usable answer either: nothing more is
            // asked.
            Failure::Unusable(error) if cause.is_some() => break TurnsEnd::Failed(error),
            // One last turn asks for the final changes from what the model
            // has already seen.
            Failure::Unusable(_) => forced = Some(FinalCause::Unusable),
        }
    };
    TurnsRun {
        end,
        asked,
        answered,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::NoopLogSink;
    use crate::{
        AiBackend, AiResponse, Availability, ModelCatalog, OutputLanguage, ProviderId, RouterConfig,
    };
    use serde_json::json;
    use std::cell::Cell;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    /// A provider that answers from a script, one entry a request.
    #[derive(Default)]
    struct Script {
        answers: Mutex<VecDeque<Result<Value, AiError>>>,
        asked: Mutex<Vec<AiRequest>>,
    }

    impl AiBackend for Script {
        fn id(&self) -> ProviderId {
            ProviderId::CLAUDE
        }
        fn availability(&self) -> Availability {
            Availability::Ready
        }
        fn models(&self) -> ModelCatalog {
            ModelCatalog::Offered(Vec::new())
        }
        fn execute(&self, request: &AiRequest, _: &CancelToken) -> Result<AiResponse, AiError> {
            self.asked.lock().unwrap().push(request.clone());
            let next = self.answers.lock().unwrap().pop_front();
            next.expect("the script has an answer for every turn")
                .map(|value| AiResponse {
                    value,
                    usage: Default::default(),
                })
        }
    }

    fn router(script: &Arc<Script>) -> AiRouter {
        AiRouter::with_sleep(
            vec![Arc::clone(script) as Arc<dyn AiBackend>],
            RouterConfig {
                priority: vec![ProviderId::CLAUDE],
                ..RouterConfig::default()
            },
            Arc::new(NoopLogSink),
            Box::new(|_| {}),
        )
    }

    fn scripted(answers: impl IntoIterator<Item = Result<Value, AiError>>) -> Arc<Script> {
        Arc::new(Script {
            answers: Mutex::new(answers.into_iter().collect()),
            asked: Mutex::default(),
        })
    }

    /// A host that records each turn it was asked to build and finishes when
    /// an answer says so.
    #[derive(Default)]
    struct Host {
        turns: Vec<Turn>,
        cap: Option<u32>,
    }

    impl TurnHost for Host {
        fn admit(&mut self) -> Result<(), String> {
            match &mut self.cap {
                Some(0) => Err("daily_cap".into()),
                Some(left) => {
                    *left -= 1;
                    Ok(())
                }
                None => Ok(()),
            }
        }
        fn request(&mut self, turn: &Turn) -> AiRequest {
            self.turns.push(*turn);
            AiRequest {
                feature_id: "turns_test".into(),
                request_id: crate::RequestId(format!("t{}", turn.number)),
                subject_id: "subject".into(),
                system: "system".into(),
                input: format!("turn {} {}", turn.number, turn.phase.as_str()),
                output_schema: json!({"type": "object"}),
                deadline: Duration::from_secs(120),
                schema_version: "turns_test.v1".into(),
                pick: None,
                output_language: Some(OutputLanguage::English),
                answer: crate::AnswerMode::PlainJson,
                provider_efforts: Vec::new(),
            }
        }
        fn answer(&mut self, _: &Turn, answer: Value) -> Result<Step, String> {
            match answer["step"].as_str() {
                Some("finish") => Ok(Step::Finish),
                Some("stop") => Ok(Step::Stop),
                Some("continue") => Ok(Step::Continue),
                _ => Err("no_step".into()),
            }
        }
    }

    fn go() -> Result<Value, AiError> {
        Ok(json!({"step": "continue"}))
    }

    fn budget(max_turns: u32, start: Instant, run: Duration) -> TurnBudget {
        TurnBudget {
            max_turns,
            wrap_up_turns: 2,
            deadline: start + run,
            turn_deadline: Duration::from_secs(60),
        }
    }

    fn phases(host: &Host) -> Vec<Phase> {
        host.turns.iter().map(|turn| turn.phase).collect()
    }

    #[test]
    fn a_run_wraps_up_before_its_last_turn_and_asks_its_final_turn_at_the_turn_limit() {
        let script = scripted(std::iter::repeat_with(go).take(5));
        let start = Instant::now();
        let mut host = Host::default();
        let run = run_turns(
            &router(&script),
            &budget(5, start, Duration::from_secs(3600)),
            &mut host,
            &CancelToken::new(),
            &|| start,
        );
        assert_eq!(run.end, TurnsEnd::Final(FinalCause::TurnLimit));
        assert_eq!((run.asked, run.answered), (5, 5));
        assert_eq!(
            phases(&host),
            [
                Phase::Explore,
                Phase::Explore,
                Phase::WrapUp,
                Phase::WrapUp,
                Phase::Final
            ]
        );
        assert_eq!(host.turns[4].left, 0);
        assert_eq!(script.asked.lock().unwrap().len(), 5, "one call a turn");
    }

    #[test]
    fn a_run_ends_when_the_feature_is_done_or_stops_it() {
        let script = scripted([go(), Ok(json!({"step": "finish"}))]);
        let start = Instant::now();
        let mut host = Host::default();
        let run = run_turns(
            &router(&script),
            &budget(30, start, Duration::from_secs(1200)),
            &mut host,
            &CancelToken::new(),
            &|| start,
        );
        assert_eq!(run.end, TurnsEnd::Finished);
        assert_eq!((run.asked, run.answered), (2, 2));

        let script = scripted([go(), Ok(json!({"step": "stop"}))]);
        let run = run_turns(
            &router(&script),
            &budget(30, start, Duration::from_secs(1200)),
            &mut Host::default(),
            &CancelToken::new(),
            &|| start,
        );
        assert_eq!(run.end, TurnsEnd::Stopped);
        assert_eq!(run.asked, 2);
    }

    #[test]
    fn the_final_turn_starts_while_a_turns_time_is_left_and_no_turn_outlives_the_run() {
        let script = scripted(std::iter::repeat_with(go).take(3));
        let start = Instant::now();
        let clock = Cell::new(start);
        let mut host = Host::default();
        let run = run_turns(
            &router(&script),
            &budget(30, start, Duration::from_secs(250)),
            &mut host,
            &CancelToken::new(),
            &|| {
                // Each turn starts 100 seconds after the one before it.
                let at = clock.get();
                clock.set(at + Duration::from_secs(100));
                at
            },
        );
        assert_eq!(run.end, TurnsEnd::Final(FinalCause::TimeLimit));
        assert_eq!(phases(&host), [Phase::Explore, Phase::WrapUp, Phase::Final]);
        let asked = script.asked.lock().unwrap();
        assert_eq!(
            asked[2].deadline,
            Duration::from_secs(50),
            "a turn is given no more than the run has left"
        );
    }

    #[test]
    fn an_unusable_turn_is_followed_by_the_final_turn_and_a_failed_final_ends_the_run() {
        let script = scripted([
            go(),
            // The router asks no provider again after a timeout.
            Err(AiError::Timeout),
            Ok(json!({"no": "step"})),
        ]);
        let start = Instant::now();
        let mut host = Host::default();
        let run = run_turns(
            &router(&script),
            &budget(30, start, Duration::from_secs(1200)),
            &mut host,
            &CancelToken::new(),
            &|| start,
        );
        assert_eq!(
            run.end,
            TurnsEnd::Failed(AiError::InvalidOutput("no_step".into()))
        );
        assert_eq!((run.asked, run.answered), (3, 1));
        assert_eq!(
            phases(&host),
            [Phase::Explore, Phase::Explore, Phase::Final]
        );
    }

    #[test]
    fn a_final_turn_answered_after_an_unusable_one_keeps_its_answer() {
        let script = scripted([Err(AiError::Timeout), go()]);
        let start = Instant::now();
        let run = run_turns(
            &router(&script),
            &budget(30, start, Duration::from_secs(1200)),
            &mut Host::default(),
            &CancelToken::new(),
            &|| start,
        );
        assert_eq!(run.end, TurnsEnd::Final(FinalCause::Unusable));
        assert_eq!((run.asked, run.answered), (2, 1));
    }

    #[test]
    fn a_usage_limit_or_the_call_cap_pauses_the_run_and_nothing_more_is_asked() {
        let script = scripted([
            go(),
            Err(AiError::UsageLimited {
                retry_after: Some(Duration::from_secs(60)),
            }),
        ]);
        let start = Instant::now();
        let run = run_turns(
            &router(&script),
            &budget(30, start, Duration::from_secs(1200)),
            &mut Host::default(),
            &CancelToken::new(),
            &|| start,
        );
        assert_eq!(
            run.end,
            TurnsEnd::Paused("usage_limited;retry_after_s=60".into())
        );
        assert_eq!(run.asked, 2);

        let script = scripted([go()]);
        let run = run_turns(
            &router(&script),
            &budget(30, start, Duration::from_secs(1200)),
            &mut Host {
                cap: Some(1),
                ..Host::default()
            },
            &CancelToken::new(),
            &|| start,
        );
        assert_eq!(run.end, TurnsEnd::Paused("daily_cap".into()));
        assert_eq!(script.asked.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_provider_that_cannot_answer_ends_the_run_without_a_final_turn() {
        let script = scripted([go(), Err(AiError::NotAuthenticated)]);
        let start = Instant::now();
        let mut host = Host::default();
        let run = run_turns(
            &router(&script),
            &budget(30, start, Duration::from_secs(1200)),
            &mut host,
            &CancelToken::new(),
            &|| start,
        );
        assert_eq!(run.end, TurnsEnd::Unavailable(AiError::NotAuthenticated));
        assert_eq!(host.turns.len(), 2);
    }

    #[test]
    fn a_cancelled_run_asks_nothing_more() {
        let script = scripted([]);
        let cancel = CancelToken::new();
        cancel.cancel();
        let start = Instant::now();
        let run = run_turns(
            &router(&script),
            &budget(30, start, Duration::from_secs(1200)),
            &mut Host::default(),
            &cancel,
            &|| start,
        );
        assert_eq!(run.end, TurnsEnd::Cancelled);
        assert_eq!(run.asked, 0);
    }
}
