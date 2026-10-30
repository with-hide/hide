//! What each pane's agent hook last reported, held by the core.
//!
//! The hook reports through hided's hook request (`HookCommand::Status`),
//! proven to come from the pane like every other hook request, and the core
//! keeps the report here (PRD engine-bundle-1 D-7). Nothing is written to
//! Herdr, so a Herdr handoff or restart loses nothing and needs no restore
//! (D-8).
//!
//! A pane can run more than one session's hooks: the operator's own Stop hook
//! that starts `claude -p` runs Hide's hooks in the pane's environment. So a
//! pane keeps the latest report of each session it heard from, at most
//! [`SESSIONS_PER_PANE`], and a projection lays only the one for the session
//! Herdr reports for the pane's agent; an agent whose session Herdr names by
//! a path, or not at all, takes the session that reported last.
//!
//! A turn the hook reported stands until the same session's next hook event
//! (`agent_state::axes`): Herdr's reading of the screen never overrides it,
//! and no timer ends it. A turn no hook reports the end of is ended by the
//! signal the agent does give (Claude Code's `idle_prompt` notification after
//! an interrupt, Codex's `Interrupt`), and a pane whose agent Herdr no longer
//! lists drops its facts.
//!
//! The store has its own small lock, never held across I/O or while the
//! runtime lock is taken: the runtime takes it under its own lock to record a
//! report, and a device's coordinator takes it alone to lay the facts on the
//! session it is about to project. What a restart must keep (runtime,
//! version, session and counts, not the turn) is saved on a thread of its own
//! when it changes ([`LatestFile`]).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use hide_node_link::panes::{HookCounts, HookStatusReport, HookTurn};
use serde::{Deserialize, Serialize};

use super::PaneHookTokens;
use crate::latest_file::{LatestFile, read_capped};
use crate::sidebar::{SessionAgentPayload, SessionSnapshotPayload};

/// The panes the core keeps facts for. A pane leaves when its pane closes or
/// its agent ends, so this bounds panes whose agents never appeared to Herdr.
pub(crate) const PANE_LIMIT: usize = 1024;
/// The sessions one pane keeps; the one heard from least recently goes first.
pub(crate) const SESSIONS_PER_PANE: usize = 8;
const READ_CAP: u64 = 4 * 1024 * 1024;
const SCHEMA_VERSION: u32 = 1;

pub(crate) type SharedHookFacts = Arc<Mutex<HookFacts>>;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
struct SessionFact {
    runtime: String,
    version: u32,
    session: Option<String>,
    counts: Option<HookCounts>,
    #[serde(skip)]
    turn: Option<HookTurn>,
    /// Herdr's state sequence for the agent when `turn` was first laid on it:
    /// a turn end the session read finds under a later state ended this
    /// turn, which no hook event of the session may follow (Claude Code's
    /// interruption, which sends neither `Stop` nor always `idle_prompt`).
    #[serde(skip)]
    turn_seq: Option<u64>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
struct PaneFacts {
    /// Oldest first; the last is the session that reported last.
    sessions: Vec<SessionFact>,
    /// A projection laid a session on an agent Herdr listed here, so a pane
    /// that lists none afterwards has lost its agent.
    seen_agent: bool,
    /// The session id Herdr last named for the pane's agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    herdr_session: Option<String>,
    /// A hook reported since Herdr last named another session for the pane:
    /// the hook's session, proven to run in the pane, is then the pane's
    /// current one even where Herdr still names an earlier one (an agent
    /// started again in the same pane).
    #[serde(skip)]
    reported_since_herdr: bool,
}

#[derive(Deserialize, Serialize)]
struct Stored {
    schema_version: u32,
    panes: BTreeMap<String, PaneFacts>,
}

/// What recording a report did.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Recorded {
    /// A projection laid now would differ.
    pub changed: bool,
}

#[derive(Default)]
pub(crate) struct HookFacts {
    panes: BTreeMap<String, PaneFacts>,
    file: Option<LatestFile<BTreeMap<String, PaneFacts>>>,
}

