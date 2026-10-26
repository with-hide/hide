//! The watch after a fix merged (PRD hide-ai-suggestions B15, D-13, D-14):
//! only sessions in checkouts shown to hold the merge count, and pull
//! requests whose head GitHub showed to have the merge commit; what is not
//! known to hold the merge does not count. The watch ends one way, in this
//! order: the same line comes back and is suggested again one step
//! stronger; else enough sessions passed and the fix worked; else its time
//! ran out and the watch ends unknown, silently. Pure: the worker loads the
//! line and its records around it, and the runtime asks GitHub.

use super::ledger::{PrRef, Record, SessionRef};
use super::lifecycle::{LineLife, Watch};
use super::notebook::{Line, LineId, Notebook, Stage};
use super::policy;
use super::store::ProjectLocation;
use std::collections::{BTreeMap, BTreeSet};

/// A watched line, as the runtime needs it to judge which checkouts hold
/// its merge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WatchedLine {
    pub project_id: String,
    pub location: ProjectLocation,
    pub line: LineId,
    pub merged_at_unix_ms: u64,
    pub base: Option<String>,
    pub holding: BTreeSet<String>,
    /// The pull request that merged, whose merge commit is asked for while
    /// `merge_commit` is unknown.
    pub pr: PrRef,
    pub merge_commit: Option<String>,
    /// Heads of the line's pull request records after the merge that GitHub
    /// was not asked about yet, at most [`policy::WATCH_HEADS`].
    pub heads: BTreeSet<String>,
}

/// How a watch ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WatchEnd {
    /// The problem came back: `문서로는 안 됐어요`, suggested again.
    Recurred,
    /// `효과 있었음`.
    Effective,
    /// Time ran out with nothing to say.
    Expired,
}

impl WatchEnd {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Recurred => "recurred",
            Self::Effective => "effective",
            Self::Expired => "expired",
        }
    }
}

