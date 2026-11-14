//! What each pane's agent hook last reported, held by the core.
//!
//! The hook reports through hided's hook request (`HookCommand::Status`),
//! proven to come from the pane like every other hook request, and the core
//! keeps the report here (PRD engine-bundle-1 D-7). Nothing is written to
//! the engine, so an engine restart loses nothing and needs no restore
//! (D-8).
//!
//! A pane can run more than one session's hooks: the operator's own Stop hook
//! that starts `claude -p` runs Hide's hooks in the pane's environment. So a
//! pane keeps the latest report of each session it heard from, at most
//! [`SESSIONS_PER_PANE`], and a projection lays only the one for the session
//! the engine reports for the pane's agent; an agent whose session the engine names by
//! a path, or not at all, takes the session that reported last.
//!
//! A turn the hook reported stands until the same session's next hook event
//! (`agent_state::axes`): the engine's reading of the screen never overrides it,
//! and no timer ends it. A turn no hook reports the end of is ended by the
//! signal the agent does give (Claude Code's `idle_prompt` notification after
//! an interrupt, Codex's `Interrupt`), and a pane whose agent the engine no longer
//! lists drops its facts.
//!
//! The store has its own small lock, never held across I/O and always taken
//! under the runtime lock (runtime, then facts): to record a report, and in
//! each sync pass, this machine's and a device's, to lay the facts on the
//! session in the step that observes it for delivery. What a restart must
//! keep (runtime, version, session and counts, not the turn) is saved on a
//! thread of its own when it changes ([`LatestFile`]), so no write happens
//! under either lock.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use hide_node_link::panes::{HookCounts, HookStatusReport, HookTurn};
use serde::{Deserialize, Serialize};

use super::PaneHookTokens;
use crate::latest_file::{LatestFile, read_capped};
use crate::sidebar::{SessionAgentPayload, SessionSnapshotPayload};

/// The panes the core keeps facts for. A pane leaves when its pane closes or
/// its agent ends, so this bounds panes whose agents never appeared to the engine.
pub(crate) const PANE_LIMIT: usize = 1024;
/// The sessions one pane keeps, the same number the hook's own file keeps;
/// the one heard from least recently goes first.
pub(crate) use hide_agent_hooks::counters::SESSIONS_PER_PANE;
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
    /// The engine state the agent was in when `turn` was first laid on it:
    /// a turn end the session read finds under a later state ended this
    /// turn, which no hook event of the session may follow (Claude Code's
    /// interruption, which sends neither `Stop` nor always `idle_prompt`).
    #[serde(skip)]
    turn_seq: Option<Laid>,
    /// The calls whose permission the operator has not answered yet, as the
    /// hook last said them: the session waits on the operator while any is
    /// open, whatever `turn` says (a background subagent's dialog outlives
    /// the main turn's `Stop`).
    #[serde(skip)]
    waiting: Vec<String>,
    /// The engine state when `waiting` was last laid on the agent: a
    /// session read under a later state that finds a call's result ends
    /// its approval (a refusal in the dialog sends no hook event).
    #[serde(skip)]
    waiting_seq: Option<Laid>,
    /// Why the provider refused the turn that ended (`StopFailure`), with
    /// when the hook said so; memory only, as the turn is, and gone once
    /// the turn is anything but ended.
    #[serde(skip)]
    refused: Option<TurnRefused>,
    /// The account variables the session's start reported its agent runs
    /// with, empty for the default account
    /// (`hide_agent_hooks::account::reported_variables`); kept, so a restart
    /// reads the pane in the same account. None before a start reported
    /// them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    account: Option<Vec<(String, String)>>,
    /// Where the start that reported `account` falls among this machine's
    /// reported starts, later ones higher, so the last start of a profile
    /// is known whatever its pane is called (D-35, B25); 0 for one saved
    /// before it was kept, older than any.
    #[serde(default, skip_serializing_if = "is_zero")]
    account_start: u64,
}

/// A turn the provider refused (`HookRefusal`) and when its hook said so,
/// which is when the turn ended: an agent that then waits on its screen for
/// a usage limit's reset reads as working to the engine, so no engine state
/// marks that time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TurnRefused {
    pub refusal: hide_node_link::panes::HookRefusal,
    pub at_unix_ms: u64,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