impl HookFacts {
    /// The facts the last run saved under `path`, and that file for the
    /// saves to come. A missing file is a first run, which starts from the
    /// pane records the hooks keep on this machine (`seed`), so a pane whose
    /// agent last reported to Herdr stays connected; any other reason a file
    /// is not used is a diagnostic, and the next save replaces it.
    pub(crate) fn restore(
        path: PathBuf,
        seed: impl FnOnce() -> Vec<(String, Vec<hide_agent_hooks::counters::SessionRecord>)>,
    ) -> Self {
        let mut seeded = false;
        let panes = match read_capped(&path, READ_CAP).and_then(|bytes| {
            bytes
                .map(|bytes| {
                    let stored: Stored = serde_json::from_slice(&bytes)
                        .map_err(|error| format!("could not be decoded: {error}"))?;
                    if stored.schema_version != SCHEMA_VERSION {
                        return Err(format!(
                            "has schema version {}, expected {SCHEMA_VERSION}",
                            stored.schema_version
                        ));
                    }
                    Ok(stored.panes)
                })
                .transpose()
        }) {
            Ok(Some(panes)) => panes,
            Ok(None) => {
                seeded = true;
                seed()
                    .into_iter()
                    .map(|(pane, records)| (pane, seeded_facts(records)))
                    .collect()
            }
            Err(message) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "agent_hooks",
                    "kind": "facts.discarded",
                    "path": path.to_string_lossy(),
                    "message": message,
                }));
                BTreeMap::new()
            }
        };
        let facts = Self {
            panes: panes
                .into_iter()
                .filter(|(_, facts)| {
                    !facts.sessions.is_empty() && facts.sessions.len() <= SESSIONS_PER_PANE
                })
                .take(PANE_LIMIT)
                .collect(),
            file: Some(LatestFile::new(
                path,
                "hide-hook-facts-save",
                "agent_hooks",
                "facts",
                encode,
            )),
        };
        // Saved at once, so the records are read on this first run only.
        if seeded {
            facts.save();
        }
        facts
    }

    /// Records one report from `pane`, the pane id as the core projects it.
    pub(crate) fn record(
        &mut self,
        pane: &str,
        report: &HookStatusReport,
    ) -> Result<Recorded, &'static str> {
        if !self.panes.contains_key(pane) && self.panes.len() >= PANE_LIMIT {
            return Err("hook_facts_full");
        }
        let facts = self.panes.entry(pane.to_owned()).or_default();
        let before = facts.clone();
        let same = |fact: &SessionFact| {
            fact.runtime == report.runtime
                && match (&report.session, &fact.session) {
                    (Some(reported), Some(kept)) => reported == kept,
                    // An event whose input named no session is the one the
                    // runtime last reported, as the hook's own record reads it.
                    (None, _) => true,
                    (Some(_), None) => true,
                }
        };
        let mut fact = match facts.sessions.iter().rposition(same) {
            Some(at) => facts.sessions.remove(at),
            None => SessionFact {
                runtime: report.runtime.clone(),
                version: report.version,
                session: None,
                counts: None,
                turn: None,
                turn_seq: None,
            },
        };
        facts.reported_since_herdr = true;
        let turn_before = fact.turn;
        fact.version = report.version;
        if report.session.is_some() {
            fact.session.clone_from(&report.session);
        }
        if report.counts.is_some() {
            fact.counts = report.counts;
        }
        if report.session_start {
            // A resumed or compacted session has no turn running yet.
            fact.turn = None;
        }
        fact.turn = match (report.turn, fact.turn) {
            // A prompt that waits after a finished turn is that turn's rest.
            (Some(HookTurn::Idle), Some(HookTurn::Done)) => Some(HookTurn::Done),
            (Some(turn), _) => Some(turn),
            (None, kept) => kept,
        };
        if fact.turn != turn_before {
            fact.turn_seq = None;
        }
        facts.sessions.push(fact);
        if facts.sessions.len() > SESSIONS_PER_PANE {
            facts.sessions.remove(0);
        }
        // The derived equality compares the turn too, which the file leaves out.
        let changed = *facts != before;
        if stored_part_moved(&before, facts) {
            self.save();
        }
        Ok(Recorded { changed })
    }

    /// Lays the facts on `payload`'s agents and answers each agent pane's
    /// hook tokens, by `payload`'s own pane id. `key` turns that id into the
    /// one the facts are kept under; `owns` says which kept ids this payload
    /// speaks for, so a pane it no longer lists, or lists without the agent
    /// it had, has its facts dropped.
    pub(crate) fn lay(
        &mut self,
        payload: &mut SessionSnapshotPayload,
        key: impl Fn(&str) -> String,
        owns: impl Fn(&str) -> bool,
    ) -> BTreeMap<String, PaneHookTokens> {
        let mut tokens = BTreeMap::new();
        let mut moved = false;
        for agent in &mut payload.agents {
            agent.hook = None;
            let Some(pane) = agent.pane_id.clone().or_else(|| agent.id.clone()) else {
                continue;
            };
            let Some(facts) = self.panes.get_mut(&key(&pane)) else {
                continue;
            };
            if !facts.seen_agent {
                facts.seen_agent = true;
                moved = true;
            }
            if let Some(named) = herdr_session(agent)
                && facts.herdr_session.as_deref() != Some(named)
            {
                // Herdr naming another session than before is newer than
                // every report so far; its first naming is not.
                if facts.herdr_session.is_some() {
                    facts.reported_since_herdr = false;
                }
                facts.herdr_session = Some(named.to_owned());
                moved = true;
            }
            let Some(fact) = bound(facts, agent) else {
                continue;
            };
            end_by_session_read(fact, agent);
            agent.hook = fact.turn;
            tokens.insert(
                pane,
                PaneHookTokens {
                    version: Some(fact.version),
                    working: fact.counts.map(|counts| counts.working),
                    done: fact.counts.map(|counts| counts.done),
                    blocked: None,
                },
            );
        }
        let listed: std::collections::BTreeSet<String> = payload
            .panes
            .iter()
            .map(|pane| key(&pane.pane_id))
            .collect();
        let with_agent: std::collections::BTreeSet<String> = payload
            .agents
            .iter()
            .filter_map(|agent| agent.pane_id.as_deref().or(agent.id.as_deref()))
            .map(&key)
            .collect();
        let before = self.panes.len();
        self.panes.retain(|pane, facts| {
            !owns(pane)
                || (listed.contains(pane) && (with_agent.contains(pane) || !facts.seen_agent))
        });
        if moved || self.panes.len() != before {
            self.save();
        }
        tokens
    }

    fn save(&self) {
        if let Some(file) = &self.file {
            file.save(self.panes.clone());
        }
    }
}

