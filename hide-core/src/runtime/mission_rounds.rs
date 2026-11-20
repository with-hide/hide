//! A node's review round (PRD mission-speed-2): the worker's check wakes
//! the node's own checkers, and a failed result goes to the worker, with no
//! Conductor between them.
//!
//! A round opens when the worker signals its work ready
//! (`MissionNode::check_signal`: its `check` line passed, or every line it
//! answers for passed on a node without one). The signal is the only cause:
//! a push wakes no one, and the same record again changes nothing, so it
//! wakes no one twice (B3, B17). Each live checker bound to the node gets a
//! cell (`RoundWake::CheckAsked`); a checker line with none alive gives
//! the Conductor one (`CheckerNeeded`), and a checker it binds takes the
//! ended one's slot (`Criterion::bind`). A checker's answer in the round
//! that fails a line gives the worker a cell (`Checked`), which its words
//! derive from the node's lines when it is read (`round_facts`); nothing
//! new is stored and nothing waits on a clock (D-02, D-09, D-16). Every one
//! of these is news to a node's worker or checker, so it reaches it at its
//! next tool use (`PendingWake::mid_turn`).

use serde_json::json;

use super::Runtime;
use crate::mission_criteria::{CriterionKind, CriterionState};
use crate::missions::{Dependency, Mission, MissionNode, RoundWake, WakeEvent};

/// What a node's worker reads of its round: each checker line's verdict in
/// the round (`Criterion::round_state`), the rounds its checkers used of
/// the mission's limit, and whether its pull request conflicts with its
/// base (D-09). Only facts the book holds; a value it does not hold yet is
/// left out.
pub(super) struct RoundFacts {
    /// Each checker line's kind, its verdict in the round, and whether a
    /// checker is bound to it at all: a line none checks waits on no one.
    pub(super) lines: Vec<(CriterionKind, CriterionState, bool)>,
    pub(super) rounds: Option<u32>,
    pub(super) limit: u32,
    pub(super) conflict: bool,
}

impl RoundFacts {
    pub(super) fn of(mission: &Mission, node: &MissionNode) -> Self {
        let signal = node.check_signal();
        let since = signal.as_ref().map_or(0, |signal| signal.at_unix_ms);
        let commit = signal.as_ref().and_then(|signal| signal.commit.as_deref());
        let limit = crate::mission_health::review_limit(mission);
        Self {
            lines: node
                .checker_lines()
                .map(|line| {
                    let bound = line
                        .checkers
                        .iter()
                        .any(|checker| checker.agent_id.is_some());
                    (line.kind, line.round_state(since, commit), bound)
                })
                .collect(),
            rounds: node.review_line(limit).map(|line| line.rounds),
            limit,
            conflict: node
                .woken
                .iter()
                .any(|woken| woken.why == crate::missions::FixReason::Conflict),
        }
    }

    pub(super) fn failed(&self) -> bool {
        self.lines
            .iter()
            .any(|(_, state, _)| *state == CriterionState::Fail)
    }

    /// Whether a line still waits on its checker in the round.
    pub(super) fn running(&self) -> bool {
        self.lines
            .iter()
            .any(|(_, state, bound)| *bound && *state == CriterionState::Open)
    }

    /// `review FAIL · verify running · round 2 of 3 · conflicts with its base`.
    pub(super) fn line(&self) -> String {
        let mut parts: Vec<String> = self
            .lines
            .iter()
            .map(|(kind, state, bound)| {
                let name = match kind {
                    CriterionKind::Verify => "verify",
                    _ => "review",
                };
                let state = match state {
                    CriterionState::Pass => "PASS",
                    CriterionState::Fail => "FAIL",
                    CriterionState::Skipped => "skipped",
                    CriterionState::Open if *bound => "running",
                    CriterionState::Open => "no checker",
                };
                format!("{name} {state}")
            })
            .collect();
        if let Some(rounds) = self.rounds {
            parts.push(format!("round {rounds} of {}", self.limit));
        }
        if self.conflict {
            parts.push("conflicts with its base".to_owned());
        }
        parts.join(" · ")
    }
}