/// The engine state a running turn or an open approval was first laid
/// under. A state sequence compares only with one read on the same engine
/// (status-model.md, Across an engine restart): a device's engine goes with
/// its link, and the one the next link opens counts from the start again,
/// so what was laid or reported before that engine began is older than any
/// state it reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Laid {
    At(u64),
    BeforeThisEngine,
}

impl Laid {
    /// Whether the engine state `now` came after this one.
    fn before(self, now: u64) -> bool {
        match self {
            Self::At(since) => now > since,
            Self::BeforeThisEngine => true,
        }
    }
}

/// The most open approvals one session reports; the hook keeps the same
/// bound (`hide_agent_hooks::counters::WAITING_LIMIT`).
const WAITING_LIMIT: usize = hide_agent_hooks::counters::WAITING_LIMIT;

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
struct PaneFacts {
    /// Oldest first; the last is the session that reported last.
    sessions: Vec<SessionFact>,
    /// A projection laid a session on an agent the engine listed here, so a pane
    /// that lists none afterwards has lost its agent.
    seen_agent: bool,
    /// The session id the engine last named for the pane's agent.
    #[serde(
        rename = "herdr_session",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    engine_session: Option<String>,
    /// A hook reported since the engine last named another session for the pane:
    /// the hook's session, proven to run in the pane, is then the pane's
    /// current one even where the engine still names an earlier one (an agent
    /// started again in the same pane).
    #[serde(skip)]
    reported_since_engine: bool,
}

#[derive(Deserialize, Serialize)]
struct Stored {
    schema_version: u32,
    panes: BTreeMap<String, PaneFacts>,
}

/// The account one pane's agent reported at its session's start.
pub(crate) struct Reported<'a> {
    pub pane: &'a str,
    /// The agent's adapter id.
    pub runtime: &'a str,
    pub session: Option<&'a str>,
    /// Empty for the agent's default account.
    pub variables: &'a [(String, String)],
    /// The start's place among the machine's reported starts, later higher.
    pub start: u64,
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
    /// High water mark, retained when the pane with the latest start closes.
    account_start: u64,
    file: Option<LatestFile<BTreeMap<String, PaneFacts>>>,
}

