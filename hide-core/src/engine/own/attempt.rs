//! The attempt of a start with a profile (PRD agent-profiles B22, D-26;
//! docs/ARCHITECTURE.md, Starting an agent with a profile).
//!
//! A profile's start line runs `hide agent exec --attempt <id>`, and typing
//! it proves nothing of what came of it. The attempt is kept with its pane
//! (`PaneLive::attempt`), apart from the pane's agent, so recognizing the
//! agent (`agents::apply`, a hook's session) never ends it. It ends once, in
//! its first transition under the topology's lock ([`Attempt::settle`]):
//!
//! - **Started** when a fresh read of its process, asked after the pane's
//!   machine named the process that sent its `Ready` (its executor, by pid
//!   and start), shows the program running: on Unix that same process no
//!   longer the attempt's executor (`exec` replaced it); on Windows the
//!   child the executor reported once it ran in its job and resumed, still
//!   its child. The pane's facts are never this evidence, and the read is
//!   the node's, never a cache ([`PaneHolders::process`]).
//! - **Failed** when the executor reports that its check refused the
//!   program or its start failed.
//! - **Unconfirmed** when nothing says: the caller's deadline passed, the
//!   process ended before a read showed the program, the pane ended, or
//!   the engine stopped. It claims neither that the program ran nor that it
//!   did not, and nothing starts again on it.
//!
//! The waiter reads and sleeps off every lock and only reads or settles the
//! attempt's state under the topology's lock, so a report or a fact never
//! waits on it; the [`Finish`] it holds ends the attempt on every path out,
//! including a waiter that returns early or unwinds.
//!
//! [`PaneHolders::process`]: hide_node_link::holders::PaneHolders::process

use std::sync::Arc;

use hide_agent_hooks::hook_request::{ExecSeen, seen_child, seen_executor};
use hide_node_link::holders::{HolderError, HolderProcess};
use hide_node_link::panes::{AgentExecEvent, ExecProcess};

use crate::engine::{ATTEMPT_UNKNOWN, AgentExecReport};

/// What came of a start with a profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Outcome {
    /// A fresh read of its process showed its program running.
    Started,
    /// Its executor reported that the program did not start: `checked`
    /// when its check refused the program before any start.
    Failed { failure: String, checked: bool },
    /// Nothing says what came of it; `why` names what ended the wait.
    Unconfirmed { why: &'static str },
}

/// The caller's deadline passed with nothing read of the program.
pub(super) const DEADLINE: &str = "deadline";
/// The attempt's process ended, or another took its pid, before a read
/// showed the program: it may have run and ended, or never run.
pub(super) const ENDED_UNSEEN: &str = "ended_unseen";
/// The pane ended, or a newer start took it.
pub(super) const PANE_ENDED: &str = "pane_ended";
/// The engine stopped.
pub(super) const ENGINE_STOPPED: &str = "engine_stopped";
/// Its waiter went before anything ended it.
pub(super) const ABANDONED: &str = "abandoned";

/// A start with a profile while its pane keeps it.
#[derive(Clone, Debug)]
pub(super) struct Attempt {
    pub(super) id: String,
    /// The claim of the start it belongs to: a failure withdraws the
    /// provisional agent of this claim alone.
    pub(super) claim: Arc<()>,
    /// The process that sent its `Ready`, as the pane's machine named it.
    executor: Option<ExecProcess>,
    /// Windows: the program's process, once the executor ran it in its job.
    child: Option<ExecProcess>,
    /// What came of it, once something did.
    outcome: Option<Outcome>,
}

/// What a waiter finds of its attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Look {
    /// It has ended.
    Ended,
    /// No `Ready` yet: nothing of the program to read.
    Unready,
    /// Its executor sent its `Ready`: what to read.
    Ready {
        executor: ExecProcess,
        child: Option<ExecProcess>,
    },
}

impl Attempt {
    pub(super) fn new(id: String, claim: Arc<()>) -> Self {
        Self {
            id,
            claim,
            executor: None,
            child: None,
            outcome: None,
        }
    }

    /// Whether nothing has ended it yet.
    pub(super) fn pending(&self) -> bool {
        self.outcome.is_none()
    }

    #[cfg(test)]
    pub(super) fn outcome(&self) -> Option<&Outcome> {
        self.outcome.as_ref()
    }

    pub(super) fn look(&self) -> Look {
        match (&self.outcome, self.executor) {
            (Some(_), _) => Look::Ended,
            (None, None) => Look::Unready,
            (None, Some(executor)) => Look::Ready {
                executor,
                child: self.child,
            },
        }
    }

