//! A mission node's finish lines (PRD mission-map-git-criteria D-09-D-16,
//! D-21, D-23): the criteria the plan names for the node, the lines Hide
//! adds (the plan's check command, the pull request's CI), the reviews and
//! verifications other agents give, and the defects a later node found.
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
    /// A defect a later node found, which the node's worker fixes (D-21).
    Defect,
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
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Recorded {
    pub role: Role,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub at_unix_ms: u64,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_unix_ms: Option<u64>,
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
    /// The commit the check command ran on, or the head the CI was read on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// Reruns on `commit` after a failure (D-14).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub reruns: u8,
    /// A failed CI on `commit` runs again (GitHub reads it pending).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub rerunning: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip: Option<Skip>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<Rule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checkers: Vec<Checker>,
    /// The node whose work found a defect (D-21).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found_by: Option<String>,
}

fn is_zero(value: &u8) -> bool {
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
            commit: None,
            reruns: 0,
            rerunning: false,
            skip: None,
            rule: None,
            checkers: Vec::new(),
            found_by: None,
        }
    }

    /// Whether the line is one only the operator's answer goes past (D-13).
    pub(crate) fn protected(&self) -> bool {
        matches!(self.kind, CriterionKind::Check | CriterionKind::Ci)
    }

    /// The state the line shows when the node's work folder is at `head`: a
    /// check command that passed on an older commit is open again (B7).
    pub fn shown(&self, head: Option<&str>) -> CriterionState {
        match (self.kind, self.state, head, self.commit.as_deref()) {
            (CriterionKind::Check, CriterionState::Pass, Some(head), Some(commit))
                if !same_commit(commit, head) =>
            {
                CriterionState::Open
            }
            (_, state, ..) => state,
        }
    }

    /// Whether the line lets its node be done.
    pub(crate) fn met(&self, head: Option<&str>) -> bool {
        matches!(
            self.shown(head),
            CriterionState::Pass | CriterionState::Skipped
        )
    }

    /// A review's or verification's state from its checkers and its rule,
    /// or skipped by the Conductor (B6).
    fn settle(&mut self) {
        if self.skip.is_some() {
            self.state = CriterionState::Skipped;
            return;
        }
        let total = self.checkers.len();
        let passed = self
            .checkers
            .iter()
            .filter(|checker| checker.state == CriterionState::Pass)
            .count();
        let failed = self
            .checkers
            .iter()
            .filter(|checker| checker.state == CriterionState::Fail)
            .count();
        self.state = match self.rule.unwrap_or_default() {
            Rule::All if failed > 0 => CriterionState::Fail,
            Rule::All if passed == total => CriterionState::Pass,
            Rule::Majority if passed * 2 > total => CriterionState::Pass,
            Rule::Majority if failed * 2 >= total => CriterionState::Fail,
            _ => CriterionState::Open,
        };
    }

    /// The owner's result for the line (B5, B7, B13). A check command's
    /// result names the commit it ran on; on that commit a failure gets one
    /// rerun, a pass after it counts it, and a record the same as the last
    /// one changes nothing, so a repeated command converges.
    pub(crate) fn record(
        &mut self,
        pass: bool,
        evidence: Option<String>,
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
                self.by = Some(by);
                Ok(Effect {
                    changed: true,
                    spent,
                })
            }
            CriterionKind::Own | CriterionKind::Defect => {
                if self.state == state && self.evidence == evidence && commit == self.commit {
                    return Ok(Effect::default());
                }
                self.state = state;
                self.skip = None;
                self.evidence = evidence;
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
        }
    }

    /// A bound checker's answer for its own slot of a review or
    /// verification (B6).
    pub(crate) fn record_checker(
        &mut self,
        slot: usize,
        pass: bool,
        evidence: Option<String>,
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
        if checker.state == state && checker.evidence == evidence {
            return Ok(false);
        }
        checker.state = state;
        checker.evidence = evidence;
        checker.at_unix_ms = Some(now);
        self.settle();
        Ok(true)
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
        true
    }

    /// The CI GitHub reports for the pull request's head (B8, B13, D-15):
    /// a fail then a pass on one head counts its rerun, and a fail again
    /// after a rerun ran sends the node back to work.
    pub(crate) fn read_ci(
        &mut self,
        head: &str,
        checks: crate::model::PullRequestChecks,
        now: u64,
    ) -> Effect {
        use crate::model::PullRequestChecks as Checks;
        let same = self
            .commit
            .as_deref()
            .is_some_and(|before| same_commit(before, head));
        let before = (self.state, self.reruns, self.rerunning, self.commit.clone());
        if !same {
            self.commit = Some(head.to_owned());
            self.reruns = 0;
            self.rerunning = false;
            self.state = CriterionState::Open;
            if self.skip.is_some() {
                // A skip was for the commit the operator was asked about.
                self.skip = None;
            }
        }
        let mut spent = false;
        if self.skip.is_none() {
            match checks {
                Checks::Passing => {
                    if self.state == CriterionState::Fail {
                        self.reruns = self.reruns.saturating_add(1);
                    }
                    self.rerunning = false;
                    self.state = CriterionState::Pass;
                }
                Checks::Pending if self.state == CriterionState::Fail => self.rerunning = true,
                Checks::Pending => self.state = CriterionState::Open,
                Checks::Failed => {
                    if self.state == CriterionState::Fail && self.rerunning {
                        self.reruns = self.reruns.saturating_add(1);
                        spent = true;
                    }
                    self.rerunning = false;
                    self.state = CriterionState::Fail;
                }
                Checks::None | Checks::Unknown => {}
            }
        }
        let changed = before != (self.state, self.reruns, self.rerunning, self.commit.clone());
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
/// reviews and verifications it asks for, and the defects already found. A
/// line the plan keeps the same keeps what was recorded on it.
pub(crate) fn plan_lines(
    existing: &[Criterion],
    own: &[String],
    checks: &[CheckInput],
    with_check: bool,
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
                        at_unix_ms: None,
                    })
            })
            .collect();
        line.settle();
        out.push(line);
    }
    out.extend(
        existing
            .iter()
            .filter(|line| line.kind == CriterionKind::Defect)
            .cloned(),
    );
    out
}