impl HookFacts {
    /// The facts the last run saved under `path`, and that file for the
    /// saves to come. A missing file is a first run, which starts from the
    /// pane records the hooks keep on this machine (`seed`), so a pane whose
    /// agent last reported to the engine stays connected; any other reason a file
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
        let account_start = panes
            .values()
            .flat_map(|facts| &facts.sessions)
            .map(|fact| fact.account_start)
            .max()
            .unwrap_or(0);
        let facts = Self {
            account_start,
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
        now: u64,
    ) -> Result<Recorded, &'static str> {
        if !self.panes.contains_key(pane) && self.panes.len() >= PANE_LIMIT {
            return Err("hook_facts_full");
        }
        if report.waiting.as_ref().is_some_and(|waiting| {
            waiting.len() > WAITING_LIMIT
                || waiting
                    .iter()
                    .any(|call| call.len() > hide_session::calls::NAME_LIMIT + 17)
        }) {
            return Err("hook_waiting_invalid");
        }
        if report.account.as_ref().is_some_and(|account| {
            account.len() > hide_agent_hooks::account::REPORTED_VARIABLES
                || account.iter().any(|(name, value)| {
                    name.is_empty()
                        || name.len() > 256
                        || value.len() > hide_agent_hooks::account::REPORTED_VALUE_BYTES
                })
        }) {
            return Err("hook_account_invalid");
        }
        let same = |fact: &SessionFact| {
            fact.runtime == report.runtime
                && match (&report.session, &fact.session) {
                    (Some(reported), Some(kept)) => reported == kept,
                    (None, _) | (Some(_), None) => true,
                }
        };
        // A repeated report keeps its place. Only a new start advances the
        // high water mark, independently of the panes still alive.
        let previous = self
            .panes
            .get(pane)
            .and_then(|facts| facts.sessions.iter().rfind(|fact| same(fact)));
        let new_start = report.account.is_some()
            && previous.is_none_or(|fact| {
                fact.account_start == 0
                    || fact.account != report.account
                    || (report.session.is_some() && fact.session != report.session)
            });
        let start = if new_start {
            let next = self
                .account_start
                .checked_add(1)
                .ok_or("hook_account_starts_full")?;
            self.account_start = next;
            Some(next)
        } else {
            None
        };
        let facts = self.panes.entry(pane.to_owned()).or_default();
        let before = facts.clone();
        let mut fact = match facts.sessions.iter().rposition(same) {
            Some(at) => facts.sessions.remove(at),
            None => SessionFact {
                runtime: report.runtime.clone(),
                version: report.version,
                session: None,
                counts: None,
                turn: None,
                turn_seq: None,
                waiting: Vec::new(),
                waiting_seq: None,
                refused: None,
                account: None,
                account_start: 0,
            },
        };
        facts.reported_since_engine = true;
        let turn_before = fact.turn;
        fact.version = report.version;
        if report.session.is_some() {
            fact.session.clone_from(&report.session);
        }
        if report.counts.is_some() {
            fact.counts = report.counts;
        }
        if let Some(start) = start {
            fact.account.clone_from(&report.account);
            fact.account_start = start;
        }
        if report.session_start {
            // A resumed or compacted session has no turn running yet.
            fact.turn = None;
            fact.waiting.clear();
        }
        if let Some(waiting) = &report.waiting
            && *waiting != fact.waiting
        {
            fact.waiting.clone_from(waiting);
            fact.waiting_seq = None;
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
        // A refusal is the ended turn's: a turn that ends again without one
        // (a `Stop`), or any other turn, has none, while the prompt that
        // rests after it (`idle_prompt`) leaves it.
        match (report.turn, &report.refused) {
            (Some(HookTurn::Done), Some(refusal)) => {
                fact.refused = Some(TurnRefused {
                    refusal: refusal.clone(),
                    at_unix_ms: now,
                });
            }
            (Some(HookTurn::Done), None) => fact.refused = None,
            _ if fact.turn != Some(HookTurn::Done) => fact.refused = None,
            _ => {}
        }
        // A report made since the engine that laid the fact ended speaks for
        // the engine reading the pane now: the turn or approvals it restates
        // are laid again under that engine's state, so a session read of an
        // earlier turn can never end them.
        if report.turn.is_some() && fact.turn_seq == Some(Laid::BeforeThisEngine) {
            fact.turn_seq = None;
        }
        if report.waiting.is_some() && fact.waiting_seq == Some(Laid::BeforeThisEngine) {
            fact.waiting_seq = None;
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

    /// Each pane's account as its agent's latest session start reported it
    /// (`SessionFact::account`).
    pub(crate) fn reported_accounts(&self) -> impl Iterator<Item = Reported<'_>> {
        self.panes.iter().filter_map(|(pane, facts)| {
            let fact = facts
                .sessions
                .iter()
                .rev()
                .find(|fact| fact.account.is_some())?;
            Some(Reported {
                pane,
                runtime: &fact.runtime,
                session: fact.session.as_deref(),
                variables: fact.account.as_deref()?,
                start: fact.account_start,
            })
        })
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
            agent.hook_refused = None;
            agent.hook_question = false;
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
            if let Some(named) = engine_session(agent)
                && facts.engine_session.as_deref() != Some(named)
            {
                // The engine naming another session than before is newer than
                // every report so far; its first naming is not.
                if facts.engine_session.is_some() {
                    facts.reported_since_engine = false;
                }
                facts.engine_session = Some(named.to_owned());
                moved = true;
            }
            let Some(fact) = bound(facts, agent) else {
                continue;
            };
            end_by_session_read(fact, agent);
            agent.hook = if fact.waiting.is_empty() {
                fact.turn
            } else {
                Some(HookTurn::Waiting)
            };
            // The dialog still holds the pane, but it asks a question: the
            // session read words it while it holds for the engine's state, and
            // between that state's change and its read the row stays a question
            // rather than turning into an approval (`axes::native_user_turn`).
            agent.hook_question = !fact.waiting.is_empty()
                && fact
                    .waiting
                    .iter()
                    .all(|call| hide_session::calls::asks_question(call));
            agent.hook_refused = fact
                .refused
                .clone()
                .filter(|_| agent.hook == Some(HookTurn::Done));
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

    /// The engine that read the panes `owns` speaks for ended with its
    /// device's link: every turn and approval it holds was laid or reported
    /// before the next engine, whose state sequences count from the start
    /// again, so any state that engine reads comes after them. Memory only,
    /// as the turns are.
    pub(crate) fn engine_ended(&mut self, owns: impl Fn(&str) -> bool) {
        for (_, facts) in self.panes.iter_mut().filter(|(pane, _)| owns(pane)) {
            for fact in &mut facts.sessions {
                if fact.turn.is_some() {
                    fact.turn_seq = Some(Laid::BeforeThisEngine);
                }
                if !fact.waiting.is_empty() {
                    fact.waiting_seq = Some(Laid::BeforeThisEngine);
                }
            }
        }
    }

    fn save(&self) {
        if let Some(file) = &self.file {
            file.save(self.panes.clone());
        }
    }
}

/// The session id the engine names for `agent`, when it names one by id.
fn engine_session(agent: &SessionAgentPayload) -> Option<&str> {
    agent
        .agent_session
        .as_ref()
        .filter(|session| session.kind == "id")
        .map(|session| session.value.as_str())
}

/// Ends the hook's running or waiting turn when the session read, made under
/// a later engine state than the one the turn was first laid under and with
/// the engine reading the agent at rest, finds the session's last turn ended. The
/// read is the later fact, so the turn stands until the session's next hook
/// event or that read, and no timer is involved (D-9).
fn end_by_session_read(fact: &mut SessionFact, agent: &SessionAgentPayload) {
    close_answered_waits(fact, agent);
    if fact.turn.is_some() && fact.turn_seq.is_none() {
        fact.turn_seq = agent.state_change_seq.map(Laid::At);
    }
    let (Some(HookTurn::Working | HookTurn::Waiting), Some(since), Some(now)) =
        (fact.turn, fact.turn_seq, agent.state_change_seq)
    else {
        return;
    };
    let at_rest = matches!(agent.agent_status.as_deref(), Some("idle" | "done"));
    let ended = agent.facts.as_ref().and_then(|facts| facts.turn_ended);
    if since.before(now) && at_rest {
        fact.turn = match ended {
            Some(hide_session::turns::TurnEnded::Completed) => Some(HookTurn::Done),
            // An interrupt closes every dialog of the turn, a subagent's too.
            Some(
                hide_session::turns::TurnEnded::Interrupted
                | hide_session::turns::TurnEnded::Unanswered,
            ) => {
                fact.waiting.clear();
                fact.waiting_seq = None;
                Some(HookTurn::Idle)
            }
            None => return,
        };
    }
}

/// Ends each open approval whose call the session read, made under a later
/// engine state than the one the approval was first laid under, finds a
/// result for: the call ran, or the operator refused it in the dialog,
/// which sends no hook event and lets the turn go on writing (D-9, D-10).
fn close_answered_waits(fact: &mut SessionFact, agent: &SessionAgentPayload) {
    if fact.waiting.is_empty() {
        return;
    }
    if fact.waiting_seq.is_none() {
        fact.waiting_seq = agent.state_change_seq.map(Laid::At);
    }
    let (Some(since), Some(now)) = (fact.waiting_seq, agent.state_change_seq) else {
        return;
    };
    let Some(answered) = agent.facts.as_ref().map(|facts| &facts.answered_calls) else {
        return;
    };
    if since.before(now) && !answered.is_empty() {
        fact.waiting.retain(|open| !answered.contains(open));
    }
}

/// The session fact that speaks for `agent`: of the agent's runtime, and the
/// session the engine reports for it when the engine names one by id. Where none has
/// that id and a hook reported since the engine last changed the id it names, the
/// session that reported last speaks: the hook proves its process runs in
/// the pane, and the engine can keep naming the session of an agent that ended
/// there.
fn bound<'a>(facts: &'a mut PaneFacts, agent: &SessionAgentPayload) -> Option<&'a mut SessionFact> {
    let runtime = hide_agent_adapter::adapter(agent.agent.as_deref()?)?.id;
    let reported = engine_session(agent);
    let named = facts.sessions.iter().rposition(|fact| {
        fact.runtime == runtime
            && match (reported, fact.session.as_deref()) {
                (Some(reported), Some(kept)) => reported == kept,
                _ => true,
            }
    });
    let at = named.or_else(|| {
        facts
            .reported_since_engine
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
            refused: None,
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
            waiting: Vec::new(),
            waiting_seq: None,
            account: None,
            account_start: 0,
        })
        .collect();
    PaneFacts {
        sessions,
        ..PaneFacts::default()
    }
}