    /// Takes `report`, refused unless it names this attempt while it runs
    /// and comes from its executor: the one process whose `Ready` it took,
    /// or, for a check's refusal, any executor of it before a `Ready`.
    pub(super) fn report(&mut self, report: &AgentExecReport) -> Result<(), &'static str> {
        if report.attempt != self.id || self.outcome.is_some() {
            return Err(ATTEMPT_UNKNOWN);
        }
        let from_executor = self.executor == Some(report.executor);
        match &report.event {
            AgentExecEvent::Ready if self.executor.is_none() => {
                self.executor = Some(report.executor);
            }
            AgentExecEvent::Refused { failure } if self.executor.is_none() || from_executor => {
                self.outcome = Some(Outcome::Failed {
                    failure: failure.clone(),
                    checked: true,
                });
            }
            AgentExecEvent::Failed { failure } if from_executor => {
                self.outcome = Some(Outcome::Failed {
                    failure: failure.clone(),
                    checked: false,
                });
            }
            AgentExecEvent::Spawned { child } if from_executor && self.child.is_none() => {
                self.child = Some(*child);
            }
            _ => return Err(ATTEMPT_MISMATCH),
        }
        Ok(())
    }

    /// Ends it with `outcome` unless something ended it first; what it
    /// ended with.
    pub(super) fn settle(&mut self, outcome: Outcome) -> Outcome {
        self.outcome.get_or_insert(outcome).clone()
    }
}

/// A report that names its attempt but not as that attempt allows: a
/// second `Ready`, or a report from another process than its executor.
pub(super) const ATTEMPT_MISMATCH: &str = "attempt_mismatch";

/// What a fresh read says of an attempt its executor made `Ready`:
/// `read` asks the pane's node for one process now.
pub(super) fn seen(
    attempt: &str,
    executor: ExecProcess,
    child: Option<ExecProcess>,
    read: impl Fn(u32) -> Result<Option<HolderProcess>, HolderError>,
) -> ExecSeen {
    // A node that could not answer says nothing yet; the deadline ends a
    // wait that never hears.
    match child {
        Some(child) => match read(child.pid) {
            Ok(process) => seen_child(
                process.map(|process| (process.started, process.parent)),
                child,
                executor,
            ),
            Err(_) => ExecSeen::Waiting,
        },
        None => match read(executor.pid) {
            Ok(process) => seen_executor(
                process
                    .as_ref()
                    .map(|process| (process.started, process.argv.as_slice())),
                executor,
                attempt,
            ),
            Err(_) => ExecSeen::Waiting,
        },
    }
}

/// When a waiter's attempt ends, and that it ends on every path out of the
/// waiter: dropped before [`Finish::end`], it ends the attempt as
/// abandoned.
pub(super) struct Finish<'a, 'g> {
    engine: &'a super::OwnEngine<'g>,
    pane: String,
    attempt: String,
    ended: bool,
}

impl<'a, 'g> Finish<'a, 'g> {
    pub(super) fn new(engine: &'a super::OwnEngine<'g>, pane: &str, attempt: &str) -> Self {
        Self {
            engine,
            pane: pane.to_owned(),
            attempt: attempt.to_owned(),
            ended: false,
        }
    }

    pub(super) fn pane(&self) -> &str {
        &self.pane
    }

    pub(super) fn attempt(&self) -> &str {
        &self.attempt
    }

    /// Hands the attempt to a waiter elsewhere (a restore's observer
    /// thread), which ends it with a [`Finish`] of its own: its pane and id.
    pub(super) fn hand_off(mut self) -> (String, String) {
        self.ended = true;
        (
            std::mem::take(&mut self.pane),
            std::mem::take(&mut self.attempt),
        )
    }

    /// Ends the attempt, settled with `abandoned` if nothing settled it,
    /// and answers what came of it.
    pub(super) fn end(mut self) -> Outcome {
        self.ended = true;
        self.engine.finish_attempt(&self.pane, &self.attempt)
    }
}

