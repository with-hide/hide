//! A mission node's finish lines (PRD mission-map-git-criteria D-09-D-16,
//! D-21, D-23): the criteria the plan names for the node, the lines Hide
//! adds (the plan's check command, the pull request's CI), the reviews and
//! verifications other agents give, the defects a later node found, and the
//! one place the operator looks before the node is done (PRD
//! mission-human-gate D-02, `human`).
//!
//! Each line keeps who recorded it. A line is recorded by its owner only:
//! the node's worker for its own lines, the check command and its defects,
//! the bound checker for its slot of a review or verification, and Hide for
//! the CI. Only the Conductor skips a line, and the check command and the CI
//! only on the operator's answered decision (D-13). The core refuses a
//! node's done while a line is unmet (`runtime/mission_criteria.rs`); this
//! module holds the values and the rules that need nothing but them.

use serde::{Deserialize, Serialize};

use crate::missions::Worker;

/// Lines of its own one node's plan names (D-09).
pub const OWN_LIMIT: usize = 3;
/// Agents one review or verification names.
pub const CHECKER_LIMIT: usize = 5;
/// Defects one node keeps; one more is refused, never evicted.
pub const DEFECT_LIMIT: usize = 16;
/// Reruns one commit gets after its check command or CI failed (D-14).
pub const RERUN_LIMIT: u8 = 1;
/// The longest text of a node's human gate, in characters (PRD
/// mission-human-gate D-13): what to look at, which a card shows whole.
pub const GATE_CHARS: usize = 80;
/// The key of a node's one human line.
pub const GATE_KEY: &str = "human";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CriterionKind {
    /// One of the node's own criteria, in the Conductor's words.
    Own,
    /// The plan's check command passed on the node's current commit.
    Check,
    /// The pull request's CI passed on its current head.
    Ci,
    Review,
    Verify,
    /// A defect a later node found, which the node's worker fixes (D-21),
    /// or the operator's words that sent a gated node back to work.
    Defect,
    /// The place the operator looks before the node is done (PRD
    /// mission-human-gate D-02): passed only by the operator's answer
    /// (`Runtime::settle_gate`), never by its owner's record or a skip.
    Human,
}

/// Where the operator's answer to a human gate came from (PRD
/// mission-human-gate D-21): the decision card, the phone, or the
/// Conductor's pane, where it relays the operator's words.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateVia {
    Card,
    Phone,
    Pane,
}

impl GateVia {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Card => "card",
            Self::Phone => "phone",
            Self::Pane => "pane",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CriterionState {
    #[default]
    Open,
    Pass,
    Fail,
    Skipped,
}

impl CriterionState {
    /// Its stored spelling, which names it in a node's resume mark.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Skipped => "skipped",
        }
    }
}

/// How several checkers decide one line (D-07).
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    #[default]
    All,
    Majority,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    Review,
    Verify,
}

impl CheckKind {
    pub(crate) fn kind(self) -> CriterionKind {
        match self {
            Self::Review => CriterionKind::Review,
            Self::Verify => CriterionKind::Verify,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Review => "review",
            Self::Verify => "verify",
        }
    }
}

/// Who recorded a line (D-23).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Worker,
    Checker,
    Conductor,
    Hide,
    /// The operator, from a raised node's `끝으로` (PRD mission-node-simple
    /// B15).
    Operator,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Recorded {
    pub role: Role,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub at_unix_ms: u64,
}

/// The operator as the one who recorded a line at `now`.
pub(crate) fn operator(now: u64) -> Recorded {
    Recorded {
        role: Role::Operator,
        name: "operator".to_owned(),
        agent_id: None,
        at_unix_ms: now,
    }
}

/// Why the Conductor went past a line (D-13). `decision` names the
/// operator's answered question it rests on, which the check command and
/// the CI need.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Skip {
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<u64>,
}

/// One agent of a review or verification: what the plan asked for, the
/// registration the Conductor bound to it, and its own answer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Checker {
    pub worker: Worker,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub state: CriterionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    /// The checker's answer in one short line of the operator's words,
    /// which the map shows before the evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_unix_ms: Option<u64>,
    /// The head the answer was given on: the commit the checker named, else
    /// the node's pull request head when it answered. A later answer on
    /// another head replaces this one in the line's verdict (`Criterion::settle`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// The bound checker's pane and its machine, as the ledger names them
    /// now (B29): the view fills them, the book never keeps them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
}

/// How many of the checkers that decide a review or verify line passed, as
/// `Criterion::settle` counts them: the one number the map draws.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Tally {
    pub passed: u32,
    pub total: u32,
}

/// A checker's answer to its line: the words it gives and the head it gave
/// them on (`--evidence`, `--summary`, `--commit`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Answer {
    pub evidence: Option<String>,
    pub summary: Option<String>,
    pub commit: Option<String>,
}

/// A review or verification the plan asks of other agents (D-07, B3).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CheckInput {
    #[serde(rename = "as")]
    pub kind: CheckKind,
    #[serde(default)]
    pub rule: Rule,
    pub by: Vec<Worker>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Criterion {
    /// `c1`-`c3` for the node's own, `check`, `ci`, `review`, `verify`,
    /// `d1`… for defects: stable while the line stays.
    pub key: String,
    pub kind: CriterionKind,
    /// The Conductor's words, for an own line or a defect; the lines Hide
    /// adds carry none and the screen words them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default)]
    pub state: CriterionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Recorded>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    /// The result in one short line of the operator's words, which the map
    /// shows before the evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// The commit the check command ran on, or the head the CI was read on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// Reruns on `commit` after a failure (D-14).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub reruns: u8,
    /// A failed CI on `commit` runs again (GitHub reads it pending).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub rerunning: bool,
    /// The CI on `commit` failed after its rerun and sent the node back to
    /// work, which it does once per head (D-15).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sent_back: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip: Option<Skip>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<Rule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checkers: Vec<Checker>,
    /// Of the checkers that decide the line, how many passed, while several
    /// were asked for. Made from `checkers` by `count_tally` on every change
    /// to them and on every restore, so it is never read from the book.
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub tally: Option<Tally>,
    /// The node whose work found a defect (D-21).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found_by: Option<String>,
    /// A review's failures recorded in a row: one more for each checker's
    /// failure the line reads failed after, none again while the line reads
    /// passed or skipped (with-hide/hide-private#552). The line keeps only
    /// each checker's last answer, so this is what remembers the rounds
    /// before it, and a replan that keeps the line keeps it too.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub fails_in_a_row: u32,
    /// Where the operator answered from, on a human line they passed and on
    /// a defect their answer to its gate wrote; the words are `evidence`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<GateVia>,
    /// On a human line: the key of the mission's decision that asks it now
    /// (`Runtime::raise_gates`), so the gate's state is read from the line
    /// and the open decision together (PRD mission-human-gate D-09).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asked: Option<u64>,
}