fn stored_part_moved(before: &PaneFacts, after: &PaneFacts) -> bool {
    before.seen_agent != after.seen_agent
        || before.engine_session != after.engine_session
        || before.sessions.len() != after.sessions.len()
        || before
            .sessions
            .iter()
            .zip(&after.sessions)
            .any(|(left, right)| {
                (
                    &left.runtime,
                    left.version,
                    &left.session,
                    left.counts,
                    &left.account,
                    left.account_start,
                ) != (
                    &right.runtime,
                    right.version,
                    &right.session,
                    right.counts,
                    &right.account,
                    right.account_start,
                )
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
            waiting: None,
            account: None,
            refused: None,
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

    /// The pane's agent as the engine reads it, `status` at state `seq`, with
    /// what the session read made under that state says of its last turn
    /// and of its tool calls.
    fn read(
        status: &str,
        seq: u64,
        ended: Option<hide_session::turns::TurnEnded>,
        answered: &[&str],
    ) -> SessionSnapshotPayload {
        let mut read = payload("s-1");
        read.agents[0].agent_status = Some(status.to_owned());
        read.agents[0].state_change_seq = Some(seq);
        read.agents[0].facts = Some(crate::request_view::RowFacts {
            turn_ended: ended,
            answered_calls: answered.iter().map(|call| (*call).to_owned()).collect(),
            ..Default::default()
        });
        read
    }

    /// The turn laid on the agent and the approvals still open after `read`.
    fn after(
        facts: &mut HookFacts,
        mut read: SessionSnapshotPayload,
    ) -> (Option<HookTurn>, Vec<String>) {
        lay(facts, &mut read);
        let open = facts.panes["w1:p1"]
            .sessions
            .last()
            .unwrap()
            .waiting
            .clone();
        (read.agents[0].hook, open)
    }

    fn asking(call: &str) -> HookStatusReport {
        let mut asking = report(Some("s-1"), Some(HookTurn::Waiting));
        asking.waiting = Some(vec![call.to_owned()]);
        asking
    }

    /// A turn the provider refused keeps why and when on the row while it
    /// stands ended, even with the engine reading the agent at work (Claude
    /// Code waits on its screen for a usage limit's reset); the rest after
    /// it keeps it, and the next turn, or an end without a refusal, clears it.
    #[test]
    fn a_refused_turn_keeps_why_and_when_until_the_next_turn() {
        use hide_node_link::panes::{HookRefusal, RefusalKind};
        let refusal = HookRefusal {
            kind: RefusalKind::Limit,
            line: "You've hit your limit · resets 4pm".to_owned(),
        };
        let refused_at = |facts: &mut HookFacts| {
            let mut laid = read("working", 9, None, &[]);
            lay(facts, &mut laid);
            let agent = &laid.agents[0];
            (
                agent.hook,
                agent
                    .hook_refused
                    .as_ref()
                    .map(|refused| refused.at_unix_ms),
            )
        };
        let mut facts = HookFacts::default();
        facts
            .record("w1:p1", &report(Some("s-1"), Some(HookTurn::Working)), 10)
            .unwrap();
        let mut ended = report(Some("s-1"), Some(HookTurn::Done));
        ended.refused = Some(refusal.clone());
        facts.record("w1:p1", &ended, 20).unwrap();
        assert_eq!(refused_at(&mut facts), (Some(HookTurn::Done), Some(20)));
        facts
            .record("w1:p1", &report(Some("s-1"), Some(HookTurn::Idle)), 80)
            .unwrap();
        facts
            .record("w1:p1", &report(Some("s-1"), None), 90)
            .unwrap();
        assert_eq!(refused_at(&mut facts), (Some(HookTurn::Done), Some(20)));
        facts
            .record("w1:p1", &report(Some("s-1"), Some(HookTurn::Working)), 100)
            .unwrap();
        assert_eq!(refused_at(&mut facts), (Some(HookTurn::Working), None));
        facts.record("w1:p1", &ended, 110).unwrap();
        facts
            .record("w1:p1", &report(Some("s-1"), Some(HookTurn::Done)), 120)
            .unwrap();
        assert_eq!(refused_at(&mut facts), (Some(HookTurn::Done), None));
        facts.record("w1:p1", &ended, 130).unwrap();
        facts.record("w1:p1", &asking("call-1"), 140).unwrap();
        assert_eq!(refused_at(&mut facts), (Some(HookTurn::Waiting), None));
    }

    /// A device's engine goes with its link and the next link's engine counts
    /// its states from the start again: a turn the hook last said was
    /// running when the link went down ends once that next engine reads the
    /// agent at rest and the session read under that state says the turn
    /// ended. A lower sequence alone, read as if it were the same engine's,
    /// never ended it, and the restore's unknown step does not either.
    /// A session's start keeps the account it reported through a restart;
    /// facts an earlier build saved, with no account, read as before.
    #[test]
    fn a_reported_account_is_kept_and_facts_saved_before_it_read_as_before() {
        let mut facts = HookFacts::default();
        let mut started = report(Some("s-1"), None);
        started.session_start = true;
        started.account = Some(vec![("CLAUDE_CONFIG_DIR".into(), "/accounts/a".into())]);
        assert!(facts.record("w1:p1", &started, 0).unwrap().changed);
        let saved = encode(&facts.panes).unwrap();
        let stored: Stored = serde_json::from_slice(&saved).unwrap();
        let restored = HookFacts {
            panes: stored.panes,
            account_start: 1,
            file: None,
        };
        let reported: Vec<_> = restored
            .reported_accounts()
            .map(|r| (r.pane, r.session, r.variables.to_vec()))
            .collect();
        assert_eq!(
            reported,
            [(
                "w1:p1",
                Some("s-1"),
                vec![("CLAUDE_CONFIG_DIR".to_owned(), "/accounts/a".to_owned())]
            )]
        );
        // A later event of the session says nothing of it and keeps it.
        let mut facts = restored;
        facts
            .record("w1:p1", &report(Some("s-1"), Some(HookTurn::Working)), 0)
            .unwrap();
        assert_eq!(facts.reported_accounts().count(), 1);
        let older: Stored = serde_json::from_value(serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "panes": {"w1:p2": {"sessions": [{"runtime": "claude-code", "version": 1, "session": "s-2", "counts": null}], "seen_agent": true}},
        }))
        .unwrap();
        let older = HookFacts {
            panes: older.panes,
            account_start: 0,
            file: None,
        };
        assert_eq!(older.reported_accounts().count(), 0);
    }

    #[test]
    fn a_turn_from_an_engine_that_ended_ends_on_the_next_engines_read_of_it_at_rest() {
        use hide_session::turns::TurnEnded::Completed;
        let mut facts = HookFacts::default();
        facts
            .record("w1:p1", &report(Some("s-1"), Some(HookTurn::Working)), 0)
            .unwrap();
        let working = Some(HookTurn::Working);
        assert_eq!(after(&mut facts, read("working", 57, None, &[])).0, working);
        assert_eq!(
            after(&mut facts, read("done", 2, Some(Completed), &[])).0,
            working,
            "a state of the same engine before the turn's ends nothing"
        );

        facts.engine_ended(|_| true);
        assert_eq!(after(&mut facts, read("unknown", 0, None, &[])).0, working);
        assert_eq!(
            after(&mut facts, read("unknown", 0, Some(Completed), &[])).0,
            working,
            "a restored pane not yet read again is not at rest"
        );
        assert_eq!(
            after(&mut facts, read("idle", 2, None, &[])).0,
            working,
            "at rest, with no session read saying the turn ended"
        );
        let done = Some(HookTurn::Done);
        assert_eq!(
            after(&mut facts, read("done", 2, Some(Completed), &[])).0,
            done
        );
        assert_eq!(
            after(&mut facts, read("done", 2, Some(Completed), &[])).0,
            done
        );
    }

    /// What a hook reports once the engine that laid its turn ended speaks
    /// for the next engine: a turn or approvals it says again, unchanged,
    /// are laid again under that engine's state, so a session read of the
    /// turn before them, under that same state, ends neither.
    #[test]
    fn a_turn_reported_again_after_its_engine_ended_is_laid_under_the_next_one() {
        use hide_session::turns::TurnEnded::Completed;
        let mut facts = HookFacts::default();
        facts.record("w1:p1", &asking("call-1"), 0).unwrap();
        after(&mut facts, read("blocked", 57, None, &[]));
        facts.engine_ended(|_| true);
        facts.record("w1:p1", &asking("call-1"), 0).unwrap();
        assert_eq!(
            after(&mut facts, read("idle", 2, Some(Completed), &["call-1"])),
            (Some(HookTurn::Waiting), vec!["call-1".to_owned()])
        );
        assert_eq!(
            after(&mut facts, read("idle", 3, Some(Completed), &["call-1"])),
            (Some(HookTurn::Done), Vec::new()),
            "a read under a later state of the next engine ends them"
        );
    }

    /// An approval open when its engine ended closes on the next engine's
    /// session read that finds its call's result, working or not (D-10); a
    /// read that finds no result, or the restore's unknown step, leaves it.
    #[test]
    fn an_approval_from_an_engine_that_ended_closes_on_the_next_engines_read_of_its_answer() {
        let mut facts = HookFacts::default();
        facts.record("w1:p1", &asking("call-1"), 0).unwrap();
        after(&mut facts, read("blocked", 57, None, &[]));
        facts.engine_ended(|_| true);
        let open = vec!["call-1".to_owned()];
        assert_eq!(after(&mut facts, read("unknown", 0, None, &[])).1, open);
        assert_eq!(after(&mut facts, read("working", 1, None, &[])).1, open);
        assert_eq!(
            after(&mut facts, read("working", 1, None, &["call-1"])),
            (Some(HookTurn::Waiting), Vec::new())
        );
    }

    /// The end of one device's engine leaves every pane it does not speak
    /// for as it was.
    #[test]
    fn an_engine_that_ended_leaves_the_turns_of_other_panes_as_they_were() {
        let mut facts = HookFacts::default();
        for pane in ["w1:p1", "remote:mini:pane:t1"] {
            facts
                .record(pane, &report(Some("s-1"), Some(HookTurn::Working)), 0)
                .unwrap();
        }
        let mut both = read("working", 57, None, &[]);
        let mut device = both.agents[0].clone();
        device.pane_id = Some("remote:mini:pane:t1".to_owned());
        both.agents.push(device);
        both.panes
            .push(serde_json::from_value(json!({"pane_id": "remote:mini:pane:t1"})).unwrap());
        facts.lay(&mut both, str::to_owned, |_| true);
        facts.engine_ended(|pane| crate::address::is_pane_of(pane, "mini"));
        let anchor = |facts: &HookFacts, pane: &str| facts.panes[pane].sessions[0].turn_seq;
        assert_eq!(anchor(&facts, "w1:p1"), Some(Laid::At(57)));
        assert_eq!(
            anchor(&facts, "remote:mini:pane:t1"),
            Some(Laid::BeforeThisEngine)
        );
    }

    /// An agent started again in its pane: the engine can keep naming the session
    /// that ended there, and the session the hook reported since speaks for
    /// the pane until the engine names yet another one.
    #[test]
    fn the_session_the_hook_reported_since_the_engine_last_changed_its_id_speaks_for_the_pane() {
        let mut facts = HookFacts::default();
        facts
            .record("w1:p1", &report(Some("s-new"), Some(HookTurn::Working)), 0)
            .unwrap();
        assert_eq!(laid_turn(&mut facts, "s-old"), Some(HookTurn::Working));
        // The engine names another session after the report: the engine is the newer
        // word, and no fact of that session is kept.
        assert_eq!(laid_turn(&mut facts, "s-other"), None);
        // The hook reports again: its session speaks again.
        facts
            .record("w1:p1", &report(Some("s-new"), Some(HookTurn::Done)), 0)
            .unwrap();
        assert_eq!(laid_turn(&mut facts, "s-other"), Some(HookTurn::Done));
        // A session the engine names that has its own fact keeps speaking, even
        // when another session in the pane reported later (`claude -p`).
        facts
            .record(
                "w1:p1",
                &report(Some("s-other"), Some(HookTurn::Waiting)),
                0,
            )
            .unwrap();
        facts
            .record(
                "w1:p1",
                &report(Some("s-print"), Some(HookTurn::Working)),
                0,
            )
            .unwrap();
        assert_eq!(laid_turn(&mut facts, "s-other"), Some(HookTurn::Waiting));
    }

    #[test]
    fn a_turn_is_laid_for_the_panes_own_session_with_its_version() {
        let mut facts = HookFacts::default();
        facts
            .record("w1:p1", &report(Some("s-1"), Some(HookTurn::Working)), 0)
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
            .record("w1:p1", &report(Some("pane"), Some(HookTurn::Done)), 0)
            .unwrap();
        let mut nested = report(Some("nested"), Some(HookTurn::Working));
        nested.counts = Some(HookCounts {
            working: 0,
            done: 0,
        });
        facts.record("w1:p1", &nested, 0).unwrap();
        assert_eq!(laid_turn(&mut facts, "pane"), Some(HookTurn::Done));
    }

    #[test]
    fn an_idle_prompt_ends_an_interrupted_turn_and_leaves_a_finished_one_finished() {
        let mut facts = HookFacts::default();
        let idle = report(Some("s-1"), Some(HookTurn::Idle));
        facts
            .record("w1:p1", &report(Some("s-1"), Some(HookTurn::Working)), 0)
            .unwrap();
        facts.record("w1:p1", &idle, 0).unwrap();
        assert_eq!(laid_turn(&mut facts, "s-1"), Some(HookTurn::Idle));
        facts
            .record("w1:p1", &report(Some("s-1"), Some(HookTurn::Done)), 0)
            .unwrap();
        assert!(!facts.record("w1:p1", &idle, 0).unwrap().changed);
        assert_eq!(laid_turn(&mut facts, "s-1"), Some(HookTurn::Done));
    }

    #[test]
    fn a_pane_whose_agent_ended_or_that_closed_loses_its_facts() {
        let mut facts = HookFacts::default();
        facts
            .record("w1:p1", &report(Some("s-1"), Some(HookTurn::Done)), 0)
            .unwrap();
        // Before the engine lists the agent, its pane keeps the report.
        let mut empty: SessionSnapshotPayload =
            serde_json::from_value(json!({"agents": [], "panes": [{"pane_id": "w1:p1"}]})).unwrap();
        lay(&mut facts, &mut empty);
        assert_eq!(facts.panes.len(), 1);
        lay(&mut facts, &mut payload("s-1"));
        lay(&mut facts, &mut empty);
        assert!(
            facts.panes.is_empty(),
            "the agent the engine listed is gone"
        );
        facts
            .record("w1:p1", &report(Some("s-1"), None), 0)
            .unwrap();
        let mut closed: SessionSnapshotPayload =
            serde_json::from_value(json!({"agents": [], "panes": []})).unwrap();
        lay(&mut facts, &mut closed);
        assert!(facts.panes.is_empty(), "the pane closed");
    }

    #[test]
    fn a_device_payload_drops_none_of_this_machines_facts() {
        let mut facts = HookFacts::default();
        facts.record("w1:p1", &report(None, None), 0).unwrap();
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
        assert!(facts.record("w1:p1", &working, 0).unwrap().changed);
        assert!(!facts.record("w1:p1", &working, 0).unwrap().changed);
        let mut start = report(Some("s-1"), None);
        start.session_start = true;
        assert!(facts.record("w1:p1", &start, 0).unwrap().changed);
        assert_eq!(laid_turn(&mut facts, "s-1"), None);
    }

    #[test]
    fn panes_past_the_limit_are_refused_and_sessions_past_theirs_drop_the_oldest() {
        let mut facts = HookFacts::default();
        for index in 0..PANE_LIMIT {
            facts
                .record(&format!("w1:p{index}"), &report(None, None), 0)
                .unwrap();
        }
        assert_eq!(
            facts.record("w1:extra", &report(None, None), 0),
            Err("hook_facts_full")
        );
        let mut facts = HookFacts::default();
        for index in 0..=SESSIONS_PER_PANE {
            facts
                .record("w1:p1", &report(Some(&format!("s-{index}")), None), 0)
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
            facts.record("w1:p1", &counted, 0).unwrap();
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
    /// kept while they reported to the engine, so a running agent stays connected;
    /// a later run reads its own file and never those records again.
    #[test]
    fn a_first_run_keeps_the_panes_the_hooks_recorded_and_reads_them_once() {
        let home = tempfile::tempdir().unwrap();
        let panes = hide_agent_hooks::counters::state_directory(home.path()).unwrap();
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

    /// The facts file the 1.2.1 daemon wrote for a Claude pane (an isolated
    /// run of that release) reads, and is written back, as the same document:
    /// the engine's session for the pane keeps its key, `herdr_session`.
    #[test]
    fn a_facts_file_release_1_2_1_wrote_is_written_back_as_the_same_document() {
        let written = include_str!("../../tests/fixtures/release-1.2.1/hook-facts.json");
        let stored: Stored = serde_json::from_str(written).unwrap();
        assert_eq!(
            stored.panes["h1:p4"].engine_session.as_deref(),
            Some("c3-session-36709")
        );
        assert_eq!(
            serde_json::to_value(&stored).unwrap(),
            serde_json::from_str::<serde_json::Value>(written).unwrap()
        );
    }
}
