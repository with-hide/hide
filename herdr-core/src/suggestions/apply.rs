//! Applying a raised line (PRD hide-ai-suggestions B11-B14, D-17, D-28,
//! D-29): the attempt a press or the automatic mode begins, the moves the
//! runtime hands back, and what the runtime is given to start and follow.
//! Every function here is pure; the worker loads the line around it and
//! saves what it answers, so one writer keeps each attempt.

use super::ledger::{PrRef, RecordIndex, Signal};
use super::lifecycle::{Attempt, AttemptMove, AttemptPhase, AttemptPurpose, LineLife, Watch};
use super::notebook::{Line, LineId, Notebook, Outcome, Stage, Strike, StruckBy};
use super::store::ProjectLocation;
use std::collections::{BTreeMap, BTreeSet};

/// An attempt for the runtime to start: an agent that opens a pull request
/// in its own worktree, or Hide's own issue (B14).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ApplyOrder {
    pub project_id: String,
    pub location: ProjectLocation,
    pub line: LineId,
    pub started_at_unix_ms: u64,
    pub auto: bool,
    pub outcome: Outcome,
    pub branch: String,
    pub agent_name: String,
    pub brief: ApplyBrief,
    /// An issue line's title and, on a retry, since when the line's issue
    /// may already exist (the attempt's fields of the same names).
    pub issue_title: Option<String>,
    pub issue_since: Option<u64>,
    /// What the agent is asked to do on the branch.
    pub purpose: AttemptPurpose,
    /// The agent of the ready branch a new agent takes over, which ends
    /// first so one worktree never holds two of Hide's agents.
    pub previous_agent: Option<String>,
}

/// How an attempt begins (PRD hide-ai-branch-first-apply D-01, D-06, B12).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Begin {
    /// A new suggestion, pressed or by the automatic mode: its fix is
    /// committed on a branch of its own.
    New,
    /// `다시 시도` after a failure: the same branch, the same ask.
    Retry,
    /// `PR 올리기` on a ready branch: the same branch is pushed and one pull
    /// request opened.
    OpenPr,
    /// `다시 맞추기` on a ready branch the base moved under: the same branch
    /// is made again on the base as it is now.
    Rebase,
}

/// What the line says, for the agent's prompt or the issue's text.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ApplyBrief {
    pub discomfort: Option<String>,
    pub cause: Option<String>,
    pub fix: Option<String>,
    pub basis: Option<(Signal, String)>,
    /// How strong a fix the line asks for (D-13): 0 a document, 1 a script
    /// or check, 2 a hook.
    pub level: u8,
    /// The pull requests the line's records name, newest first.
    pub prs: Vec<u32>,
    /// How many records the line rests on.
    pub records: usize,
    /// The pull request a retry continues on the same branch.
    pub pr: Option<PrRef>,
    /// The branch a ready branch is measured against, which `다시 맞추기`
    /// makes it again on.
    pub base: Option<String>,
}

/// An attempt the runtime follows until it settles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FollowedAttempt {
    pub project_id: String,
    pub location: ProjectLocation,
    pub line: LineId,
    pub started_at_unix_ms: u64,
    pub outcome: Outcome,
    pub phase: AttemptPhase,
    pub branch: String,
    pub agent_id: Option<String>,
    pub repository: Option<String>,
    pub pr: Option<PrRef>,
    pub issue_url: Option<String>,
    pub issue_title: Option<String>,
    pub issue_since: Option<u64>,
    pub reviewed: bool,
    pub purpose: AttemptPurpose,
    pub branch_facts: Option<super::lifecycle::BranchSummary>,
    /// Why the last merge of its ready branch did not happen.
    pub failure: Option<String>,
}

/// Why a line cannot be applied now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NotReady {
    /// Apply needs a new suggestion; retry needs a failed attempt.
    Stage,
    /// A global instruction line is written by the worker, not an agent.
    Local,
}