fn is_zero(value: &u8) -> bool {
    *value == 0
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

/// Whether two spellings name one commit: a recorded commit may be the
/// short form of the full one Git reports.
pub(crate) fn same_commit(a: &str, b: &str) -> bool {
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    short.len() >= 7
        && long
            .get(..short.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(short))
}

/// What one record or reading did to its node beyond the line.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Effect {
    pub changed: bool,
    /// A check or CI failed again on the commit its rerun was for: the node
    /// goes back to work (B13).
    pub spent: bool,
}

impl Criterion {
    pub(crate) fn new(key: impl Into<String>, kind: CriterionKind) -> Self {
        Self {
            key: key.into(),
            kind,
            text: None,
            state: CriterionState::Open,
            by: None,
            evidence: None,
            summary: None,
            commit: None,
            reruns: 0,
            rerunning: false,
            sent_back: false,
            skip: None,
            rule: None,
            checkers: Vec::new(),
            tally: None,
            found_by: None,
            fails_in_a_row: 0,
            via: None,
            asked: None,
        }
    }

    /// A ci line an earlier build saved before `sent_back` existed: failed
    /// with its rerun counted is how it recorded a spent rerun, which had
    /// sent the node back to work already. Read on every restore; a line
    /// this build saved in that state has `sent_back` set anyway.
    pub(crate) fn carry_spent_rerun(&mut self) {
        if self.kind == CriterionKind::Ci
            && self.state == CriterionState::Fail
            && self.reruns >= RERUN_LIMIT
        {
            self.sent_back = true;
        }
    }

    /// Whether the line is one only the operator's answer goes past (D-13):
    /// the check command and the CI on an answered decision, a human gate
    /// only by the operator's answer itself (`Runtime::settle_gate`).
    pub(crate) fn protected(&self) -> bool {
        matches!(
            self.kind,
            CriterionKind::Check | CriterionKind::Ci | CriterionKind::Human
        )
    }

    /// The operator passes the node's human gate with `words` from `via`
    /// (PRD mission-human-gate D-11, D-21); whether the line changed. The
    /// one change a human line takes, made only by `Runtime::settle_gate`.
    pub(crate) fn pass_gate(&mut self, words: String, via: GateVia, now: u64) -> bool {
        if self.kind != CriterionKind::Human || self.state == CriterionState::Pass {
            return false;
        }
        self.state = CriterionState::Pass;
        self.skip = None;
        self.evidence = Some(words);
        self.via = Some(via);
        self.asked = None;
        self.by = Some(operator(now));
        true
    }

    /// The state the line shows against `heads`: a check command passes
    /// only for the commit the node's work folder is at now, so one that
    /// passed on an older commit, or with no folder Hide can read, is open
    /// again (B7, D-16), unless the node's pull request landed with its CI
    /// passed on the merged head (`Heads::landed`): the commits after the
    /// check, a merge of main among them, were proved there, and a check
    /// nobody can run again on a merged branch is no question for the
    /// operator; a CI verdict holds only for the head it was read
    /// on, so once a read names another head the line is open, whatever
    /// that head's checks are known to be (D-15).
    pub fn shown(&self, heads: Heads<'_>) -> CriterionState {
        let on = |head: &str| {
            self.commit
                .as_deref()
                .is_some_and(|commit| same_commit(commit, head))
        };
        match (self.kind, self.state) {
            (CriterionKind::Check, CriterionState::Pass)
                if !heads.landed && !heads.work.is_some_and(on) =>
            {
                CriterionState::Open
            }
            (CriterionKind::Ci, CriterionState::Pass | CriterionState::Fail)
                if heads.pull.is_some_and(|head| !on(head)) =>
            {
                CriterionState::Open
            }
            (_, state) => state,
        }
    }

    /// Whether the line lets its node be done.
    pub(crate) fn met(&self, heads: Heads<'_>) -> bool {
        matches!(
            self.shown(heads),
            CriterionState::Pass | CriterionState::Skipped
        )
    }

    /// A review's or verification's state from its checkers and its rule,
    /// or skipped by the Conductor (B6).
    ///
    /// Only the checkers that were bound count: a slot the plan asked for and
    /// nobody took is no answer the line waits for. Of those, an answer given
    /// on another head than the newest answer's is an older review of
    /// older work, and the newest head's answers decide; two answers are
    /// compared by head only when both name one.
    fn settle(&mut self) {
        self.count_tally();
        if self.skip.is_some() {
            self.state = CriterionState::Skipped;
            return;
        }
        let (passed, total) = self.deciding();
        let failed = self.deciding_failed();
        self.state = match self.rule.unwrap_or_default() {
            _ if total == 0 => CriterionState::Open,
            Rule::All if failed > 0 => CriterionState::Fail,
            Rule::All if passed == total => CriterionState::Pass,
            Rule::Majority if passed * 2 > total => CriterionState::Pass,
            Rule::Majority if failed * 2 >= total => CriterionState::Fail,
            _ => CriterionState::Open,
        };
    }