impl Drop for Finish<'_, '_> {
    fn drop(&mut self) {
        if !self.ended {
            let _ = self.engine.finish_attempt(&self.pane, &self.attempt);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "1794175536904-d976a9485d217ccbe6a55d4704337678";
    const EXECUTOR: ExecProcess = ExecProcess {
        pid: 77,
        started: 5,
    };

    fn report(event: AgentExecEvent, executor: ExecProcess) -> AgentExecReport {
        AgentExecReport {
            attempt: ID.to_owned(),
            event,
            executor,
        }
    }

    fn attempt() -> Attempt {
        Attempt::new(ID.to_owned(), Arc::new(()))
    }

    #[test]
    fn an_attempt_takes_one_ready_and_then_reports_only_from_that_executor() {
        let mut attempt = attempt();
        attempt
            .report(&report(AgentExecEvent::Ready, EXECUTOR))
            .unwrap();
        let other = ExecProcess {
            pid: 77,
            started: 6,
        };
        assert_eq!(
            attempt.report(&report(AgentExecEvent::Ready, EXECUTOR)),
            Err(ATTEMPT_MISMATCH)
        );
        let failed = AgentExecEvent::Failed {
            failure: "no".to_owned(),
        };
        assert_eq!(
            attempt.report(&report(failed.clone(), other)),
            Err(ATTEMPT_MISMATCH)
        );
        assert!(attempt.pending());
        attempt.report(&report(failed, EXECUTOR)).unwrap();
        assert_eq!(
            attempt.outcome(),
            Some(&Outcome::Failed {
                failure: "no".to_owned(),
                checked: false
            })
        );
    }

    #[test]
    fn a_report_of_another_or_an_ended_attempt_changes_nothing() {
        let mut attempt = attempt();
        let mut other = report(AgentExecEvent::Ready, EXECUTOR);
        other.attempt = "1794175536904-00000000000000000000000000000000".to_owned();
        assert_eq!(attempt.report(&other), Err(ATTEMPT_UNKNOWN));
        assert_eq!(
            attempt.settle(Outcome::Unconfirmed { why: DEADLINE }),
            Outcome::Unconfirmed { why: DEADLINE }
        );
        assert_eq!(
            attempt.report(&report(
                AgentExecEvent::Refused {
                    failure: "late".to_owned()
                },
                EXECUTOR
            )),
            Err(ATTEMPT_UNKNOWN)
        );
        assert_eq!(
            attempt.settle(Outcome::Started),
            Outcome::Unconfirmed { why: DEADLINE }
        );
    }

    #[test]
    fn a_windows_child_counts_only_after_its_executor_reported_it() {
        let child = ExecProcess {
            pid: 90,
            started: 9,
        };
        let running = |pid: u32| {
            Ok(Some(HolderProcess {
                started: if pid == child.pid { 9 } else { 5 },
                parent: Some(if pid == child.pid { 77 } else { 1 }),
                argv: if pid == child.pid {
                    vec!["claude".to_owned()]
                } else {
                    ["hide.exe", "agent", "exec", "--attempt", ID]
                        .map(str::to_owned)
                        .to_vec()
                },
            }))
        };
        // A suspended child the system already lists is no evidence: the
        // executor's own process is still its executor.
        assert_eq!(seen(ID, EXECUTOR, None, running), ExecSeen::Waiting);
        assert_eq!(seen(ID, EXECUTOR, Some(child), running), ExecSeen::Ran);
        let reused = |_| {
            Ok(Some(HolderProcess {
                started: 10,
                parent: Some(77),
                argv: vec!["claude".to_owned()],
            }))
        };
        assert_eq!(seen(ID, EXECUTOR, Some(child), reused), ExecSeen::Ended);
    }

    #[test]
    fn a_unix_exec_counts_only_for_the_same_process_no_longer_the_executor() {
        let read = |started: u64, argv: &[&str]| {
            let argv = argv.iter().map(|word| (*word).to_owned()).collect();
            move |_| {
                Ok(Some(HolderProcess {
                    started,
                    parent: Some(1),
                    argv: Vec::clone(&argv),
                }))
            }
        };
        let executor = [
            "/opt/hide",
            "agent",
            "exec",
            "--attempt",
            ID,
            "--profile",
            "p",
        ];
        assert_eq!(
            seen(ID, EXECUTOR, None, read(5, &executor)),
            ExecSeen::Waiting
        );
        assert_eq!(seen(ID, EXECUTOR, None, read(5, &[])), ExecSeen::Waiting);
        assert_eq!(
            seen(ID, EXECUTOR, None, read(5, &["node", "/opt/claude/cli.js"])),
            ExecSeen::Ran
        );
        assert_eq!(
            seen(ID, EXECUTOR, None, read(6, &["node", "/opt/claude/cli.js"])),
            ExecSeen::Ended
        );
        assert_eq!(seen(ID, EXECUTOR, None, |_| Ok(None)), ExecSeen::Ended);
        assert_eq!(
            seen(ID, EXECUTOR, None, |_| Err(HolderError::Unreachable {
                message: "lost".to_owned()
            })),
            ExecSeen::Waiting
        );
    }
}