/// Begins an attempt on `line`: a new suggestion on a press or by itself,
/// a failed attempt on `다시 시도`, which continues the same branch, pull
/// request and ask (B12), or a ready branch on `PR 올리기` or `다시 맞추기`,
/// which start an agent on that same branch (PRD hide-ai-branch-first-apply
/// D-06, B12). An issue line's retry keeps the same title, looked for since
/// the first try before another is made (B14). The line moves to 반영 중 and
/// its attempt is `starting`.
pub(crate) fn begin(
    line: &mut Line,
    life: &mut LineLife,
    index: &RecordIndex,
    how: Begin,
    auto: bool,
    now: u64,
) -> Result<(Outcome, String, String, ApplyBrief, AttemptPurpose), NotReady> {
    let previous = life.attempt.as_ref();
    let continues = previous.filter(|attempt| match how {
        Begin::New => false,
        Begin::Retry => matches!(attempt.phase, AttemptPhase::Failed | AttemptPhase::CiFailed),
        Begin::OpenPr | Begin::Rebase => attempt.phase == AttemptPhase::Ready,
    });
    let ready = match how {
        Begin::New => line.stage == Stage::Suggested,
        _ => line.stage == Stage::Applying && continues.is_some(),
    };
    if !ready || line.struck.is_some() {
        return Err(NotReady::Stage);
    }
    let outcome = line.outcome.unwrap_or(Outcome::Pr);
    if outcome == Outcome::Local {
        return Err(NotReady::Local);
    }
    if outcome == Outcome::Issue && matches!(how, Begin::OpenPr | Begin::Rebase) {
        return Err(NotReady::Stage);
    }
    let purpose = match (how, continues) {
        (Begin::OpenPr, _) => AttemptPurpose::Pr,
        (Begin::Rebase, _) => AttemptPurpose::Rebase,
        // A rebase that failed is asked again as a rebase; any other retry
        // asks what the failed attempt asked.
        (Begin::Retry, Some(attempt)) => attempt.purpose,
        _ => AttemptPurpose::Branch,
    };
    let stamp = base36(now);
    let branch = continues.map_or_else(
        || format!("hide-suggestion-s{}-{stamp}", line.id.0),
        |attempt| attempt.branch.clone(),
    );
    let agent_name = format!("suggestion-apply-s{}-{stamp}", line.id.0);
    let pr = continues.and_then(|attempt| attempt.pr.clone());
    let repository = continues.and_then(|attempt| attempt.repository.clone());
    let branch_facts = continues.and_then(|attempt| attempt.branch_facts.clone());
    let mut brief = brief(line, index, pr.clone());
    brief.base = branch_facts.as_ref().map(|facts| facts.base.clone());
    let (issue_title, issue_since) = match outcome {
        Outcome::Pr | Outcome::Local => (None, None),
        Outcome::Issue => (
            continues
                .and_then(|attempt| attempt.issue_title.clone())
                .or_else(|| Some(issue_text(&brief).0)),
            continues.map(|attempt| attempt.issue_since.unwrap_or(attempt.started_at_unix_ms)),
        ),
    };
    // An automatic attempt keeps holding the next one until the operator
    // decides, whichever step runs now.
    let auto = auto || continues.is_some_and(|attempt| attempt.auto);
    life.attempt = Some(Attempt {
        started_at_unix_ms: now,
        auto,
        phase: AttemptPhase::Starting,
        branch: branch.clone(),
        agent_name: agent_name.clone(),
        agent_id: None,
        repository,
        pr,
        issue_url: None,
        issue_title,
        issue_since,
        failure: None,
        reviewed: false,
        purpose,
        branch_facts,
    });
    line.stage = Stage::Applying;
    line.updated_at_unix_ms = now;
    Ok((outcome, branch, agent_name, brief, purpose))
}

