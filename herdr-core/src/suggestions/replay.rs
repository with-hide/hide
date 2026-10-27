//! The calibration replay (PRD hide-ai-suggestions D-12): a past month of
//! this machine's records run day by day through the ledger's fold, the
//! shipped grouping, the scores and the caps, to see how many suggestions
//! the starting values would have raised. It prints counts only; no session
//! text leaves the readers.
//!
//! Run it on purpose:
//!
//! ```text
//! HIDE_SUGGESTIONS_REPLAY_PROJECTS=/path/to/repo:/path/to/other \
//!   scripts/verify-cargo.sh test-scoped -p herdr-core --lib \
//!   suggestions::replay -- --ignored --nocapture
//! ```
//!
//! `HIDE_SUGGESTIONS_REPLAY_DAYS` (default 30) sets the span, and
//! `HIDE_SUGGESTIONS_REPLAY_EXTRA` names a JSON Lines file of further
//! records the local session files do not hold (a pull request's failed
//! checks, the Memory cards that stand for corrections), each an
//! [`Observation`] with a `project` path beside it.
//!
//! What it reads as the live core does: each session's tool calls left
//! failed at a turn's end. What it reads less often than the live core: a
//! question left for the operator is seen only when a read ends on it,
//! where the live label worker reads after every change, so waits are
//! undercounted.

use super::analysis::{Analysis, AnalysisBudget, AnalysisInput, MemoryPen, StopFlag, TagGrouping};
use super::ledger::{Observation, RecordEntry, RecordId, RecordIndex, SessionRef, Signal};
use super::notebook::{Notebook, Stage};
use super::policy::{self, Quantity};
use super::scoring;
use hide_session::label_transcript::{self, LabelTranscriptRequest};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The most reads one session is given, so a broken checkpoint cannot spin.
const READS_PER_SESSION: usize = 4_096;

#[derive(Default)]
struct SessionCounts {
    read: usize,
    refused: BTreeMap<String, usize>,
}

/// Every fact one Project's sessions in the span hold.
fn session_facts(
    home: &Path,
    root: &Path,
    since: u64,
    counts: &mut SessionCounts,
) -> Vec<Observation> {
    let identity = hide_project::resolve(root, "local").expect("a replay Project resolves");
    let catalog = hide_session::SessionCatalog::new(home, "local");
    let sessions = catalog
        .project_sessions(&identity)
        .expect("the Project's sessions are listed");
    let mut facts = Vec::new();
    for session in sessions.sessions {
        if session.updated_at_unix_ms < since || !session.agent.reads_tool_failures() {
            continue;
        }
        let Some(locator) = session.locator.to_str() else {
            continue;
        };
        let checkout = session.checkout_path.to_string_lossy().into_owned();
        let mut request = LabelTranscriptRequest {
            agent: session.agent,
            reference_kind: "path".into(),
            reference_value: locator.to_owned(),
            cwd: Some(checkout.clone()),
            checkpoint: None,
            subagents: BTreeMap::new(),
            turns: None,
            tools: None,
        };
        let unit = SessionRef {
            agent: session.agent.as_str().to_owned(),
            id: session.id.clone(),
        };
        let fact = |signal, tag: String, at| Observation {
            signal,
            tag,
            session: Some(unit.clone()),
            pr: None,
            checkout: Some(checkout.clone()),
            at_unix_ms: at,
            always: false,
            head: None,
            improvement: None,
        };
        let mut failed = Vec::new();
        let mut waited = None;
        let mut refused = None;
        for _ in 0..READS_PER_SESSION {
            let transcript = match label_transcript::read(home, &request) {
                Ok(transcript) => transcript,
                Err(reason) => {
                    refused = Some(reason.to_string());
                    break;
                }
            };
            failed.extend(transcript.failed_tools);
            if waited.is_none()
                && transcript
                    .turns
                    .as_ref()
                    .and_then(hide_session::turns::TurnTracker::waiting)
                    == Some(hide_session::turns::Waiting::Question)
            {
                waited = transcript.events.last().map(|event| event.at_unix_ms);
            }
            request.checkpoint = Some(transcript.checkpoint);
            request.turns = transcript.turns;
            request.tools = transcript.tools;
            request.subagents = transcript.subagents;
            if !transcript.has_more {
                if let Some(tools) = request.tools.as_mut() {
                    failed.extend(tools.end_turn());
                }
                break;
            }
        }
        if let Some(reason) = refused {
            *counts.refused.entry(reason).or_default() += 1;
            continue;
        }
        counts.read += 1;
        facts.extend(
            failed
                .into_iter()
                .filter(|failure| failure.at_unix_ms >= since)
                .map(|failure| fact(Signal::FailedTool, failure.head, failure.at_unix_ms)),
        );
        if let Some(at) = waited.filter(|at| *at >= since) {
            facts.push(fact(Signal::AgentWait, "question".into(), at));
        }
    }
    facts
}