/// The checker lines of `node` that `agent` checks, as a cell names them:
/// `review`, `verify` or `review and verify`.
pub(super) fn lines_of(node: &MissionNode, agent: &str) -> String {
    let keys: Vec<&str> = node
        .checker_lines()
        .filter(|line| {
            line.checkers
                .iter()
                .any(|checker| checker.agent_id.as_deref() == Some(agent))
        })
        .map(|line| line.key.as_str())
        .collect();
    if keys.is_empty() {
        "check".to_owned()
    } else {
        keys.join(" and ")
    }
}

/// The commit `agent` last answered on for `node`, and whether it answered
/// at all: a first check reads everything, a later one what changed.
pub(super) fn last_answer(node: &MissionNode, agent: &str) -> Option<Option<String>> {
    node.checker_lines()
        .flat_map(|line| &line.checkers)
        .filter(|checker| {
            checker.agent_id.as_deref() == Some(agent) && checker.at_unix_ms.is_some()
        })
        .max_by_key(|checker| checker.at_unix_ms)
        .map(|checker| checker.commit.clone())
}

/// The line a node worker's brief ends with: on a node with a check
/// command, then on one without.
const BRIEF_LINES: [&str; 2] = [
    "Before you record your `check` line, run the local checks the change touches: recording `check` starts the node's review and verification.",
    "Recording every one of your lines passed starts the node's review and verification.",
];

/// A commit as a cell shows it.
pub(super) fn short(commit: Option<&str>) -> &str {
    commit.map_or("-", |commit| commit.get(..7).unwrap_or(commit))
}

impl Runtime {
    /// After a record on node `node` of mission `id` changed it: a new
    /// check signal wakes the node's checkers (B1-B5), and a checker's
    /// answer that fails the round goes to the worker (B6, B8). Whether a
    /// wake was recorded.
    pub(super) fn mission_round_moved(
        &mut self,
        id: &str,
        node: &str,
        kind: CriterionKind,
        signal_before: Option<crate::missions::CheckSignal>,
    ) -> bool {
        let Some(mission) = self.missions.book.get(id) else {
            return false;
        };
        let Some(row) = mission.node(node) else {
            return false;
        };
        let woke = if kind.checker_judges() {
            self.tell_worker(id, node)
        } else {
            let signal = row.check_signal();
            signal.is_some() && signal != signal_before && self.ask_checkers(id, node)
        };
        if woke {
            // A cell for a sleeping worker or checker wakes it
            // (`agent_sleep::letters`).
            self.letter_wakes.touch();
        }
        woke
    }