    /// The checkers whose answers decide the line now: the bound ones, on
    /// the head of the newest answer.
    fn deciding_checkers(&self) -> Vec<&Checker> {
        let bound = || {
            self.checkers
                .iter()
                .filter(|checker| checker.agent_id.is_some())
        };
        let newest = bound()
            .filter(|checker| checker.state != CriterionState::Open)
            .max_by_key(|checker| checker.at_unix_ms)
            .and_then(|checker| checker.commit.as_deref());
        bound()
            .filter(|checker| match (newest, checker.commit.as_deref()) {
                (Some(newest), Some(commit)) => same_commit(newest, commit),
                _ => true,
            })
            .collect()
    }

    /// How many of the deciding checkers passed, and how many there are.
    fn deciding(&self) -> (usize, usize) {
        let current = self.deciding_checkers();
        let passed = current
            .iter()
            .filter(|checker| checker.state == CriterionState::Pass)
            .count();
        (passed, current.len())
    }

    fn deciding_failed(&self) -> usize {
        self.deciding_checkers()
            .iter()
            .filter(|checker| checker.state == CriterionState::Fail)
            .count()
    }

    /// Makes `tally` from the checkers: present while the plan asked several
    /// and one is bound, since one checker's count says nothing the line's
    /// state does not.
    pub(crate) fn count_tally(&mut self) {
        let (passed, total) = self.deciding();
        let several = self.checkers.len() > 1;
        self.tally = (several && total > 0).then_some(Tally {
            passed: passed as u32,
            total: total as u32,
        });
    }

    /// Binds registration `id`, an agent of kind `agent`, to the line's next
    /// free slot asking for that kind, and judges the line again: a checker
    /// taken after the line passed is one more it waits for.
    pub(crate) fn bind(&mut self, id: &str, agent: &str) -> Result<(), String> {
        if self
            .checkers
            .iter()
            .any(|checker| checker.agent_id.as_deref() == Some(id))
        {
            return Ok(());
        }
        let slot = self
            .checkers
            .iter_mut()
            .find(|checker| {
                checker.agent_id.is_none()
                    && hide_agent_adapter::canonical_kind(&checker.worker.agent) == agent
            })
            .ok_or("mission_checker_unknown")?;
        slot.agent_id = Some(id.to_owned());
        self.settle();
        self.count_failures(false);
        Ok(())
    }

    /// The owner's result for the line (B5, B7, B13). A check command's
    /// result names the commit it ran on; on that commit a failure gets one
    /// rerun, a pass after it counts it, and a record the same as the last
    /// one changes nothing, so a repeated command converges.
    pub(crate) fn record(
        &mut self,
        pass: bool,
        evidence: Option<String>,
        summary: Option<String>,
        commit: Option<String>,
        rerun: bool,
        by: Recorded,
    ) -> Result<Effect, String> {
        let state = if pass {
            CriterionState::Pass
        } else {
            CriterionState::Fail
        };
        match self.kind {
            CriterionKind::Check => {
                let commit = commit.ok_or("mission_commit_required")?;
                let same = self
                    .commit
                    .as_deref()
                    .is_some_and(|before| same_commit(before, &commit));
                let mut spent = false;
                if same {
                    match (self.state, state) {
                        // The same answer on the same commit is the same
                        // record, a repeated failed rerun included.
                        (before, now)
                            if before == now
                                && (!rerun
                                    || now == CriterionState::Pass
                                    || self.reruns >= RERUN_LIMIT) =>
                        {
                            return Ok(Effect::default());
                        }
                        (CriterionState::Fail, _) => {
                            if self.reruns >= RERUN_LIMIT {
                                return Err("mission_rerun_spent".into());
                            }
                            self.reruns += 1;
                            spent = !pass;
                        }
                        _ => {}
                    }
                } else {
                    self.reruns = 0;
                }
                self.commit = Some(commit);
                self.state = state;
                self.skip = None;
                self.evidence = evidence;
                self.summary = summary;
                self.by = Some(by);
                Ok(Effect {
                    changed: true,
                    spent,
                })
            }
            CriterionKind::Own | CriterionKind::Defect => {
                if self.state == state
                    && self.evidence == evidence
                    && self.summary == summary
                    && commit == self.commit
                {
                    return Ok(Effect::default());
                }
                self.state = state;
                self.skip = None;
                self.evidence = evidence;
                self.summary = summary;
                self.commit = commit;
                self.by = Some(by);
                Ok(Effect {
                    changed: true,
                    spent: false,
                })
            }
            CriterionKind::Ci | CriterionKind::Review | CriterionKind::Verify => {
                Err("mission_criterion_not_yours".into())
            }
            CriterionKind::Human => Err("mission_gate_operator_only".into()),
        }
    }

    /// A bound checker's answer for its own slot of a review or
    /// verification (B6).
    pub(crate) fn record_checker(
        &mut self,
        slot: usize,
        pass: bool,
        answer: Answer,
        now: u64,
    ) -> Result<bool, String> {
        let checker = self
            .checkers
            .get_mut(slot)
            .ok_or("mission_criterion_not_yours")?;
        let state = if pass {
            CriterionState::Pass
        } else {
            CriterionState::Fail
        };
        let Answer {
            evidence,
            summary,
            commit,
        } = answer;
        if checker.state == state
            && checker.evidence == evidence
            && checker.summary == summary
            && checker.commit == commit
        {
            return Ok(false);
        }
        checker.state = state;
        checker.evidence = evidence;
        checker.summary = summary;
        checker.commit = commit;
        checker.at_unix_ms = Some(now);
        self.settle();
        self.count_failures(!pass);
        Ok(true)
    }

    /// Follows the line's verdict with its failures in a row: a failure the
    /// review reads failed after adds one, and the line reading passed or
    /// skipped clears them, so a late failure on a line its rule passed, or
    /// one the Conductor skipped, counts nothing.
    fn count_failures(&mut self, failed: bool) {
        match self.state {
            CriterionState::Pass | CriterionState::Skipped => self.fails_in_a_row = 0,
            CriterionState::Fail if failed && self.kind == CriterionKind::Review => {
                self.fails_in_a_row = self.fails_in_a_row.saturating_add(1);
            }
            _ => {}
        }
    }