/// The extra records file: one `{"project": path, ...Observation}` a line.
fn extra_facts(path: &Path) -> Vec<(PathBuf, Observation)> {
    #[derive(serde::Deserialize)]
    struct Extra {
        project: PathBuf,
        #[serde(flatten)]
        fact: Observation,
    }
    std::fs::read_to_string(path)
        .expect("the extra records file is read")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let extra: Extra = serde_json::from_str(line).expect("an extra record parses");
            (extra.project, extra.fact)
        })
        .collect()
}

/// What a month of one Project's records raised at one quantity.
#[derive(Default, serde::Serialize)]
struct Raised {
    /// Raised while nobody acted, so each one held its open slot.
    untouched: usize,
    /// Raised when each suggestion was acted on the day after it rose.
    handled: usize,
    /// What those handled ones rest on: signal and tag.
    kinds: Vec<(String, String)>,
}

#[derive(Default, serde::Serialize)]
struct ProjectReplay {
    records: BTreeMap<&'static str, usize>,
    lines: usize,
    run_days: usize,
    /// Lines over each threshold at the span's end.
    over: BTreeMap<String, usize>,
    raised: BTreeMap<&'static str, Raised>,
    /// The strongest lines at the span's end: signal, tag, sessions, points.
    strongest: Vec<(String, String, usize, f64)>,
}