    /// Ends a node worker's brief with what starts its node's checkers (PRD
    /// mission-speed-2 B15), on a node that has checkers. A spawn asked
    /// again is the spawn it was (`SpawnRecord::asked_with`, engineering
    /// #11): when an earlier spawn of the intent was asked with the brief as
    /// written or with either line, its brief stands, whatever the plan
    /// says now and whichever build reserved it; a brief the line would
    /// take past the letter limit is left as the Conductor wrote it, and
    /// logged.
    pub(super) fn brief_round_line(
        &self,
        actor: &crate::delivery::Actor,
        intent: &str,
        brief: &mut String,
        earlier: Option<&crate::coordination::SpawnRecord>,
    ) {
        if let Some(earlier) = earlier {
            let asked = std::iter::once(brief.clone())
                .chain(BRIEF_LINES.iter().map(|line| format!("{brief}\n\n{line}")))
                .find(|text| earlier.asked_with(Some(text)));
            if let Some(asked) = asked {
                *brief = asked;
                return;
            }
        }
        let Some(mission) = self.missions.book.conducted_open(actor) else {
            return;
        };
        let Some(node) = mission.intent_node(intent) else {
            return;
        };
        if node.checker_lines().next().is_none() {
            return;
        }
        let line = if node
            .criteria
            .iter()
            .any(|line| line.kind == CriterionKind::Check)
        {
            BRIEF_LINES[0]
        } else {
            BRIEF_LINES[1]
        };
        if brief.len() + line.len() + 2 > crate::delivery::BODY_LIMIT {
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.brief_line_skipped",
                "mission": mission.id, "node": node.key, "cause": "brief_limit",
            }));
            return;
        }
        brief.push_str("\n\n");
        brief.push_str(line);
    }

    /// Wakes each live checker of `node` for the round its check opened,
    /// and the Conductor for a checker line with none alive.
    fn ask_checkers(&mut self, id: &str, node: &str) -> bool {
        let ledger = self.delivery_state().ok();
        let records = ledger.as_deref().map(|ledger| &ledger.agents);
        let sleeping =
            self.sleeping_registrations(records.into_iter().flatten().filter(|row| !row.ended));
        let live = |agent: &str| {
            records
                .into_iter()
                .flatten()
                .find(|row| row.id == agent && !row.ended)
                .is_some_and(|row| {
                    !matches!(
                        super::missions::worker_life(row, &sleeping),
                        crate::delivery::answer::Runtime::Ended
                    )
                })
        };
        let Some(mission) = self.missions.book.get_mut(id) else {
            return false;
        };
        let Some(row) = mission.node(node) else {
            return false;
        };
        let Some(signal) = row.check_signal() else {
            return false;
        };
        let mut asked: Vec<String> = Vec::new();
        let mut needed: Vec<String> = Vec::new();
        for line in row
            .checker_lines()
            .filter(|line| line.state != CriterionState::Skipped)
        {
            let bound: Vec<&str> = line
                .checkers
                .iter()
                .filter_map(|checker| checker.agent_id.as_deref())
                .collect();
            let alive: Vec<&str> = bound.iter().copied().filter(|agent| live(agent)).collect();
            if alive.is_empty() || alive.len() < bound.len() {
                needed.push(line.key.clone());
            }
            for agent in alive {
                if !asked.iter().any(|kept| kept == agent) {
                    asked.push(agent.to_owned());
                }
            }
        }
        let fact = match &signal.commit {
            Some(commit) => commit.get(..12).unwrap_or(commit).to_owned(),
            None => signal.at_unix_ms.to_string(),
        };
        let mut woke = false;
        for agent in &asked {
            let mut wake = super::missions::wake_about(
                node.to_owned(),
                None,
                WakeEvent::NewHead,
                signal.commit.clone(),
                Some(format!("check_asked:{node}:{fact}")),
            );
            wake.to = Some(agent.clone());
            wake.round = Some(RoundWake::CheckAsked);
            woke |= super::missions::record(mission, wake).is_some();
        }
        if !needed.is_empty() {
            let mut wake = super::missions::wake_about(
                node.to_owned(),
                None,
                WakeEvent::NewHead,
                signal.commit.clone(),
                Some(format!("checker_needed:{node}:{fact}:{}", needed.join("+"))),
            );
            wake.round = Some(RoundWake::CheckerNeeded);
            // One line names it; both of the node's, none.
            if let [line] = needed.as_slice() {
                wake.line = Some(Dependency {
                    node: node.to_owned(),
                    line: Some(line.clone()),
                });
            }
            woke |= super::missions::record(mission, wake).is_some();
        }
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.round_opened",
            "mission": id, "node": node, "checkers": asked.len(),
            "lines_without_checker": needed,
        }));
        woke
    }

    /// Gives `node`'s worker its round's result once a line failed in it
    /// (B6, B8); a round with no failure tells it nothing (B9).
    fn tell_worker(&mut self, id: &str, node: &str) -> bool {
        let Some(mission) = self.missions.book.get_mut(id) else {
            return false;
        };
        let Some(row) = mission.node(node) else {
            return false;
        };
        // A node its Conductor works itself has no worker to tell (B16).
        let Some(worker) = row.agent_id.clone() else {
            return false;
        };
        let facts = RoundFacts::of(mission, row);
        if !facts.failed() {
            return false;
        }
        let since = row.check_signal().map_or(0, |signal| signal.at_unix_ms);
        let states: Vec<&str> = facts
            .lines
            .iter()
            .map(|(_, state, _)| state.as_str())
            .collect();
        let head = row.check_signal().and_then(|signal| signal.commit);
        let mut wake = super::missions::wake_about(
            node.to_owned(),
            row.current_pr(),
            WakeEvent::NewHead,
            head,
            Some(format!("checked:{node}:{since}:{}", states.join("+"))),
        );
        wake.to = Some(worker);
        wake.round = Some(RoundWake::Checked);
        let woke = super::missions::record(mission, wake).is_some();
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.round_result",
            "mission": id, "node": node, "lines": states, "recorded": woke,
        }));
        woke
    }
}