    /// The Conductor goes past the line with its reason (B11, B12); the
    /// caller has checked the decision a protected line needs.
    pub(crate) fn skip(&mut self, reason: String, decision: Option<u64>, by: Recorded) -> bool {
        let skip = Skip { reason, decision };
        if self.skip.as_ref() == Some(&skip) {
            return false;
        }
        self.skip = Some(skip);
        self.state = CriterionState::Skipped;
        self.by = Some(by);
        self.count_failures(false);
        true
    }

    /// The CI GitHub reports for the pull request's head (B8, B13, D-15):
    /// the line follows the run that finished last on that head, whoever
    /// started it. On one head the first failure gets one rerun: the next
    /// answer after it is that rerun, a pass counting it and a failure
    /// spending it; a failure after the rerun passed spends it too. Spending
    /// it sends the node back to work, once per head. A failure is read only
    /// once nothing on the head runs (`PullRequestSnapshot::ci_end`), the
    /// moment the Conductor's CI letter is sent.
    pub(crate) fn read_ci(&mut self, head: &str, end: crate::model::CiEnd, now: u64) -> Effect {
        use crate::missions::CiResult;
        use crate::model::CiEnd;
        let same = self
            .commit
            .as_deref()
            .is_some_and(|before| same_commit(before, head));
        let before = (
            self.state,
            self.reruns,
            self.rerunning,
            self.sent_back,
            self.commit.clone(),
        );
        if !same {
            self.commit = Some(head.to_owned());
            self.reruns = 0;
            self.rerunning = false;
            self.sent_back = false;
            self.state = CriterionState::Open;
            if self.skip.is_some() {
                // A skip was for the commit the operator was asked about.
                self.skip = None;
            }
        }
        let mut spent = false;
        if self.skip.is_none() {
            match end {
                CiEnd::Ended(CiResult::Passed) => {
                    let after_failure = self.state == CriterionState::Fail || self.rerunning;
                    if after_failure && self.reruns < RERUN_LIMIT {
                        self.reruns += 1;
                    }
                    self.rerunning = false;
                    self.state = CriterionState::Pass;
                }
                CiEnd::Running => {
                    // A run after a failure is the rerun; the line is open
                    // while it is queued or running.
                    self.rerunning |= self.state == CriterionState::Fail;
                    self.state = CriterionState::Open;
                }
                CiEnd::Ended(CiResult::Failed | CiResult::Cancelled) => {
                    let rerun_failed = self.rerunning;
                    let after_rerun =
                        !self.rerunning && self.state != CriterionState::Fail && self.reruns > 0;
                    if (rerun_failed || after_rerun) && !self.sent_back {
                        self.reruns = self.reruns.max(RERUN_LIMIT);
                        self.sent_back = true;
                        spent = true;
                    }
                    self.rerunning = false;
                    self.state = CriterionState::Fail;
                }
                CiEnd::NoChecks | CiEnd::Unread => {}
            }
        }
        let changed = before
            != (
                self.state,
                self.reruns,
                self.rerunning,
                self.sent_back,
                self.commit.clone(),
            );
        if changed {
            self.by = Some(Recorded {
                role: Role::Hide,
                name: "hide".into(),
                agent_id: None,
                at_unix_ms: now,
            });
        }
        Effect { changed, spent }
    }
}

/// The lines a plan makes of a node (B1-B3): its own criteria in order,
/// the check command where the plan has one, the CI GitHub reported, the
/// reviews and verifications it asks for, the human gate the node has
/// (`set_gate` changes it, PRD mission-human-gate D-02), and the defects
/// already found. A line the plan keeps the same keeps what was recorded on
/// it; the check command's line keeps it only while the command is the same
/// one (`same_check`), since a pass of one command is no pass of another.
pub(crate) fn plan_lines(
    existing: &[Criterion],
    own: &[String],
    checks: &[CheckInput],
    with_check: bool,
    same_check: bool,
) -> Vec<Criterion> {
    let mut out = Vec::new();
    for (at, text) in own.iter().enumerate() {
        let key = format!("c{}", at + 1);
        let kept = existing.iter().find(|line| {
            line.kind == CriterionKind::Own && line.key == key && line.text.as_deref() == Some(text)
        });
        out.push(kept.cloned().unwrap_or_else(|| {
            let mut line = Criterion::new(key, CriterionKind::Own);
            line.text = Some(text.clone());
            line
        }));
    }
    let of = |kind: CriterionKind| existing.iter().find(|line| line.kind == kind);
    if with_check {
        out.push(
            of(CriterionKind::Check)
                .filter(|_| same_check)
                .cloned()
                .unwrap_or_else(|| Criterion::new("check", CriterionKind::Check)),
        );
    }
    if let Some(ci) = of(CriterionKind::Ci) {
        out.push(ci.clone());
    }
    for kind in [CheckKind::Review, CheckKind::Verify] {
        let Some(input) = checks.iter().find(|check| check.kind == kind) else {
            continue;
        };
        let before = of(kind.kind());
        let mut line = Criterion::new(kind.key(), kind.kind());
        line.rule = Some(input.rule);
        line.skip = before.and_then(|line| line.skip.clone());
        line.by = before.and_then(|line| line.by.clone());
        line.fails_in_a_row = before.map_or(0, |line| line.fails_in_a_row);
        line.checkers = input
            .by
            .iter()
            .enumerate()
            .map(|(at, worker)| {
                before
                    .and_then(|line| line.checkers.get(at))
                    .filter(|checker| checker.worker == *worker)
                    .cloned()
                    .unwrap_or_else(|| Checker {
                        worker: worker.clone(),
                        agent_id: None,
                        state: CriterionState::Open,
                        evidence: None,
                        summary: None,
                        at_unix_ms: None,
                        commit: None,
                        pane_id: None,
                        machine: None,
                    })
            })
            .collect();
        line.settle();
        line.count_failures(false);
        out.push(line);
    }
    if let Some(gate) = of(CriterionKind::Human) {
        out.push(gate.clone());
    }
    out.extend(
        existing
            .iter()
            .filter(|line| line.kind == CriterionKind::Defect)
            .cloned(),
    );
    out
}