/// Whether `checkout`, a session's folder, is inside one of the checkouts
/// that hold the merge.
pub(crate) fn holds(holding: &BTreeSet<String>, checkout: &str) -> bool {
    holding.iter().any(|root| {
        checkout == root
            || checkout
                .strip_prefix(root.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

/// Counts a session seen after the merge in a checkout that holds it;
/// answers whether the watch changed.
pub(crate) fn saw_session(
    line: &Line,
    life: &mut LineLife,
    session: &SessionRef,
    checkout: &str,
    at_unix_ms: u64,
) -> bool {
    if line.stage != Stage::Watching {
        return false;
    }
    let Some(watch) = life.watch.as_mut() else {
        return false;
    };
    at_unix_ms > watch.merged_at_unix_ms
        && holds(&watch.holding, checkout)
        && watch.sessions.insert(session.clone())
}

/// Keeps what GitHub said about a watched line: its merge commit, and
/// whether each head asked about has that commit. Answers about another
/// merge commit than the one the watch keeps are dropped. Answers whether
/// the watch changed.
pub(crate) fn judged(
    line: &Line,
    life: &mut LineLife,
    merge_commit: String,
    heads: BTreeMap<String, bool>,
) -> bool {
    if line.stage != Stage::Watching {
        return false;
    }
    let Some(watch) = life.watch.as_mut() else {
        return false;
    };
    let mut changed = false;
    match &watch.merge_commit {
        Some(kept) if *kept != merge_commit => return false,
        Some(_) => {}
        None => {
            watch.merge_commit = Some(merge_commit);
            changed = true;
        }
    }
    for (head, contains) in heads {
        changed |= watch.heads.insert(head, contains) != Some(contains);
    }
    changed
}

/// Whether `record`, one of a watched line's, is the problem coming back
/// with the fix in place: after the merge, in a session in a checkout that
/// holds it, or on a pull request head shown to have the merge commit.
fn came_back(watch: &Watch, record: &Record) -> bool {
    let fact = &record.fact;
    if fact.at_unix_ms <= watch.merged_at_unix_ms {
        return false;
    }
    match &fact.pr {
        Some(_) => fact
            .head
            .as_ref()
            .is_some_and(|head| watch.heads.get(head) == Some(&true)),
        None => {
            fact.session.is_some()
                && fact
                    .checkout
                    .as_deref()
                    .is_some_and(|checkout| holds(&watch.holding, checkout))
        }
    }
}

/// The heads of a watched line's pull request records after the merge that
/// GitHub has not answered about, oldest record first.
fn unasked_heads(watch: &Watch, records: &[Record]) -> BTreeSet<String> {
    let mut heads = BTreeSet::new();
    for record in records {
        let fact = &record.fact;
        let Some(head) = fact.head.as_ref().filter(|_| fact.pr.is_some()) else {
            continue;
        };
        if fact.at_unix_ms > watch.merged_at_unix_ms && !watch.heads.contains_key(head) {
            heads.insert(head.clone());
            if heads.len() == policy::WATCH_HEADS {
                break;
            }
        }
    }
    heads
}

/// Ends the watch when it is over, in B15's order, and answers how.
/// `records` are the line's own.
pub(crate) fn settle(
    line: &mut Line,
    life: &mut LineLife,
    records: &[Record],
    now: u64,
) -> Option<WatchEnd> {
    if line.stage != Stage::Watching {
        return None;
    }
    let watch = life.watch.as_ref()?;
    let recurred = records.iter().any(|record| came_back(watch, record));
    let span = if watch.ci {
        policy::WATCH_CI_SPAN_MS
    } else {
        policy::WATCH_SPAN_MS
    };
    let end = if recurred {
        WatchEnd::Recurred
    } else if watch.sessions.len() >= policy::WATCH_SESSIONS {
        WatchEnd::Effective
    } else if now.saturating_sub(watch.merged_at_unix_ms) > span {
        WatchEnd::Expired
    } else {
        return None;
    };
    match end {
        WatchEnd::Recurred => {
            // Suggested again at once with a stronger fix and the pull
            // request that did not hold (D-13).
            life.escalated_from = Some(watch.pr.clone());
            life.watch = None;
            life.attempt = None;
            life.raised_at_unix_ms = Some(now);
            line.level = (line.level + 1).min(policy::MAX_FIX_LEVEL);
            line.stage = Stage::Suggested;
        }
        WatchEnd::Effective => line.stage = Stage::Effective,
        WatchEnd::Expired => line.stage = Stage::Settled,
    }
    line.updated_at_unix_ms = now;
    Some(end)
}

/// The Project's lines being watched; `records_of` loads a line's records.
pub(crate) fn watched(
    project_id: &str,
    location: &ProjectLocation,
    notebook: &Notebook,
    lives: &BTreeMap<LineId, LineLife>,
    records_of: impl Fn(&Line) -> Vec<Record>,
) -> Vec<WatchedLine> {
    notebook
        .lines
        .iter()
        .filter(|line| line.stage == Stage::Watching)
        .filter_map(|line| {
            let watch = lives.get(&line.id)?.watch.as_ref()?;
            Some(WatchedLine {
                project_id: project_id.to_owned(),
                location: location.clone(),
                line: line.id,
                merged_at_unix_ms: watch.merged_at_unix_ms,
                base: watch.base.clone(),
                holding: watch.holding.clone(),
                pr: watch.pr.clone(),
                merge_commit: watch.merge_commit.clone(),
                heads: unasked_heads(watch, &records_of(line)),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suggestions::ledger::{Observation, RecordId, Signal};

    const MERGED: u64 = 10_000;
    const MERGE: &str = "0123456789abcdef0123456789abcdef01234567";
    /// A pull request head GitHub said has the merge commit, and one it
    /// said has not.
    const WITH_FIX: &str = "1111111111111111111111111111111111111111";
    const WITHOUT_FIX: &str = "2222222222222222222222222222222222222222";

    fn watching(ci: bool) -> (Line, LineLife) {
        let line = Line {
            id: LineId(5),
            discomfort: None,
            cause: None,
            fix: None,
            outcome: None,
            basis: Some((Signal::FailedTool, "cargo test".into())),
            records: Vec::new(),
            struck: None,
            stage: Stage::Watching,
            level: 0,
            created_at_unix_ms: 1,
            updated_at_unix_ms: 1,
        };
        let life = LineLife {
            watch: Some(Watch {
                merged_at_unix_ms: MERGED,
                pr: PrRef {
                    number: 40,
                    url: "https://github.com/acme/app/pull/40".into(),
                },
                base: Some("main".into()),
                ci,
                holding: BTreeSet::from(["/repo".to_owned()]),
                sessions: BTreeSet::new(),
                merge_commit: Some(MERGE.into()),
                heads: BTreeMap::new(),
            }),
            ..Default::default()
        };
        (line, life)
    }

    fn session(id: &str) -> SessionRef {
        SessionRef {
            agent: "claude".into(),
            id: id.into(),
        }
    }

    fn failure(at: u64, checkout: &str, pr: Option<u32>) -> Record {
        Record {
            id: RecordId(at),
            fact: Observation {
                signal: Signal::FailedTool,
                tag: "cargo test".into(),
                session: pr.is_none().then(|| session("s9")),
                pr: pr.map(|number| PrRef {
                    number,
                    url: format!("https://github.com/acme/app/pull/{number}"),
                }),
                checkout: Some(checkout.into()),
                at_unix_ms: at,
                always: false,
                head: None,
            },
        }
    }

    /// A check of pull request `number` that failed on `head` at `at`.
    fn check_failure(at: u64, number: u32, head: &str) -> Record {
        Record {
            id: RecordId(at),
            fact: Observation {
                signal: Signal::CheckFailed,
                tag: "verify".into(),
                session: None,
                pr: Some(PrRef {
                    number,
                    url: format!("https://github.com/acme/app/pull/{number}"),
                }),
                checkout: None,
                at_unix_ms: at,
                always: false,
                head: Some(head.into()),
            },
        }
    }

    #[test]
    fn a_checkout_holds_the_merge_only_inside_a_holding_root() {
        let holding = BTreeSet::from(["/repo".to_owned()]);
        assert!(holds(&holding, "/repo"));
        assert!(holds(&holding, "/repo/web"));
        assert!(!holds(&holding, "/repo-old"));
        assert!(!holds(&holding, "/other"));
    }

    /// B15: sessions count only after the merge, in a checkout that holds
    /// it, and each once; five end the watch as effective.
    #[test]
    fn enough_sessions_in_checkouts_with_the_merge_end_the_watch_effective() {
        let (mut line, mut life) = watching(false);
        assert!(!saw_session(
            &line,
            &mut life,
            &session("before"),
            "/repo",
            MERGED - 1
        ));
        assert!(!saw_session(
            &line,
            &mut life,
            &session("branch"),
            "/repo-old",
            MERGED + 1
        ));
        for id in ["a", "b", "c", "d"] {
            assert!(saw_session(
                &line,
                &mut life,
                &session(id),
                "/repo/web",
                MERGED + 1
            ));
        }
        assert!(!saw_session(
            &line,
            &mut life,
            &session("a"),
            "/repo",
            MERGED + 2
        ));
        assert_eq!(settle(&mut line, &mut life, &[], MERGED + 3), None);
        assert!(saw_session(
            &line,
            &mut life,
            &session("e"),
            "/repo",
            MERGED + 4
        ));
        assert_eq!(
            settle(&mut line, &mut life, &[], MERGED + 5),
            Some(WatchEnd::Effective)
        );
        assert_eq!(line.stage, Stage::Effective);
    }

    /// B15, D-13: the same problem in a checkout with the merge brings the
    /// line back at once, one step stronger, naming the pull request that
    /// did not hold; one in a branch from before the merge does not.
    #[test]
    fn a_recurrence_after_the_merge_suggests_the_line_again_stronger() {
        let (mut line, mut life) = watching(false);
        let stale = [
            failure(MERGED - 5, "/repo", None),
            failure(MERGED + 5, "/repo-old", None),
            failure(MERGED + 5, "/repo", Some(40)),
        ];
        assert_eq!(settle(&mut line, &mut life, &stale, MERGED + 6), None);
        assert_eq!(
            settle(
                &mut line,
                &mut life,
                &[failure(MERGED + 7, "/repo/web", None)],
                MERGED + 8
            ),
            Some(WatchEnd::Recurred)
        );
        assert_eq!(line.stage, Stage::Suggested);
        assert_eq!(line.level, 1);
        assert_eq!(life.escalated_from.as_ref().map(|pr| pr.number), Some(40));
        assert!(life.watch.is_none() && life.attempt.is_none());
    }

    /// D-14: with nothing to say, the watch ends after 30 days, a CI
    /// line's after 14, silently; a recurrence wins over enough sessions.
    #[test]
    fn a_quiet_watch_runs_out_and_a_recurrence_comes_first() {
        let (mut line, mut life) = watching(true);
        let fortnight = MERGED + policy::WATCH_CI_SPAN_MS;
        assert_eq!(settle(&mut line, &mut life, &[], fortnight), None);
        assert_eq!(
            settle(&mut line, &mut life, &[], fortnight + 1),
            Some(WatchEnd::Expired)
        );
        assert_eq!(line.stage, Stage::Settled);

        let (mut line, mut life) = watching(false);
        for id in ["a", "b", "c", "d", "e"] {
            saw_session(&line, &mut life, &session(id), "/repo", MERGED + 1);
        }
        assert_eq!(
            settle(
                &mut line,
                &mut life,
                &[failure(MERGED + 2, "/repo/web", None)],
                MERGED + 3
            ),
            Some(WatchEnd::Recurred),
            "a session's failure with the merge comes before enough sessions"
        );
    }

    /// B15: a pull request's failure after the merge counts only on a head
    /// GitHub showed to have the merge commit; one shown not to have it,
    /// or not asked about yet, does not count.
    #[test]
    fn a_pull_request_failure_counts_only_on_a_head_that_has_the_merge() {
        let (mut line, mut life) = watching(true);
        let records = [
            check_failure(MERGED - 1, 41, WITH_FIX),
            check_failure(MERGED + 1, 42, WITHOUT_FIX),
            check_failure(MERGED + 2, 43, WITH_FIX),
        ];
        let watch = life.watch.clone().unwrap();
        assert_eq!(
            unasked_heads(&watch, &records),
            BTreeSet::from([WITHOUT_FIX.to_owned(), WITH_FIX.to_owned()]),
            "only heads failing after the merge are asked about"
        );
        assert_eq!(
            settle(&mut line, &mut life, &records, MERGED + 3),
            None,
            "a head not asked about yet does not count"
        );

        assert!(judged(
            &line,
            &mut life,
            MERGE.into(),
            BTreeMap::from([(WITHOUT_FIX.to_owned(), false)])
        ));
        assert_eq!(
            settle(&mut line, &mut life, &records[..2], MERGED + 3),
            None,
            "a head without the merge commit does not count"
        );
        assert!(
            !judged(
                &line,
                &mut life,
                "3333333333333333333333333333333333333333".into(),
                BTreeMap::from([(WITH_FIX.to_owned(), true)])
            ),
            "an answer about another merge commit is dropped"
        );

        assert!(judged(
            &line,
            &mut life,
            MERGE.into(),
            BTreeMap::from([(WITH_FIX.to_owned(), true)])
        ));
        assert_eq!(
            settle(&mut line, &mut life, &records[..2], MERGED + 3),
            None,
            "the head with the fix failed only before the merge"
        );
        assert_eq!(
            settle(&mut line, &mut life, &records, MERGED + 3),
            Some(WatchEnd::Recurred),
            "a head with the merge commit failing the same way is the problem back"
        );
        assert_eq!(line.stage, Stage::Suggested);
        assert_eq!(line.level, 1);
    }
}