/// An issue's title and body (B14, D-29): the check or command the line
/// rests on and the pull requests it happened in, and nothing an agent's
/// session said, whatever the repository's visibility.
pub(crate) fn issue_text(brief: &ApplyBrief) -> (String, String) {
    let (signal, tag) = brief
        .basis
        .clone()
        .unwrap_or((Signal::FailedTool, "unknown".to_owned()));
    let title = match signal {
        Signal::CheckFailed => format!("CI check `{tag}` keeps failing"),
        Signal::ChangeRequest => format!("Reviews keep asking for changes: `{tag}`"),
        _ => format!("Recurring failure: `{tag}`"),
    };
    let mut body = vec![format!(
        "Hide found this {} recurring in {} records.",
        signal.as_str(),
        brief.records
    )];
    if !brief.prs.is_empty() {
        body.push(format!(
            "Pull requests: {}",
            brief
                .prs
                .iter()
                .map(|number| format!("#{number}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    (title, body.join("\n\n"))
}

fn brief(line: &Line, index: &RecordIndex, pr: Option<PrRef>) -> ApplyBrief {
    let mut entries = line
        .records
        .iter()
        .filter_map(|id| index.get(*id))
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.at_unix_ms));
    let mut seen = BTreeSet::new();
    let prs = entries
        .iter()
        .filter_map(|entry| entry.pr)
        .filter(|number| seen.insert(*number))
        .take(super::policy::BRIEF_PRS)
        .collect();
    ApplyBrief {
        discomfort: line.discomfort.clone(),
        cause: line.cause.clone(),
        fix: line.fix.clone().or_else(|| said_fix(line, index)),
        basis: line.basis.clone(),
        level: line.level,
        prs,
        records: line.records.len(),
        pr,
        base: None,
    }
}

/// What a line of the operator's repeated request or ignored card asks the
/// repository for: the line Memory named, as a skill when it is a procedure
/// (PRD memory-self-improve D-20).
fn said_fix(line: &Line, index: &RecordIndex) -> Option<String> {
    let text = super::read::local_line(line, index)?;
    let procedure = line
        .records
        .iter()
        .filter_map(|id| index.get(*id))
        .any(|entry| entry.said.as_ref().is_some_and(|said| said.procedure));
    Some(if procedure {
        format!(
            "a skill the agents can follow for this procedure the operator keeps asking for: {text}"
        )
    } else {
        format!("a rule the agents always follow, which the operator keeps asking for: {text}")
    })
}

/// Saves a move onto the attempt it names; answers whether anything
/// changed. A move for an earlier attempt, or for a line no longer being
/// applied, changes nothing.
pub(crate) fn moved(
    line: &mut Line,
    life: &mut LineLife,
    started_at_unix_ms: u64,
    change: AttemptMove,
    now: u64,
) -> bool {
    if line.stage != Stage::Applying {
        return false;
    }
    let Some(attempt) = life
        .attempt
        .as_mut()
        .filter(|attempt| attempt.started_at_unix_ms == started_at_unix_ms)
    else {
        return false;
    };
    let before = attempt.clone();
    match change {
        AttemptMove::Started {
            agent_id,
            repository,
        } => {
            if attempt.phase == AttemptPhase::Starting {
                attempt.phase = AttemptPhase::Working;
            }
            attempt.agent_id = Some(agent_id);
            if repository.is_some() {
                attempt.repository = repository;
            }
        }
        AttemptMove::Ready(facts) => {
            // A pull request the agent opened anyway is followed as one; a
            // ready branch read again keeps the merge refusal it had unless
            // its commit moved.
            match attempt.phase {
                AttemptPhase::Starting | AttemptPhase::Working => attempt.failure = None,
                AttemptPhase::Ready => {
                    if attempt.branch_facts.as_ref().map(|old| &old.tip) != Some(&facts.tip) {
                        attempt.failure = None;
                    }
                }
                _ => return false,
            }
            attempt.phase = AttemptPhase::Ready;
            attempt.branch_facts = Some(facts);
        }
        AttemptMove::DecisionRefused(code) => {
            if attempt.phase != AttemptPhase::Ready {
                return false;
            }
            attempt.failure = Some(code);
        }
        AttemptMove::Discarded => {
            attempt.phase = AttemptPhase::Closed;
            line.struck = Some(Strike {
                by: StruckBy::Operator,
                reason: None,
                at_unix_ms: now,
            });
            line.stage = Stage::Candidate;
        }
        AttemptMove::PrOpen(pr) => {
            attempt.phase = AttemptPhase::PrOpen;
            attempt.pr = Some(pr);
            attempt.failure = None;
        }
        AttemptMove::CiFailed(pr) => {
            attempt.phase = AttemptPhase::CiFailed;
            attempt.pr = Some(pr);
        }
        AttemptMove::Reviewed => attempt.reviewed = true,
        AttemptMove::IssueOpen { url } => {
            attempt.phase = AttemptPhase::IssueOpen;
            attempt.issue_url = Some(url);
        }
        AttemptMove::Failed(code) => {
            attempt.phase = AttemptPhase::Failed;
            attempt.failure = Some(code);
        }
        AttemptMove::Merged {
            pr,
            at_unix_ms,
            base,
            merge_commit,
        } => {
            attempt.phase = AttemptPhase::Merged;
            attempt.reviewed = true;
            attempt.failure = None;
            if pr.is_some() {
                attempt.pr = pr.clone();
            }
            life.watch = Some(Watch {
                merged_at_unix_ms: at_unix_ms,
                pr,
                base,
                holding: BTreeSet::new(),
                ci: line
                    .basis
                    .as_ref()
                    .is_some_and(|(signal, _)| *signal == Signal::CheckFailed),
                sessions: BTreeSet::new(),
                merge_commit,
                heads: BTreeMap::new(),
            });
            line.stage = Stage::Watching;
        }
        AttemptMove::Closed => {
            attempt.phase = AttemptPhase::Closed;
            // Closing the pull request unmerged is the operator's word that
            // the fix is not wanted now: the line is skipped as `넘어가기`
            // skips it.
            line.struck = Some(Strike {
                by: StruckBy::Operator,
                reason: None,
                at_unix_ms: now,
            });
            line.stage = Stage::Candidate;
        }
        AttemptMove::IssueClosed => {
            attempt.phase = AttemptPhase::Closed;
            line.stage = Stage::Settled;
        }
    }
    let changed = life.attempt.as_ref() != Some(&before) || line.stage != Stage::Applying;
    if changed {
        line.updated_at_unix_ms = now;
    }
    changed
}

/// The new suggestion the automatic mode applies next (B13): the one raised
/// first, while no automatic attempt waits for its first review (D-29).
pub(crate) fn next_auto(notebook: &Notebook, lives: &BTreeMap<LineId, LineLife>) -> Option<LineId> {
    if super::read::unreviewed_auto_pr(notebook, lives) {
        return None;
    }
    notebook
        .lines
        .iter()
        // A global instruction line waits for the operator's press whatever
        // the mode (D-33), so it never holds up the pull requests behind it.
        .filter(|line| {
            line.stage == Stage::Suggested
                && line.struck.is_none()
                && line.outcome != Some(Outcome::Local)
        })
        .min_by_key(|line| {
            (
                lives
                    .get(&line.id)
                    .and_then(|life| life.raised_at_unix_ms)
                    .unwrap_or(u64::MAX),
                line.id,
            )
        })
        .map(|line| line.id)
}

/// The Project's attempts still to follow.
pub(crate) fn followed(
    project_id: &str,
    location: &ProjectLocation,
    notebook: &Notebook,
    lives: &BTreeMap<LineId, LineLife>,
) -> Vec<FollowedAttempt> {
    notebook
        .lines
        .iter()
        .filter(|line| line.stage == Stage::Applying)
        .filter_map(|line| {
            let attempt = lives.get(&line.id)?.attempt.as_ref()?;
            attempt.phase.followed().then(|| FollowedAttempt {
                project_id: project_id.to_owned(),
                location: location.clone(),
                line: line.id,
                started_at_unix_ms: attempt.started_at_unix_ms,
                outcome: line.outcome.unwrap_or(Outcome::Pr),
                phase: attempt.phase,
                branch: attempt.branch.clone(),
                agent_id: attempt.agent_id.clone(),
                repository: attempt.repository.clone(),
                pr: attempt.pr.clone(),
                issue_url: attempt.issue_url.clone(),
                issue_title: attempt.issue_title.clone(),
                issue_since: attempt.issue_since,
                reviewed: attempt.reviewed,
                purpose: attempt.purpose,
                branch_facts: attempt.branch_facts.clone(),
                failure: attempt.failure.clone(),
            })
        })
        .collect()
}

fn base36(mut value: u64) -> String {
    let mut digits = Vec::new();
    loop {
        digits.push(char::from_digit((value % 36) as u32, 36).unwrap_or('0'));
        value /= 36;
        if value == 0 {
            break;
        }
    }
    digits.iter().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(stage: Stage) -> Line {
        Line {
            id: LineId(4),
            discomfort: Some("the same check fails".into()),
            cause: None,
            fix: None,
            outcome: None,
            basis: Some((Signal::CheckFailed, "verify".into())),
            records: Vec::new(),
            struck: None,
            stage,
            level: 0,
            created_at_unix_ms: 1,
            updated_at_unix_ms: 1,
        }
    }

    fn pr(number: u32) -> PrRef {
        PrRef {
            number,
            url: format!("https://github.com/acme/app/pull/{number}"),
        }
    }

    /// B11, B12: a press begins one attempt on its own branch; a retry
    /// after a failure continues the same branch and pull request; neither
    /// begins on a line in the wrong place.
    /// B14: a retry of an issue line keeps the first try's title and looks
    /// for its issue from the first try on, however many retries came since.
    #[test]
    fn an_issue_retry_keeps_the_title_and_looks_since_the_first_try() {
        let mut line = line(Stage::Suggested);
        line.outcome = Some(Outcome::Issue);
        let index = RecordIndex::default();
        let mut life = LineLife::default();
        begin(&mut line, &mut life, &index, Begin::New, false, 100).unwrap();
        let first = life.attempt.clone().unwrap();
        assert_eq!(
            first.issue_title.as_deref(),
            Some("CI check `verify` keeps failing")
        );
        assert_eq!(
            first.issue_since, None,
            "a first try has nothing to look for"
        );
        for retried in [300, 400] {
            life.attempt.as_mut().unwrap().phase = AttemptPhase::Failed;
            begin(&mut line, &mut life, &index, Begin::Retry, false, retried).unwrap();
            let attempt = life.attempt.as_ref().unwrap();
            assert_eq!(attempt.issue_title, first.issue_title);
            assert_eq!(attempt.issue_since, Some(100), "since the first try");
        }
    }

    #[test]
    fn an_attempt_begins_on_a_new_suggestion_and_a_retry_continues_its_branch() {
        let index = RecordIndex::default();
        let mut suggested = line(Stage::Suggested);
        let mut life = LineLife::default();
        let (outcome, branch, agent, _, purpose) =
            begin(&mut suggested, &mut life, &index, Begin::New, false, 1_000).unwrap();
        assert_eq!(outcome, Outcome::Pr);
        assert_eq!(branch, "hide-suggestion-s4-rs");
        assert_eq!(agent, "suggestion-apply-s4-rs");
        assert_eq!(purpose, AttemptPurpose::Branch, "a fix starts on a branch");
        assert_eq!(suggested.stage, Stage::Applying);
        assert_eq!(life.attempt.as_ref().unwrap().phase, AttemptPhase::Starting);
        assert_eq!(
            begin(&mut suggested, &mut life, &index, Begin::New, false, 2_000),
            Err(NotReady::Stage),
            "already applying"
        );
        assert_eq!(
            begin(
                &mut suggested,
                &mut life,
                &index,
                Begin::Retry,
                false,
                2_000
            ),
            Err(NotReady::Stage),
            "nothing failed"
        );

        assert!(moved(
            &mut suggested,
            &mut life,
            1_000,
            AttemptMove::CiFailed(pr(9)),
            3_000
        ));
        let (_, again, _, brief, _) = begin(
            &mut suggested,
            &mut life,
            &index,
            Begin::Retry,
            false,
            4_000,
        )
        .unwrap();
        assert_eq!(again, branch);
        assert_eq!(brief.pr, Some(pr(9)));
        let attempt = life.attempt.as_ref().unwrap();
        assert_eq!(
            (attempt.started_at_unix_ms, attempt.phase),
            (4_000, AttemptPhase::Starting)
        );
    }

    /// A move names its attempt by its start: one meant for an earlier
    /// attempt changes nothing; a merge starts the watch, and a pull
    /// request closed unmerged skips the line as the operator's `넘어가기`.
    #[test]
    fn a_move_lands_only_on_its_own_attempt() {
        let index = RecordIndex::default();
        let mut applying = line(Stage::Suggested);
        let mut life = LineLife::default();
        begin(&mut applying, &mut life, &index, Begin::New, true, 1_000).unwrap();
        assert!(!moved(
            &mut applying,
            &mut life,
            999,
            AttemptMove::PrOpen(pr(7)),
            2_000
        ));
        assert!(moved(
            &mut applying,
            &mut life,
            1_000,
            AttemptMove::Started {
                agent_id: "a1".into(),
                repository: Some("acme/app".into())
            },
            2_000
        ));
        assert!(life.attempt.as_ref().unwrap().holds_next_auto());
        assert!(moved(
            &mut applying,
            &mut life,
            1_000,
            AttemptMove::Reviewed,
            3_000
        ));
        assert!(!life.attempt.as_ref().unwrap().holds_next_auto());
        assert!(!moved(
            &mut applying,
            &mut life,
            1_000,
            AttemptMove::Reviewed,
            3_500
        ));

        let mut merged = applying.clone();
        let mut merged_life = life.clone();
        assert!(moved(
            &mut merged,
            &mut merged_life,
            1_000,
            AttemptMove::Merged {
                pr: Some(pr(7)),
                at_unix_ms: 4_000,
                base: Some("main".into()),
                merge_commit: None,
            },
            4_000
        ));
        assert_eq!(merged.stage, Stage::Watching);
        let watch = merged_life.watch.unwrap();
        assert!(watch.ci, "a CI line's watch");
        assert_eq!(watch.pr, Some(pr(7)));
        assert_eq!(watch.base.as_deref(), Some("main"));

        assert!(moved(
            &mut applying,
            &mut life,
            1_000,
            AttemptMove::Closed,
            5_000
        ));
        assert_eq!(applying.stage, Stage::Candidate);
        assert_eq!(applying.struck.unwrap().by, StruckBy::Operator);
    }

    /// memory-self-improve D-33: a global instruction line raised first
    /// waits for the operator's press and the automatic mode takes the pull
    /// request behind it.
    #[test]
    fn the_automatic_mode_passes_over_a_global_instruction_line() {
        let mut notebook = Notebook::default();
        let mut lives = BTreeMap::new();
        for (id, raised, outcome) in [(1, 100, Outcome::Local), (2, 200, Outcome::Pr)] {
            let mut suggested = line(Stage::Suggested);
            suggested.id = LineId(id);
            suggested.outcome = Some(outcome);
            notebook.lines.push(suggested);
            lives.insert(
                LineId(id),
                LineLife {
                    raised_at_unix_ms: Some(raised),
                    ..Default::default()
                },
            );
        }
        assert_eq!(next_auto(&notebook, &lives), Some(LineId(2)));
    }

    /// B13, D-29: the automatic mode applies the suggestion raised first,
    /// and none while an automatic attempt waits for its first review.
    #[test]
    fn the_automatic_mode_waits_for_the_last_automatic_pull_requests_review() {
        let mut notebook = Notebook::default();
        let mut lives = BTreeMap::new();
        for (id, raised) in [(1, 300), (2, 100), (3, 200)] {
            let mut suggested = line(Stage::Suggested);
            suggested.id = LineId(id);
            notebook.lines.push(suggested);
            lives.insert(
                LineId(id),
                LineLife {
                    raised_at_unix_ms: Some(raised),
                    ..Default::default()
                },
            );
        }
        assert_eq!(next_auto(&notebook, &lives), Some(LineId(2)));
        let index = RecordIndex::default();
        let life = lives.get_mut(&LineId(2)).unwrap();
        begin(
            &mut notebook.lines[1],
            life,
            &index,
            Begin::New,
            true,
            1_000,
        )
        .unwrap();
        assert_eq!(next_auto(&notebook, &lives), None);
        let life = lives.get_mut(&LineId(2)).unwrap();
        moved(
            &mut notebook.lines[1],
            life,
            1_000,
            AttemptMove::PrOpen(pr(5)),
            2_000,
        );
        assert_eq!(next_auto(&notebook, &lives), None, "open and unreviewed");
        let life = lives.get_mut(&LineId(2)).unwrap();
        moved(
            &mut notebook.lines[1],
            life,
            1_000,
            AttemptMove::Reviewed,
            3_000,
        );
        assert_eq!(next_auto(&notebook, &lives), Some(LineId(3)));

        let location = ProjectLocation {
            node: "test-node".into(),
            path: "/repo".into(),
        };
        let followed = followed("p", &location, &notebook, &lives);
        assert_eq!(followed.len(), 1);
        assert_eq!(followed[0].line, LineId(2));
        assert_eq!(followed[0].pr, Some(pr(5)));
        assert!(followed[0].reviewed);
    }
}