/// Adds a defect a later node found (B21) and answers its key; past
/// `DEFECT_LIMIT` it is refused.
pub(crate) fn add_defect(
    lines: &mut Vec<Criterion>,
    text: String,
    found_by: String,
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
    line.found_by = Some(found_by);
    line.by = Some(by);
    lines.push(line);
    Ok(key)
}

/// The keys of the lines that keep a node from being done when its work
/// folder is at `head`.
pub(crate) fn unmet<'a>(lines: &'a [Criterion], head: Option<&str>) -> Vec<&'a str> {
    lines
        .iter()
        .filter(|line| !line.met(head))
        .map(|line| line.key.as_str())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PullRequestChecks;

    fn by(role: Role) -> Recorded {
        Recorded {
            role,
            name: "w".into(),
            agent_id: Some("agent-2".into()),
            at_unix_ms: 1,
        }
    }

    fn worker(agent: &str) -> Worker {
        Worker {
            agent: agent.into(),
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
            line.record(true, None, None, false, by(Role::Worker)),
            Err("mission_commit_required".into())
        );
        line.record(true, None, Some(A[..7].into()), false, by(Role::Worker))
            .unwrap();
        assert!(line.met(Some(A)));
        assert_eq!(line.shown(Some(B)), CriterionState::Open);
        assert!(!line.met(Some(B)));
    }

    #[test]
    fn a_failed_check_gets_one_rerun_per_commit_and_a_repeat_changes_nothing() {
        let mut line = Criterion::new("check", CriterionKind::Check);
        let fail = |line: &mut Criterion, commit: &str, rerun: bool| {
            line.record(false, None, Some(commit.into()), rerun, by(Role::Worker))
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
            line.record(true, None, Some(A.into()), true, by(Role::Worker)),
            Err("mission_rerun_spent".into())
        );
        // A new commit gets its own rerun.
        fail(&mut line, B, false).unwrap();
        let passed = line
            .record(true, None, Some(B.into()), true, by(Role::Worker))
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
        let lines = plan_lines(&[], &["works".into()], std::slice::from_ref(&input), true);
        let keys: Vec<&str> = lines.iter().map(|line| line.key.as_str()).collect();
        assert_eq!(keys, ["c1", "check", "review"]);
        let mut review = lines[2].clone();
        assert_eq!(
            review.record(true, None, None, false, by(Role::Worker)),
            Err("mission_criterion_not_yours".into())
        );
        review.record_checker(0, true, None, 2).unwrap();
        assert_eq!(review.state, CriterionState::Open);
        review.record_checker(1, false, None, 2).unwrap();
        assert_eq!(review.state, CriterionState::Open);
        review.record_checker(2, true, None, 2).unwrap();
        assert_eq!(review.state, CriterionState::Pass);
        let mut all = review.clone();
        all.rule = Some(Rule::All);
        all.settle();
        assert_eq!(all.state, CriterionState::Fail);
        // A replan that keeps a checker keeps its answer.
        let again = plan_lines(&[review], &["works".into()], &[input], false);
        assert_eq!(again[1].checkers[0].state, CriterionState::Pass);
        assert_eq!(again[1].state, CriterionState::Pass);
    }

    #[test]
    fn a_ci_fail_then_pass_on_one_head_counts_its_rerun() {
        let mut line = Criterion::new("ci", CriterionKind::Ci);
        line.read_ci(A, PullRequestChecks::Failed, 1);
        line.read_ci(A, PullRequestChecks::Pending, 2);
        assert!(line.rerunning);
        line.read_ci(A, PullRequestChecks::Passing, 3);
        assert_eq!((line.state, line.reruns), (CriterionState::Pass, 1));
        // A failure on a rerun sends the node back; a new head starts over.
        let mut again = Criterion::new("ci", CriterionKind::Ci);
        again.read_ci(A, PullRequestChecks::Failed, 1);
        again.read_ci(A, PullRequestChecks::Pending, 2);
        assert!(again.read_ci(A, PullRequestChecks::Failed, 3).spent);
        again.read_ci(B, PullRequestChecks::Pending, 4);
        assert_eq!((again.state, again.reruns), (CriterionState::Open, 0));
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
        assert_eq!(unmet(&lines, None).len(), DEFECT_LIMIT);
    }
}