/// The node's human gate, where it has one.
pub fn gate(lines: &[Criterion]) -> Option<&Criterion> {
    lines.iter().find(|line| line.kind == CriterionKind::Human)
}

/// The node's human gate made `text`, or taken away with none (PRD
/// mission-human-gate D-15): the same text keeps the line as it stands, a
/// new one is open, and it goes before the defects. Whether it changed.
pub(crate) fn set_gate(lines: &mut Vec<Criterion>, text: Option<&str>) -> bool {
    let at = lines
        .iter()
        .position(|line| line.kind == CriterionKind::Human);
    match (at, text) {
        (Some(at), Some(text)) if lines[at].text.as_deref() == Some(text) => false,
        (None, None) => false,
        (at, text) => {
            if let Some(at) = at {
                lines.remove(at);
            }
            if let Some(text) = text {
                let mut line = Criterion::new(GATE_KEY, CriterionKind::Human);
                line.text = Some(text.to_owned());
                let place = lines
                    .iter()
                    .position(|line| line.kind == CriterionKind::Defect)
                    .unwrap_or(lines.len());
                lines.insert(place, line);
            }
            true
        }
    }
}

/// Adds a defect a later node found (B21) and answers its key; past
/// `DEFECT_LIMIT` it is refused.
pub(crate) fn add_defect(
    lines: &mut Vec<Criterion>,
    text: String,
    found_by: String,
    by: Recorded,
) -> Result<String, String> {
    push_defect(lines, text, Some(found_by), by)
}

/// Adds a defect, found by a later node or, with `found_by` none, the
/// operator's words that sent a gated node back to work (PRD
/// mission-human-gate D-11), and answers its key; past `DEFECT_LIMIT` it is
/// refused.
pub(crate) fn push_defect(
    lines: &mut Vec<Criterion>,
    text: String,
    found_by: Option<String>,
    by: Recorded,
) -> Result<String, String> {
    let defects = lines
        .iter()
        .filter(|line| line.kind == CriterionKind::Defect)
        .count();
    if defects >= DEFECT_LIMIT {
        return Err("mission_defects_too_many".into());
    }
    let next = lines
        .iter()
        .filter_map(|line| line.key.strip_prefix('d')?.parse::<u32>().ok())
        .max()
        .unwrap_or(0)
        + 1;
    let key = format!("d{next}");
    let mut line = Criterion::new(key.clone(), CriterionKind::Defect);
    line.text = Some(text);
    line.found_by = found_by;
    line.by = Some(by);
    lines.push(line);
    Ok(key)
}

/// The commits a node's lines are judged against now.
#[derive(Clone, Copy, Debug, Default)]
pub struct Heads<'a> {
    /// The commit the node's work folder is on, which a check command's
    /// line is for; `None` when Hide cannot read it.
    pub work: Option<&'a str>,
    /// The head of the node's pull request as the last read names it, which
    /// its CI line is for; `None` when no read names it.
    pub pull: Option<&'a str>,
    /// The node's pull request is merged and its CI line passed on the
    /// merged head: a check that passed on an earlier commit stands.
    pub landed: bool,
}