/// Folds the facts as the ledger does (one per fact and session or pull
/// request, the first kept) and replays the days.
fn replay(mut facts: Vec<Observation>, since: u64, now: u64) -> ProjectReplay {
    facts.sort_by_key(|fact| fact.at_unix_ms);
    let mut seen = BTreeSet::new();
    let facts = facts
        .into_iter()
        .filter_map(Observation::normalized)
        .filter(|fact| fact.key().is_some_and(|key| seen.insert(key)))
        .collect::<Vec<_>>();
    let mut out = ProjectReplay::default();
    for fact in &facts {
        *out.records.entry(fact.signal.as_str()).or_default() += 1;
    }
    let entries = facts
        .iter()
        .enumerate()
        .map(|(at, fact)| RecordEntry {
            id: RecordId(at as u64 + 1),
            signal: fact.signal,
            tag: fact.tag.clone(),
            session: fact.session.clone(),
            pr: fact.pr.as_ref().map(|pr| pr.number),
            at_unix_ms: fact.at_unix_ms,
            always: fact.always,
            improvement: None,
            line: None,
            fresh: false,
        })
        .collect::<Vec<_>>();
    let days = now.saturating_sub(since).div_ceil(policy::DAY_MS);
    let ends = (1..=days).map(|day| (since + day * policy::DAY_MS).min(now));
    // The notebook does not depend on the quantity: one grouping pass a day.
    let mut notebooks = Vec::new();
    let mut notebook = Notebook::default();
    let mut taken = 0;
    for end in ends.clone() {
        let upto = entries.partition_point(|entry| entry.at_unix_ms <= end);
        let mut index = RecordIndex {
            entries: entries[..upto].to_vec(),
        };
        for entry in &mut index.entries[taken..] {
            entry.fresh = true;
        }
        if upto > taken {
            out.run_days += 1;
            let input = AnalysisInput {
                project_id: "replay".into(),
                records: index.clone(),
                language: hide_ai::OutputLanguage::English,
                budget: AnalysisBudget {
                    max_turns: policy::RUN_TURNS,
                    deadline: Instant::now() + Duration::from_secs(60),
                    stop: StopFlag::default(),
                },
            };
            let mut pen = MemoryPen::new(notebook, &index, end);
            TagGrouping.run(&input, &mut pen);
            notebook = pen.notebook;
        }
        taken = upto;
        for entry in &mut index.entries {
            entry.fresh = false;
            entry.line = notebook.line_of(entry.id);
        }
        notebooks.push((end, notebook.clone(), index));
    }
    out.lines = notebook.lines.len();
    if let Some((end, notebook, index)) = notebooks.last() {
        let mut scored = notebook
            .lines
            .iter()
            .map(|line| (line, scoring::score(line, index, *end)))
            .filter(|(line, score)| scoring::answers_the_questions(line, score))
            .collect::<Vec<_>>();
        for threshold in [2.0, 3.0, 4.0, 5.0] {
            out.over.insert(
                format!("{threshold}"),
                scored.iter().filter(|(_, s)| s.points >= threshold).count(),
            );
        }
        scored.sort_by(|a, b| b.1.points.total_cmp(&a.1.points));
        out.strongest = scored
            .iter()
            .take(8)
            .map(|(line, score)| {
                let (signal, tag) = line.basis.clone().expect("a grouped line has a basis");
                (
                    signal.as_str().to_owned(),
                    tag,
                    score.sessions,
                    (score.points * 100.0).round() / 100.0,
                )
            })
            .collect();
    }
    for (name, quantity) in [
        ("few", Quantity::Few),
        ("normal", Quantity::Normal),
        ("many", Quantity::Many),
    ] {
        let raised = out.raised.entry(name).or_default();
        for handled in [false, true] {
            let mut stage = BTreeMap::new();
            let mut raised_at = Vec::new();
            let mut quiet_since = None;
            let mut count = 0;
            for (end, notebook, index) in &notebooks {
                let open = if handled {
                    0
                } else {
                    stage.values().filter(|s| **s == Stage::Suggested).count()
                };
                let scored = notebook
                    .lines
                    .iter()
                    .filter(|line| !stage.contains_key(&line.id))
                    .map(|line| (line, scoring::score(line, index, *end)))
                    .collect::<Vec<_>>();
                raised_at.retain(|at| end.saturating_sub(*at) < policy::WEEK_MS);
                let since = *quiet_since.get_or_insert(*end);
                for id in scoring::choose(&scored, open, &raised_at, since, quantity, *end) {
                    stage.insert(id, Stage::Suggested);
                    raised_at.push(*end);
                    quiet_since = Some(*end);
                    count += 1;
                    if handled
                        && let Some((signal, tag)) =
                            notebook.line(id).and_then(|line| line.basis.clone())
                    {
                        raised.kinds.push((signal.as_str().to_owned(), tag));
                    }
                }
            }
            if handled {
                raised.handled = count;
            } else {
                raised.untouched = count;
            }
        }
    }
    out
}

#[test]
#[ignore = "reads this machine's own agent sessions: HIDE_SUGGESTIONS_REPLAY_PROJECTS"]
fn a_month_of_this_machines_records_replays_into_counts() {
    let projects = std::env::var("HIDE_SUGGESTIONS_REPLAY_PROJECTS")
        .expect("HIDE_SUGGESTIONS_REPLAY_PROJECTS names the Projects");
    let days = std::env::var("HIDE_SUGGESTIONS_REPLAY_DAYS")
        .ok()
        .map_or(30, |days| {
            days.parse::<u64>().expect("a whole number of days")
        });
    let home = PathBuf::from(std::env::var("HOME").expect("HOME is set"));
    let now = crate::runtime::unix_milliseconds();
    let since = now - days * policy::DAY_MS;
    let mut extra = std::env::var("HIDE_SUGGESTIONS_REPLAY_EXTRA")
        .ok()
        .map(|path| extra_facts(Path::new(&path)))
        .unwrap_or_default();
    let mut report = BTreeMap::new();
    let mut counts = SessionCounts::default();
    for root in std::env::split_paths(&projects) {
        let started = Instant::now();
        let mut facts = session_facts(&home, &root, since, &mut counts);
        facts.extend(
            extra
                .extract_if(.., |(project, _)| *project == root)
                .map(|(_, fact)| fact)
                .filter(|fact| fact.at_unix_ms >= since),
        );
        let replayed = replay(facts, since, now);
        eprintln!(
            "replayed {} in {:.1}s",
            root.display(),
            started.elapsed().as_secs_f64()
        );
        report.insert(root.display().to_string(), replayed);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "days": days,
            "sessions_read": counts.read,
            "sessions_refused": counts.refused,
            "extra_unmatched": extra.len(),
            "projects": report,
        }))
        .expect("the report serializes")
    );
}