/// The session id Herdr names for `agent`, when it names one by id.
fn herdr_session(agent: &SessionAgentPayload) -> Option<&str> {
    agent
        .agent_session
        .as_ref()
        .filter(|session| session.kind == "id")
        .map(|session| session.value.as_str())
}

/// Ends the hook's running or waiting turn when the session read, made under
/// a later Herdr state than the one the turn was first laid under and with
/// Herdr reading the agent at rest, finds the session's last turn ended. The
/// read is the later fact, so the turn stands until the session's next hook
/// event or that read, and no timer is involved (D-9).
fn end_by_session_read(fact: &mut SessionFact, agent: &SessionAgentPayload) {
    if fact.turn.is_some() && fact.turn_seq.is_none() {
        fact.turn_seq = agent.state_change_seq;
    }
    let (Some(HookTurn::Working | HookTurn::Waiting), Some(since), Some(now)) =
        (fact.turn, fact.turn_seq, agent.state_change_seq)
    else {
        return;
    };
    let at_rest = matches!(agent.agent_status.as_deref(), Some("idle" | "done"));
    let ended = agent.facts.as_ref().and_then(|facts| facts.turn_ended);
    if now > since && at_rest {
        fact.turn = match ended {
            Some(hide_session::turns::TurnEnded::Completed) => Some(HookTurn::Done),
            Some(
                hide_session::turns::TurnEnded::Interrupted
                | hide_session::turns::TurnEnded::Unanswered,
            ) => Some(HookTurn::Idle),
            None => return,
        };
    }
}