/// The keys of the lines that keep a node from being done against `heads`.
pub(crate) fn unmet<'a>(lines: &'a [Criterion], heads: Heads<'_>) -> Vec<&'a str> {
    lines
        .iter()
        .filter(|line| !line.met(heads))
        .map(|line| line.key.as_str())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A work folder at `head`, with no pull request read.
    fn work(head: Option<&str>) -> Heads<'_> {
        Heads {
            work: head,
            ..Heads::default()
        }
    }
    use crate::missions::CiResult;
    use crate::model::CiEnd;

    fn by(role: Role) -> Recorded {
        Recorded {
            role,
            name: "w".into(),
            agent_id: Some("agent-2".into()),
            at_unix_ms: 1,
        }
    }

    /// The line with every slot its plan asked for taken by an agent, as the
    /// Conductor's bindings leave it.
    fn bound(mut line: Criterion) -> Criterion {
        for (at, checker) in line.checkers.iter_mut().enumerate() {
            checker.agent_id = Some(format!("agent-{}", at + 10));
        }
        line.settle();
        line
    }

    fn worker(agent: &str) -> Worker {
        Worker {
            agent: agent.into(),
            profile: None,
            model: Some("m".into()),
            effort: None,
            effort_auto: false,
        }
    }

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn a_check_passed_on_an_older_commit_is_open_again() {
        let mut line = Criterion::new("check", CriterionKind::Check);
        assert_eq!(
            line.record(true, None, None, None, false, by(Role::Worker)),
            Err("mission_commit_required".into())
        );
        line.record(
            true,
            None,
            None,
            Some(A[..7].into()),
            false,
            by(Role::Worker),
        )
        .unwrap();
        assert!(line.met(work(Some(A))));
        assert_eq!(line.shown(work(Some(B))), CriterionState::Open);
        assert!(!line.met(work(Some(B))));
        // A work folder Hide cannot read proves nothing (D-16).
        assert_eq!(line.shown(work(None)), CriterionState::Open);
        // Landed with its CI passed on the merged head, the earlier pass
        // stands, whatever commit the merge left as the head.
        let landed = Heads {
            work: Some(B),
            pull: Some(B),
            landed: true,
        };
        assert!(line.met(landed));
        // A check that never passed is not made to pass by the landing.
        assert!(!Criterion::new("check", CriterionKind::Check).met(landed));
    }

    #[test]
    fn a_failed_check_gets_one_rerun_per_commit_and_a_repeat_changes_nothing() {
        let mut line = Criterion::new("check", CriterionKind::Check);
        let fail = |line: &mut Criterion, commit: &str, rerun: bool| {
            line.record(
                false,
                None,
                None,
                Some(commit.into()),
                rerun,
                by(Role::Worker),
            )
        };
        assert!(fail(&mut line, A, false).unwrap().changed);
        // The same failure sent again is the same record.
        assert_eq!(fail(&mut line, A, false).unwrap(), Effect::default());
        // Its rerun failing too sends the node back to work.
        assert_eq!(
            fail(&mut line, A, true).unwrap(),
            Effect {
                changed: true,
                spent: true
            }
        );
        assert_eq!(fail(&mut line, A, true).unwrap(), Effect::default());
        assert_eq!(
            line.record(true, None, None, Some(A.into()), true, by(Role::Worker)),
            Err("mission_rerun_spent".into())
        );
        // A new commit gets its own rerun.
        fail(&mut line, B, false).unwrap();
        let passed = line
            .record(true, None, None, Some(B.into()), true, by(Role::Worker))
            .unwrap();
        assert!(passed.changed && !passed.spent);
        assert_eq!((line.state, line.reruns), (CriterionState::Pass, 1));
    }

    #[test]
    fn a_review_follows_its_rule_and_only_its_checkers_record_it() {
        let input = CheckInput {
            kind: CheckKind::Review,
            rule: Rule::Majority,
            by: vec![worker("codex"), worker("claude"), worker("opencode")],
        };
        let lines = plan_lines(
            &[],
            &["works".into()],
            std::slice::from_ref(&input),
            true,
            true,
        );
        let keys: Vec<&str> = lines.iter().map(|line| line.key.as_str()).collect();
        assert_eq!(keys, ["c1", "check", "review"]);
        let mut review = bound(lines[2].clone());
        assert_eq!(
            review.record(true, None, None, None, false, by(Role::Worker)),
            Err("mission_criterion_not_yours".into())
        );
        review
            .record_checker(0, true, Answer::default(), 2)
            .unwrap();
        assert_eq!(review.state, CriterionState::Open);
        review
            .record_checker(1, false, Answer::default(), 2)
            .unwrap();
        assert_eq!(review.state, CriterionState::Open);
        review
            .record_checker(2, true, Answer::default(), 2)
            .unwrap();
        assert_eq!(review.state, CriterionState::Pass);
        let mut all = review.clone();
        all.rule = Some(Rule::All);
        all.settle();
        assert_eq!(all.state, CriterionState::Fail);
        // A replan that keeps a checker keeps its answer.
        let again = plan_lines(&[review], &["works".into()], &[input], false, true);
        assert_eq!(again[1].checkers[0].state, CriterionState::Pass);
        assert_eq!(again[1].state, CriterionState::Pass);
    }

    /// with-hide/hide-private#655: a slot nobody took is no answer the line
    /// waits for, and a line with no slot taken waits for the first.
    #[test]
    fn a_review_waits_on_no_slot_that_nobody_took() {
        let input = CheckInput {
            kind: CheckKind::Review,
            rule: Rule::All,
            by: vec![worker("codex"), worker("claude"), worker("opencode")],
        };
        let lines = plan_lines(&[], &["works".into()], &[input], false, true);
        let mut review = lines[1].clone();
        assert_eq!(review.state, CriterionState::Open);
        review.checkers[0].agent_id = Some("agent-10".into());
        review.checkers[1].agent_id = Some("agent-11".into());
        review
            .record_checker(0, true, Answer::default(), 1)
            .unwrap();
        assert_eq!(
            review.state,
            CriterionState::Open,
            "agent-11 has not answered"
        );
        review
            .record_checker(1, true, Answer::default(), 2)
            .unwrap();
        assert_eq!(review.state, CriterionState::Pass);
    }

    /// with-hide/hide-private#655: of answers on different heads the newest
    /// answer's head decides; answers that name no head are all current.
    #[test]
    fn a_review_is_decided_by_the_newest_head_answered() {
        let input = CheckInput {
            kind: CheckKind::Review,
            rule: Rule::All,
            by: vec![worker("codex"), worker("claude")],
        };
        let lines = plan_lines(&[], &["works".into()], &[input], false, true);
        let mut review = bound(lines[1].clone());
        let on = |head: &str| Answer {
            evidence: Some("it ran".into()),
            commit: Some(head.into()),
            ..Answer::default()
        };
        review
            .record_checker(0, false, on("a1b2c3d4e5f6"), 1)
            .unwrap();
        assert_eq!(review.state, CriterionState::Fail);
        review
            .record_checker(1, true, on("b2c3d4e5f6a1"), 2)
            .unwrap();
        assert_eq!(
            review.state,
            CriterionState::Pass,
            "the first answer is of an older head"
        );
        assert_eq!(review.fails_in_a_row, 0, "the line reads passed");
        // An answer given later decides, whatever head it names; the same
        // answer sent again is no new answer.
        assert!(
            !review
                .record_checker(1, true, on("b2c3d4e5f6a1"), 3)
                .unwrap()
        );
        let again = Answer {
            evidence: Some("again".into()),
            commit: Some("a1b2c3d4e5f6".into()),
            ..Answer::default()
        };
        review.record_checker(0, false, again, 4).unwrap();
        assert_eq!(review.state, CriterionState::Fail);
        // With no head named by anyone, every answer counts.
        let mut unnamed = bound(lines[1].clone());
        unnamed
            .record_checker(0, false, Answer::default(), 1)
            .unwrap();
        unnamed
            .record_checker(1, true, Answer::default(), 2)
            .unwrap();
        assert_eq!(unnamed.state, CriterionState::Fail);
    }

    /// with-hide/hide-private#655: a checker taken after the line passed is
    /// one more it waits for, and the count the map draws follows.
    #[test]
    fn a_checker_bound_after_the_line_passed_opens_it_again() {
        let input = CheckInput {
            kind: CheckKind::Review,
            rule: Rule::All,
            by: vec![worker("codex"), worker("claude"), worker("opencode")],
        };
        let lines = plan_lines(&[], &["works".into()], &[input], false, true);
        let mut review = lines[1].clone();
        review.bind("agent-10", "codex").unwrap();
        review
            .record_checker(0, true, Answer::default(), 1)
            .unwrap();
        assert_eq!(review.state, CriterionState::Pass);
        review.bind("agent-11", "claude").unwrap();
        assert_eq!(review.state, CriterionState::Open);
        assert_eq!(
            review.tally,
            Some(Tally {
                passed: 1,
                total: 2
            })
        );
        // The same registration again binds nothing; a kind nobody asked for
        // is refused.
        review.bind("agent-11", "claude").unwrap();
        assert_eq!(review.checkers[2].agent_id, None);
        assert_eq!(
            review.bind("agent-12", "grok"),
            Err("mission_checker_unknown".into())
        );
        review
            .record_checker(1, true, Answer::default(), 2)
            .unwrap();
        assert_eq!(review.state, CriterionState::Pass);
    }

    /// with-hide/hide-private#655: the tally counts the checkers that decide
    /// the line, so an empty slot and an answer of an older head are not in
    /// it, and a line asking one checker has none.
    #[test]
    fn the_tally_counts_the_checkers_that_decide_the_line() {
        let two_of_three = CheckInput {
            kind: CheckKind::Review,
            rule: Rule::All,
            by: vec![worker("codex"), worker("claude"), worker("opencode")],
        };
        let lines = plan_lines(&[], &["works".into()], &[two_of_three], false, true);
        let mut review = lines[1].clone();
        assert_eq!(review.tally, None, "none bound");
        review.bind("agent-10", "codex").unwrap();
        review.bind("agent-11", "claude").unwrap();
        let on = |head: &str| Answer {
            commit: Some(head.into()),
            ..Answer::default()
        };
        review
            .record_checker(0, true, on("a1b2c3d4e5f6"), 1)
            .unwrap();
        assert_eq!(
            review.tally,
            Some(Tally {
                passed: 1,
                total: 2
            })
        );
        review
            .record_checker(1, false, on("b2c3d4e5f6a1"), 2)
            .unwrap();
        assert_eq!(
            review.tally,
            Some(Tally {
                passed: 0,
                total: 1
            }),
            "the first answer is of an older head"
        );
        let one = CheckInput {
            kind: CheckKind::Review,
            rule: Rule::All,
            by: vec![worker("codex")],
        };
        let mut single = plan_lines(&[], &["works".into()], &[one], false, true)[1].clone();
        single.bind("agent-10", "codex").unwrap();
        assert_eq!(single.tally, None);
        // A line saved with a count reads it from its checkers, not the book.
        let mut saved: Criterion =
            serde_json::from_value(serde_json::to_value(&review).unwrap()).unwrap();
        assert_eq!(saved.tally, None);
        saved.count_tally();
        assert_eq!(saved.tally, review.tally);
    }

    /// with-hide/hide-private#552: each failure recorded on the review line
    /// counts one more in a row, and the line passing starts the count over;
    /// a checker passing while the line still fails does not.
    #[test]
    fn a_review_counts_its_failures_in_a_row_until_it_passes() {
        let input = CheckInput {
            kind: CheckKind::Review,
            rule: Rule::All,
            by: vec![worker("codex"), worker("claude")],
        };
        let lines = plan_lines(&[], &["works".into()], &[input], false, true);
        let mut review = bound(lines[1].clone());
        assert_eq!(review.fails_in_a_row, 0);
        let fail = |review: &mut Criterion, slot: usize, why: &str, at: u64| {
            review
                .record_checker(
                    slot,
                    false,
                    Answer {
                        evidence: Some(why.into()),
                        ..Answer::default()
                    },
                    at,
                )
                .unwrap()
        };
        assert!(fail(&mut review, 0, "round 1", 1));
        assert_eq!(review.fails_in_a_row, 1);
        // The same answer sent again is the same record.
        assert!(!fail(&mut review, 0, "round 1", 2));
        assert_eq!(review.fails_in_a_row, 1);
        // One checker passing leaves the line failed, and the count stands.
        review
            .record_checker(1, true, Answer::default(), 3)
            .unwrap();
        assert_eq!(
            (review.state, review.fails_in_a_row),
            (CriterionState::Fail, 1)
        );
        fail(&mut review, 0, "round 2", 4);
        assert_eq!(review.fails_in_a_row, 2);
        review
            .record_checker(0, true, Answer::default(), 5)
            .unwrap();
        assert_eq!(
            (review.state, review.fails_in_a_row),
            (CriterionState::Pass, 0)
        );
        fail(&mut review, 1, "round 3", 6);
        assert_eq!(review.fails_in_a_row, 1);
        // A skip goes past the line, and the count with it.
        review.skip("operator".into(), None, by(Role::Conductor));
        assert_eq!(review.fails_in_a_row, 0);
    }

    /// with-hide/hide-private#552 review: a replan that keeps the review
    /// line keeps its failures in a row, as it keeps its checkers' answers
    /// and its skip, so the same plan applied again leaves the count where
    /// the reviews put it.
    #[test]
    fn a_replan_that_keeps_the_review_keeps_its_failures_in_a_row() {
        let input = CheckInput {
            kind: CheckKind::Review,
            rule: Rule::All,
            by: vec![worker("codex")],
        };
        let plan = |existing: &[Criterion]| {
            plan_lines(
                existing,
                &["works".into()],
                std::slice::from_ref(&input),
                false,
                true,
            )
        };
        let mut lines = plan(&[]);
        lines[1] = bound(lines[1].clone());
        for round in ["round 1", "round 2"] {
            lines[1]
                .record_checker(
                    0,
                    false,
                    Answer {
                        evidence: Some(round.into()),
                        ..Answer::default()
                    },
                    1,
                )
                .unwrap();
        }
        assert_eq!(lines[1].fails_in_a_row, 2);
        let again = plan(&lines);
        assert_eq!(
            (again[1].state, again[1].fails_in_a_row),
            (CriterionState::Fail, 2)
        );
    }

    /// with-hide/hide-private#552 review: the count follows the line's own
    /// verdict, so a failure recorded on a review its rule already passed,
    /// or one the Conductor skipped, counts nothing.
    #[test]
    fn a_failure_on_a_review_already_passed_or_skipped_counts_nothing() {
        let input = CheckInput {
            kind: CheckKind::Review,
            rule: Rule::Majority,
            by: vec![worker("codex"), worker("claude"), worker("opencode")],
        };
        let lines = plan_lines(&[], &["works".into()], &[input], false, true);
        let mut passed = bound(lines[1].clone());
        passed
            .record_checker(0, true, Answer::default(), 1)
            .unwrap();
        passed
            .record_checker(1, true, Answer::default(), 2)
            .unwrap();
        let mut skipped = bound(lines[1].clone());
        skipped.skip("operator".into(), None, by(Role::Conductor));
        for review in [&mut passed, &mut skipped] {
            let settled = review.state;
            for round in ["late 1", "late 2"] {
                review
                    .record_checker(
                        2,
                        false,
                        Answer {
                            evidence: Some(round.into()),
                            ..Answer::default()
                        },
                        3,
                    )
                    .unwrap();
            }
            assert_eq!((review.state, review.fails_in_a_row), (settled, 0));
        }
    }

    /// A line saved before the count reads as none in a row.
    #[test]
    fn a_review_line_from_before_the_count_reads_none_in_a_row() {
        let line: Criterion =
            serde_json::from_str(r#"{"key":"review","kind":"review","state":"fail"}"#).unwrap();
        assert_eq!(line.fails_in_a_row, 0);
        assert!(
            !serde_json::to_string(&line)
                .unwrap()
                .contains("fails_in_a_row")
        );
    }

    /// with-hide/hide#2080 review: a run queued or running again after a
    /// failure leaves the line open, not failed, while it still counts as
    /// the rerun.
    #[test]
    fn a_ci_line_is_open_while_a_run_after_its_failure_has_not_finished() {
        let mut line = Criterion::new("ci", CriterionKind::Ci);
        line.read_ci(A, CiEnd::Ended(CiResult::Failed), 1);
        line.read_ci(A, CiEnd::Running, 2);
        assert_eq!((line.state, line.rerunning), (CriterionState::Open, true));
        assert!(
            line.read_ci(A, CiEnd::Ended(CiResult::Failed), 3).spent,
            "its rerun failed"
        );
    }

    #[test]
    fn a_ci_fail_then_pass_on_one_head_counts_its_rerun() {
        let mut line = Criterion::new("ci", CriterionKind::Ci);
        line.read_ci(A, CiEnd::Ended(CiResult::Failed), 1);
        line.read_ci(A, CiEnd::Running, 2);
        assert!(line.rerunning);
        line.read_ci(A, CiEnd::Ended(CiResult::Passed), 3);
        assert_eq!((line.state, line.reruns), (CriterionState::Pass, 1));
        // A failure on a rerun sends the node back; a new head starts over.
        let mut again = Criterion::new("ci", CriterionKind::Ci);
        again.read_ci(A, CiEnd::Ended(CiResult::Failed), 1);
        again.read_ci(A, CiEnd::Running, 2);
        assert!(again.read_ci(A, CiEnd::Ended(CiResult::Failed), 3).spent);
        again.read_ci(B, CiEnd::Running, 4);
        assert_eq!((again.state, again.reruns), (CriterionState::Open, 0));
    }

    /// B13, D-14, D-15: one rerun per head however often GitHub's runs go
    /// round, and a spent line still follows the run that finished last on
    /// its head: a run someone started again that passed is a pass
    /// (with-hide/hide #2075, #2077), and the node is sent back once.
    #[test]
    fn a_ci_gets_one_rerun_per_head_and_follows_the_last_run_after_it() {
        let mut line = Criterion::new("ci", CriterionKind::Ci);
        line.read_ci(A, CiEnd::Ended(CiResult::Failed), 1);
        line.read_ci(A, CiEnd::Running, 2);
        assert!(line.read_ci(A, CiEnd::Ended(CiResult::Failed), 3).spent);
        line.read_ci(A, CiEnd::Running, 4);
        assert!(line.read_ci(A, CiEnd::Ended(CiResult::Passed), 5).changed);
        assert_eq!(line.state, CriterionState::Pass);
        let again = line.read_ci(A, CiEnd::Ended(CiResult::Failed), 6);
        assert!(again.changed && !again.spent, "the node was sent back once");
        assert_eq!(line.state, CriterionState::Fail);
        // A pass on its rerun, then another failure on the same head,
        // spends it as well, and a later pass is still read.
        let mut passed = Criterion::new("ci", CriterionKind::Ci);
        passed.read_ci(A, CiEnd::Ended(CiResult::Failed), 1);
        passed.read_ci(A, CiEnd::Ended(CiResult::Passed), 2);
        passed.read_ci(A, CiEnd::Running, 3);
        assert!(passed.read_ci(A, CiEnd::Ended(CiResult::Failed), 4).spent);
        passed.read_ci(A, CiEnd::Ended(CiResult::Passed), 5);
        assert_eq!(passed.state, CriterionState::Pass);
        passed.read_ci(B, CiEnd::Ended(CiResult::Passed), 6);
        assert_eq!((passed.state, passed.reruns), (CriterionState::Pass, 0));
    }

    /// B2, B7: a replan with another check command opens its line, though
    /// the commit is the same.
    #[test]
    fn a_changed_check_command_opens_its_line_again() {
        let mut lines = plan_lines(&[], &["works".into()], &[], true, true);
        lines[1]
            .record(true, None, None, Some(A.into()), false, by(Role::Worker))
            .unwrap();
        assert!(plan_lines(&lines, &["works".into()], &[], true, true)[1].met(work(Some(A))));
        let changed = plan_lines(&lines, &["works".into()], &[], true, false);
        assert_eq!(changed[1].state, CriterionState::Open);
        assert_eq!(changed[1].commit, None);
    }

    #[test]
    fn a_defect_takes_the_next_key_and_the_cap_refuses_one_more() {
        let mut lines = Vec::new();
        for _ in 0..DEFECT_LIMIT {
            add_defect(&mut lines, "x".into(), "b".into(), by(Role::Conductor)).unwrap();
        }
        assert_eq!(lines.last().unwrap().key, format!("d{DEFECT_LIMIT}"));
        assert_eq!(
            add_defect(&mut lines, "x".into(), "b".into(), by(Role::Conductor)),
            Err("mission_defects_too_many".into())
        );
        assert_eq!(unmet(&lines, Heads::default()).len(), DEFECT_LIMIT);
    }
}