/// The session fact that speaks for `agent`: of the agent's runtime, and the
/// session Herdr reports for it when Herdr names one by id. Where none has
/// that id and a hook reported since Herdr last changed the id it names, the
/// session that reported last speaks: the hook proves its process runs in
/// the pane, and Herdr can keep naming the session of an agent that ended
/// there.
fn bound<'a>(facts: &'a mut PaneFacts, agent: &SessionAgentPayload) -> Option<&'a mut SessionFact> {
    let runtime = hide_agent_adapter::adapter(agent.agent.as_deref()?)?.id;
    let reported = herdr_session(agent);
    let named = facts.sessions.iter().rposition(|fact| {
        fact.runtime == runtime
            && match (reported, fact.session.as_deref()) {
                (Some(reported), Some(kept)) => reported == kept,
                _ => true,
            }
    });
    let at = named.or_else(|| {
        facts
            .reported_since_herdr
            .then(|| {
                facts
                    .sessions
                    .iter()
                    .rposition(|fact| fact.runtime == runtime)
            })
            .flatten()
    })?;
    facts.sessions.get_mut(at)
}

/// A pane's facts from the records its hooks kept: no turn, which no record
/// before this one carried, and counts for an agent whose hooks count.
fn seeded_facts(records: Vec<hide_agent_hooks::counters::SessionRecord>) -> PaneFacts {
    let sessions = records
        .into_iter()
        .rev()
        .take(SESSIONS_PER_PANE)
        .rev()
        .map(|record| SessionFact {
            counts: hide_agent_adapter::adapter(&record.agent)
                .and_then(|adapter| adapter.subagent_counts)
                .map(|_| HookCounts {
                    working: record.counters.working,
                    done: record.counters.done,
                }),
            runtime: record.agent,
            version: record.version,
            session: record.session,
            turn: None,
            turn_seq: None,
        })
        .collect();
    PaneFacts {
        sessions,
        ..PaneFacts::default()
    }
}

fn stored_part_moved(before: &PaneFacts, after: &PaneFacts) -> bool {
    before.seen_agent != after.seen_agent
        || before.herdr_session != after.herdr_session
        || before.sessions.len() != after.sessions.len()
        || before
            .sessions
            .iter()
            .zip(&after.sessions)
            .any(|(left, right)| {
                (&left.runtime, left.version, &left.session, left.counts)
                    != (&right.runtime, right.version, &right.session, right.counts)
            })
}

fn encode(panes: &BTreeMap<String, PaneFacts>) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&Stored {
        schema_version: SCHEMA_VERSION,
        panes: panes.clone(),
    })
    .map_err(|error| format!("could not be encoded: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn report(session: Option<&str>, turn: Option<HookTurn>) -> HookStatusReport {
        HookStatusReport {
            runtime: "claude-code".to_owned(),
            version: hide_agent_hooks::HOOK_VERSION,
            session: session.map(str::to_owned),
            turn,
            counts: None,
            session_start: false,
        }
    }

    fn payload(session: &str) -> SessionSnapshotPayload {
        serde_json::from_value(json!({
            "agents": [{"pane_id": "w1:p1", "agent": "claude", "agent_status": "idle",
                "agent_session": {"kind": "id", "value": session}}],
            "panes": [{"pane_id": "w1:p1"}],
        }))
        .unwrap()
    }

    fn lay(
        facts: &mut HookFacts,
        payload: &mut SessionSnapshotPayload,
    ) -> BTreeMap<String, PaneHookTokens> {
        facts.lay(payload, str::to_owned, |pane| !pane.starts_with("remote:"))
    }

    fn laid_turn(facts: &mut HookFacts, session: &str) -> Option<HookTurn> {
        let mut laid = payload(session);
        lay(facts, &mut laid);
        laid.agents[0].hook
    }

    /// An agent started again in its pane: Herdr can keep naming the session
    /// that ended there, and the session the hook reported since speaks for
    /// the pane until Herdr names yet another one.
    #[test]
    fn the_session_the_hook_reported_since_herdr_last_changed_its_id_speaks_for_the_pane() {
        let mut facts = HookFacts::default();
        facts
            .record("w1:p1", &report(Some("s-new"), Some(HookTurn::Working)))
            .unwrap();
        assert_eq!(laid_turn(&mut facts, "s-old"), Some(HookTurn::Working));
        // Herdr names another session after the report: Herdr is the newer
        // word, and no fact of that session is kept.
        assert_eq!(laid_turn(&mut facts, "s-other"), None);
        // The hook reports again: its session speaks again.
        facts
            .record("w1:p1", &report(Some("s-new"), Some(HookTurn::Done)))
            .unwrap();
        assert_eq!(laid_turn(&mut facts, "s-other"), Some(HookTurn::Done));
        // A session Herdr names that has its own fact keeps speaking, even
        // when another session in the pane reported later (`claude -p`).
        facts
            .record("w1:p1", &report(Some("s-other"), Some(HookTurn::Waiting)))
            .unwrap();
        facts
            .record("w1:p1", &report(Some("s-print"), Some(HookTurn::Working)))
            .unwrap();
        assert_eq!(laid_turn(&mut facts, "s-other"), Some(HookTurn::Waiting));
    }

    #[test]
    fn a_turn_is_laid_for_the_panes_own_session_with_its_version() {
        let mut facts = HookFacts::default();
        facts
            .record("w1:p1", &report(Some("s-1"), Some(HookTurn::Working)))
            .unwrap();
        let mut laid = payload("s-1");
        let tokens = lay(&mut facts, &mut laid);
        assert_eq!(laid.agents[0].hook, Some(HookTurn::Working));
        assert_eq!(
            tokens["w1:p1"].version,
            Some(hide_agent_hooks::HOOK_VERSION)
        );
        assert_eq!(laid_turn(&mut facts, "another"), None);
    }

    #[test]
    fn a_nested_session_never_takes_the_panes_own_sessions_place() {
        let mut facts = HookFacts::default();
        facts
            .record("w1:p1", &report(Some("pane"), Some(HookTurn::Done)))
            .unwrap();
        let mut nested = report(Some("nested"), Some(HookTurn::Working));
        nested.counts = Some(HookCounts {
            working: 0,
            done: 0,
        });
        facts.record("w1:p1", &nested).unwrap();
        assert_eq!(laid_turn(&mut facts, "pane"), Some(HookTurn::Done));
    }

    #[test]
    fn an_idle_prompt_ends_an_interrupted_turn_and_leaves_a_finished_one_finished() {
        let mut facts = HookFacts::default();
        let idle = report(Some("s-1"), Some(HookTurn::Idle));
        facts
            .record("w1:p1", &report(Some("s-1"), Some(HookTurn::Working)))
            .unwrap();
        facts.record("w1:p1", &idle).unwrap();
        assert_eq!(laid_turn(&mut facts, "s-1"), Some(HookTurn::Idle));
        facts
            .record("w1:p1", &report(Some("s-1"), Some(HookTurn::Done)))
            .unwrap();
        assert!(!facts.record("w1:p1", &idle).unwrap().changed);
        assert_eq!(laid_turn(&mut facts, "s-1"), Some(HookTurn::Done));
    }

    #[test]
    fn a_pane_whose_agent_ended_or_that_closed_loses_its_facts() {
        let mut facts = HookFacts::default();
        facts
            .record("w1:p1", &report(Some("s-1"), Some(HookTurn::Done)))
            .unwrap();
        // Before Herdr lists the agent, its pane keeps the report.
        let mut empty: SessionSnapshotPayload =
            serde_json::from_value(json!({"agents": [], "panes": [{"pane_id": "w1:p1"}]})).unwrap();
        lay(&mut facts, &mut empty);
        assert_eq!(facts.panes.len(), 1);
        lay(&mut facts, &mut payload("s-1"));
        lay(&mut facts, &mut empty);
        assert!(facts.panes.is_empty(), "the agent Herdr listed is gone");
        facts.record("w1:p1", &report(Some("s-1"), None)).unwrap();
        let mut closed: SessionSnapshotPayload =
            serde_json::from_value(json!({"agents": [], "panes": []})).unwrap();
        lay(&mut facts, &mut closed);
        assert!(facts.panes.is_empty(), "the pane closed");
    }

    #[test]
    fn a_device_payload_drops_none_of_this_machines_facts() {
        let mut facts = HookFacts::default();
        facts.record("w1:p1", &report(None, None)).unwrap();
        let mut device: SessionSnapshotPayload =
            serde_json::from_value(json!({"agents": [], "panes": []})).unwrap();
        facts.lay(
            &mut device,
            |pane| format!("remote:d:pane:{pane}"),
            |pane| pane.starts_with("remote:d:pane:"),
        );
        assert_eq!(facts.panes.len(), 1);
    }

    #[test]
    fn a_session_start_clears_the_turn_and_repeats_change_nothing() {
        let mut facts = HookFacts::default();
        let working = report(Some("s-1"), Some(HookTurn::Working));
        assert!(facts.record("w1:p1", &working).unwrap().changed);
        assert!(!facts.record("w1:p1", &working).unwrap().changed);
        let mut start = report(Some("s-1"), None);
        start.session_start = true;
        assert!(facts.record("w1:p1", &start).unwrap().changed);
        assert_eq!(laid_turn(&mut facts, "s-1"), None);
    }

    #[test]
    fn panes_past_the_limit_are_refused_and_sessions_past_theirs_drop_the_oldest() {
        let mut facts = HookFacts::default();
        for index in 0..PANE_LIMIT {
            facts
                .record(&format!("w1:p{index}"), &report(None, None))
                .unwrap();
        }
        assert_eq!(
            facts.record("w1:extra", &report(None, None)),
            Err("hook_facts_full")
        );
        let mut facts = HookFacts::default();
        for index in 0..=SESSIONS_PER_PANE {
            facts
                .record("w1:p1", &report(Some(&format!("s-{index}")), None))
                .unwrap();
        }
        let kept = &facts.panes["w1:p1"].sessions;
        assert_eq!(kept.len(), SESSIONS_PER_PANE);
        assert_eq!(kept[0].session.as_deref(), Some("s-1"));
    }

    #[test]
    fn what_a_restart_keeps_reads_back_without_the_turn() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("hook-facts.json");
        {
            let mut facts = HookFacts::restore(path.clone(), Vec::new);
            let mut counted = report(Some("s-1"), Some(HookTurn::Working));
            counted.counts = Some(HookCounts {
                working: 2,
                done: 1,
            });
            facts.record("w1:p1", &counted).unwrap();
        }
        let mut facts = HookFacts::restore(path, Vec::new);
        let mut laid = payload("s-1");
        let tokens = lay(&mut facts, &mut laid);
        assert_eq!(laid.agents[0].hook, None);
        assert_eq!(
            (tokens["w1:p1"].working, tokens["w1:p1"].done),
            (Some(2), Some(1))
        );
    }

    /// The first run with no facts file starts from the records the hooks
    /// kept while they reported to Herdr, so a running agent stays connected;
    /// a later run reads its own file and never those records again.
    #[test]
    fn a_first_run_keeps_the_panes_the_hooks_recorded_and_reads_them_once() {
        let home = tempfile::tempdir().unwrap();
        let panes = hide_agent_hooks::counters::state_directory(home.path());
        std::fs::create_dir_all(&panes).unwrap();
        std::fs::write(
            panes.join("w1_p1.json"),
            r#"{"working":1,"done":2,"version":8,"agent":"claude-code","session":"s-1"}"#,
        )
        .unwrap();
        std::fs::write(panes.join("stray.json"), "{}").unwrap();
        let state = tempfile::tempdir().unwrap();
        let path = state.path().join("hook-facts.json");
        {
            let mut facts = HookFacts::restore(path.clone(), || {
                hide_agent_hooks::counters::every_pane(home.path(), PANE_LIMIT)
            });
            let tokens = lay(&mut facts, &mut payload("s-1"));
            assert_eq!(
                (
                    tokens["w1:p1"].version,
                    tokens["w1:p1"].working,
                    tokens["w1:p1"].done
                ),
                (Some(8), Some(1), Some(2))
            );
        }
        let mut facts = HookFacts::restore(path, || unreachable!("read once"));
        assert!(lay(&mut facts, &mut payload("s-1")).contains_key("w1:p1"));
    }
}
