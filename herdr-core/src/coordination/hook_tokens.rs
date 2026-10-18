//! Puts a pane's hook tokens back on a Herdr that lost them.
//!
//! The hook helper reports a pane's `hide_hooks`, `hide_sub_working` and
//! `hide_sub_done` tokens after each of its events and keeps what it
//! reported in the pane's file under `~/.hide/agent-hooks/panes/`
//! (`hide_agent_hooks::counters`). Herdr keeps a token only as long as the
//! server that took it: a live handoff or a restart starts the new server
//! with none, while the pane ids stay (measured on the pinned Herdr, and
//! the reason a file can be paired with a pane at all). Until the agent's
//! next event the pane then reads as a session that predates Hide's setup,
//! with a Reopen that restarts a healthy session (issue 799).
//!
//! The rule is the lineage writer's: the agent list the coordinator already
//! reads each second is the observation, and a pane that lacks its token
//! while its file has one is written, not a pane that changed. It differs in
//! what makes a pane worth asking about again. A lineage patch is a pure
//! function of the ledger, so it can be compared with the list on every
//! read. A file is read from disk, so each pane is asked once for each
//! connection to Herdr (every connect is a bootstrap, and a handoff or a
//! restart ends the connection); a pane with no file, or one an older helper
//! wrote, is not asked again, because the helper's own next event reports
//! the token and writes the file in the same step.
//!
//! A pane id alone does not say that the agent in the pane now is the one the
//! file describes, so the file also names the agent that reported and the
//! session it reported for, and Herdr names what it detects in the pane now.
//! The version token is put back when the agent is the same one: a hook is
//! installed in a runtime's configuration, not in a session. The counts are
//! put back only when the session is the same too, and are left out, not
//! written as zero, when Herdr names no session or another one, so the pane
//! reads as an instrumented agent whose children are not known until its
//! next event. Nothing here relies on `SessionStart` having reset the file:
//! a session that started after the file was written and before this
//! connection is exactly the case the session id tells apart.
//!
//! The file is read and the report is sent on a worker thread: the
//! coordinator never waits on either (`docs/ARCHITECTURE.md`, the
//! session-sync thread never blocks). One batch is in flight and one waits,
//! at most [`BATCH_LIMIT`] panes each; panes beyond that stay unasked and are
//! taken by the next observation. A failure is retried at most [`ATTEMPTS`]
//! times for a connection and written to the diagnostic log with the pane.

use crate::agent_hooks::PaneHookTokens;
use crate::session_sync::ProjectedAgent;
use hide_agent_hooks::counters::{self, PaneCounters, Restorable, Restore};
use hide_agent_hooks::report::Counts;
use hide_herdr_client::{ApiConnector, request_with_connector};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Panes in one batch. A device with more agents than this is taken in
/// several observations, a second apart.
const BATCH_LIMIT: usize = 256;
/// How many times a pane whose restore failed is asked in one connection.
const ATTEMPTS: u8 = 3;
/// One report holds the worker this long at most.
const REPORT_TIMEOUT: Duration = Duration::from_secs(1);
/// How many times a report is sent again because the pane's file changed
/// while it was on its way.
const RESEND_LIMIT: usize = 3;
/// Completions that can be waiting for the coordinator: one batch in
/// flight, one queued.
const COMPLETION_CAPACITY: usize = BATCH_LIMIT * 2;

/// One pane to ask about, with what Herdr says about it now.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Ask {
    pane: String,
    /// The adapter id of the agent Herdr detects in the pane.
    agent: String,
    /// The session Herdr reports for that agent, when it reports one.
    session: Option<String>,
}

#[derive(Debug)]
struct Batch {
    generation: u64,
    asks: Vec<Ask>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Outcome {
    /// The version token is back on the pane, and its counts too when
    /// `counts` says the file's session is the one running there.
    Restored {
        counts: bool,
    },
    /// The helper never counted this pane here.
    NoRecord,
    /// An older helper wrote the file, so which agent reported is not known.
    Unpairable,
    /// The file is another agent's: the pane was taken over.
    OtherAgent,
    Failed(String),
}

#[derive(Debug)]
struct Completion {
    pane: String,
    generation: u64,
    outcome: Outcome,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Standing {
    /// Asked of the worker in this batch, on this attempt.
    InFlight { generation: u64, attempts: u8 },
    /// Answered for this connection: put back, or nothing to put back.
    Settled,
    /// The worker could not put it back this many times.
    Failed { attempts: u8 },
}

pub(crate) struct Restorer {
    sender: Option<mpsc::SyncSender<Batch>>,
    worker: Option<JoinHandle<()>>,
    completed: mpsc::Receiver<Completion>,
    stopping: Arc<AtomicBool>,
    generation: u64,
    /// Where each pane stands in this connection. Panes Herdr no longer lists
    /// are dropped on every observation, so it holds at most the agent list.
    standing: HashMap<String, Standing>,
    /// Whether this connection has already said it had more panes than a
    /// batch holds.
    deferral_reported: bool,
}

impl Restorer {
    /// `home` is the account whose hook files these are: this machine's, so
    /// the coordinator of a device (whose files are the device's) makes none.
    pub(crate) fn new(home: PathBuf, connector: Arc<dyn ApiConnector>) -> Result<Self, String> {
        let (sender, receiver) = mpsc::sync_channel::<Batch>(1);
        let (done, completed) = mpsc::sync_channel(COMPLETION_CAPACITY);
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&stopping);
        let worker = thread::Builder::new()
            .name("hide-hook-tokens".into())
            .spawn(move || {
                while let Ok(batch) = receiver.recv() {
                    let mut tally = Tally::default();
                    for ask in batch.asks {
                        if stop.load(Ordering::Acquire) {
                            return;
                        }
                        let outcome = restore(&home, connector.as_ref(), &ask);
                        tally.add(&ask.pane, &outcome);
                        // The channel holds every outstanding completion
                        // (`COMPLETION_CAPACITY`), so this cannot fail while
                        // the coordinator lives.
                        let _ = done.try_send(Completion {
                            pane: ask.pane,
                            generation: batch.generation,
                            outcome,
                        });
                    }
                    tally.report(batch.generation);
                }
            })
            .map_err(|error| format!("the hook token worker did not start: {error}"))?;
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
            completed,
            stopping,
            generation: 0,
            standing: HashMap::new(),
            deferral_reported: false,
        })
    }

    /// Looks at one read of the agent list. `reconnected` is true for the
    /// list read right after a connect: Herdr may be another server than the
    /// one the panes were last asked of, so every pane is asked again.
    pub(crate) fn observe(&mut self, agents: &[ProjectedAgent], reconnected: bool) {
        if reconnected {
            self.standing.clear();
            self.deferral_reported = false;
        }
        while let Ok(done) = self.completed.try_recv() {
            self.settle(done);
        }
        let listed: HashSet<&str> = agents.iter().map(|agent| agent.pane_id.as_str()).collect();
        self.standing
            .retain(|pane, _| listed.contains(pane.as_str()));
        let mut wanted = Vec::new();
        let mut attempts = Vec::new();
        for (agent, kind) in agents
            .iter()
            .filter_map(|agent| Some((agent, hook_kind(agent)?)))
        {
            match self.standing.get(&agent.pane_id) {
                None => attempts.push(0),
                Some(Standing::Failed { attempts: made }) if *made < ATTEMPTS => {
                    attempts.push(*made)
                }
                Some(_) => continue,
            }
            wanted.push(Ask {
                pane: agent.pane_id.clone(),
                agent: kind.to_owned(),
                session: agent
                    .agent_session
                    .as_ref()
                    .map(|session| session.value.clone()),
            });
        }
        if wanted.is_empty() {
            return;
        }
        if wanted.len() > BATCH_LIMIT && !self.deferral_reported {
            self.deferral_reported = true;
            crate::diagnostic!(json!({
                "component": "hook_tokens",
                "kind": "restore.deferred",
                "panes": wanted.len(),
                "batch_limit": BATCH_LIMIT,
            }));
        }
        wanted.truncate(BATCH_LIMIT);
        let Some(generation) = self.generation.checked_add(1) else {
            return;
        };
        let Some(sender) = self.sender.as_ref() else {
            return;
        };
        let batch = Batch {
            generation,
            asks: wanted.clone(),
        };
        match sender.try_send(batch) {
            Ok(()) => {
                self.generation = generation;
                for (ask, made) in wanted.into_iter().zip(attempts) {
                    self.standing.insert(
                        ask.pane,
                        Standing::InFlight {
                            generation,
                            attempts: made,
                        },
                    );
                }
            }
            // The worker is still on the last batch: these panes are asked
            // for again by the next observation.
            Err(mpsc::TrySendError::Full(_)) => {}
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.sender = None;
                crate::diagnostic!(json!({
                    "component": "hook_tokens",
                    "kind": "restore.worker_ended",
                }));
            }
        }
    }

    fn settle(&mut self, done: Completion) {
        let Some(Standing::InFlight {
            generation,
            attempts,
        }) = self.standing.get(&done.pane).copied()
        else {
            return;
        };
        // A batch asked before the last connect describes another server.
        if generation != done.generation {
            return;
        }
        let standing = match done.outcome {
            Outcome::Restored { .. }
            | Outcome::NoRecord
            | Outcome::Unpairable
            | Outcome::OtherAgent => Standing::Settled,
            Outcome::Failed(_) => {
                let attempts = attempts.saturating_add(1);
                if attempts >= ATTEMPTS {
                    crate::diagnostic!(json!({
                        "component": "hook_tokens",
                        "kind": "restore.gave_up",
                        "pane_id": done.pane,
                        "attempts": attempts,
                    }));
                }
                Standing::Failed { attempts }
            }
        };
        self.standing.insert(done.pane, standing);
    }
}

impl Drop for Restorer {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// The adapter id of an agent Herdr lists that carries no hook version: the
/// state a handoff leaves every instrumented pane in, and the state of a
/// session the hook never reached. An agent Hide has no adapter for has no
/// hook to restore.
fn hook_kind(agent: &ProjectedAgent) -> Option<&'static str> {
    if PaneHookTokens::read(&agent.tokens).version.is_some() {
        return None;
    }
    hide_agent_adapter::adapter(agent.agent.as_deref()?).map(|adapter| adapter.id)
}

/// What one report puts on a pane: the version, and the counts when the file
/// is the running session's.
type Report = (u32, Option<PaneCounters>);

/// Puts one pane's tokens back from its file.
fn restore(home: &std::path::Path, connector: &dyn ApiConnector, ask: &Ask) -> Outcome {
    let mut sent: Option<Report> = None;
    for _ in 0..=RESEND_LIMIT {
        let found = match counters::restore_of(home, &ask.pane) {
            Ok(Restore::NoRecord) => return finish(connector, ask, sent, Outcome::NoRecord),
            Ok(Restore::Unpairable) => return finish(connector, ask, sent, Outcome::Unpairable),
            Ok(Restore::Report(found)) => found,
            Err(error) => return Outcome::Failed(format!("the pane's file: {error}")),
        };
        if found.agent != ask.agent {
            return finish(connector, ask, sent, Outcome::OtherAgent);
        }
        let report = (found.version, running_counts(&found, ask));
        // A file another event changed while the last report was on its way
        // is sent again, the way the helper does, so Herdr ends on the latest.
        if sent == Some(report) {
            return Outcome::Restored {
                counts: report.1.is_some(),
            };
        }
        // Counts this file cannot vouch for are left out, unless an earlier
        // report of this restore already put some on the pane: a report
        // merges token by token, so those are cleared and not left behind.
        let counts = match (report.1, sent.is_some_and(|earlier| earlier.1.is_some())) {
            (Some(counters), _) => Counts::Set(counters),
            (None, true) => Counts::Clear,
            (None, false) => Counts::Leave,
        };
        // The same report the helper sends, so the tokens are the helper's.
        if let Err(error) = request_with_connector(
            connector,
            "pane.report_metadata",
            hide_agent_hooks::report::report_params(&ask.pane, report.0, counts),
            REPORT_TIMEOUT,
        ) {
            return Outcome::Failed(format!("pane.report_metadata: {error}"));
        }
        sent = Some(report);
    }
    Outcome::Restored {
        counts: sent.is_some_and(|report| report.1.is_some()),
    }
}

/// The file's counts, when the session that wrote them is the one Herdr says
/// is running in the pane. A session either side does not name is not the
/// same session.
fn running_counts(found: &Restorable, ask: &Ask) -> Option<PaneCounters> {
    match (&found.session, &ask.session) {
        (Some(recorded), Some(running)) if recorded == running => Some(found.counters),
        _ => None,
    }
}

/// Ends a restore whose file stopped being the pane's: the file went (swept
/// with its pane) or another agent's replaced it. What an earlier report of
/// this restore put on the pane is cleared when it was counts, since they
/// are no longer vouched for; the version it put stays, as the helper's own
/// next report will say.
fn finish(
    connector: &dyn ApiConnector,
    ask: &Ask,
    sent: Option<Report>,
    unreadable: Outcome,
) -> Outcome {
    let Some((version, counts)) = sent else {
        return unreadable;
    };
    if counts.is_some()
        && let Err(error) = request_with_connector(
            connector,
            "pane.report_metadata",
            hide_agent_hooks::report::report_params(&ask.pane, version, Counts::Clear),
            REPORT_TIMEOUT,
        )
    {
        return Outcome::Failed(format!("pane.report_metadata: {error}"));
    }
    Outcome::Restored { counts: false }
}

#[derive(Default)]
struct Tally {
    restored: usize,
    counts_withheld: usize,
    no_record: usize,
    unpairable: usize,
    other_agent: usize,
    failed: usize,
}

impl Tally {
    fn add(&mut self, pane: &str, outcome: &Outcome) {
        match outcome {
            Outcome::Restored { counts } => {
                self.restored += 1;
                if !counts {
                    self.counts_withheld += 1;
                }
            }
            Outcome::NoRecord => self.no_record += 1,
            Outcome::Unpairable => self.unpairable += 1,
            Outcome::OtherAgent => self.other_agent += 1,
            Outcome::Failed(message) => {
                self.failed += 1;
                crate::diagnostic!(json!({
                    "component": "hook_tokens",
                    "kind": "restore.failed",
                    "pane_id": pane,
                    "message": message,
                }));
            }
        }
    }

    fn report(&self, generation: u64) {
        crate::diagnostic!(json!({
            "component": "hook_tokens",
            "kind": "restore.batch",
            "generation": generation,
            "restored": self.restored,
            "counts_withheld": self.counts_withheld,
            "no_record": self.no_record,
            "unpairable": self.unpairable,
            "other_agent": self.other_agent,
            "failed": self.failed,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_herdr::FakeHerdr;
    use crate::sidebar::SessionAgentSessionPayload;
    use hide_agent_hooks::HOOK_VERSION;
    use hide_agent_hooks::counters::{Change, Reporter, change};
    use serde_json::{Value, json};
    use std::collections::BTreeMap;
    use std::time::Instant;

    /// The session and agent the hook's files in these tests were written by.
    const WHO: Reporter<'static> = Reporter {
        agent: Some("claude-code"),
        session: Some("session-a"),
    };

    /// Herdr's view of `pane`: Claude Code, in `session`, with `tokens`.
    fn agent(pane: &str, session: Option<&str>, tokens: &[(&str, &str)]) -> ProjectedAgent {
        ProjectedAgent {
            pane_id: pane.into(),
            terminal_id: format!("term-{pane}"),
            name: None,
            workspace_id: "w1".into(),
            tab_id: "w1:t1".into(),
            cwd: None,
            agent: Some("claude".into()),
            agent_status: Some("idle".into()),
            agent_session: session.map(|value| SessionAgentSessionPayload {
                kind: "id".into(),
                value: value.into(),
            }),
            spawned_from_pane_id: None,
            spawned_from_machine_id: None,
            declared_parent_session: None,
            lineage_session: None,
            state_change_seq: 1,
            completion_seq: None,
            launch_pending: false,
            tokens: tokens
                .iter()
                .map(|(key, value)| ((*key).to_owned(), json!(value)))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    fn herdr() -> FakeHerdr {
        FakeHerdr::start("hook-tokens", |method, _| match method {
            "pane.report_metadata" => json!({"type": "ok"}),
            other => panic!("unexpected {other}"),
        })
    }

    fn restorer(home: &tempfile::TempDir, herdr: &FakeHerdr) -> Restorer {
        Restorer::new(home.path().to_path_buf(), Arc::new(herdr.connector())).unwrap()
    }

    /// Two subagents started and one stopped in `pane` by `who`: one working,
    /// one done.
    fn count(home: &tempfile::TempDir, pane: &str, who: Reporter<'_>) {
        for step in [
            Change::Started(None),
            Change::Started(None),
            Change::Stopped(None),
        ] {
            change(home.path(), pane, step, who).unwrap();
        }
    }

    fn reports(herdr: &FakeHerdr) -> Vec<Value> {
        herdr
            .calls()
            .into_iter()
            .inspect(|(method, _)| assert_eq!(method, "pane.report_metadata"))
            .map(|(_, params)| params)
            .collect()
    }

    /// What the helper would report for `pane` after `count`.
    fn full(pane: &str) -> Value {
        hide_agent_hooks::report::report_params(
            pane,
            HOOK_VERSION,
            Counts::Set(PaneCounters {
                working: 1,
                done: 1,
            }),
        )
    }

    fn version_only(pane: &str) -> Value {
        hide_agent_hooks::report::report_params(pane, HOOK_VERSION, Counts::Leave)
    }

    fn cleared(pane: &str) -> Value {
        hide_agent_hooks::report::report_params(pane, HOOK_VERSION, Counts::Clear)
    }

    /// The tokens Herdr ends up holding for `pane`: every report merged in
    /// order, token by token, a `null` removing the token, as Herdr does.
    fn held(herdr: &FakeHerdr, pane: &str) -> BTreeMap<String, String> {
        let mut tokens = BTreeMap::new();
        for params in reports(herdr)
            .iter()
            .filter(|params| params["pane_id"] == pane)
        {
            for (name, value) in params["tokens"].as_object().unwrap() {
                match value.as_str() {
                    Some(value) => tokens.insert(name.clone(), value.to_owned()),
                    None => tokens.remove(name),
                };
            }
        }
        tokens
    }

    fn expect_tokens(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    /// Asks again until no pane is out with the worker, so what the worker
    /// did is on the fake and in `standing` when this returns.
    #[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
    fn until_answered(restorer: &mut Restorer, agents: &[ProjectedAgent]) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            restorer.observe(agents, false);
            if !restorer
                .standing
                .values()
                .any(|standing| matches!(standing, Standing::InFlight { .. }))
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the worker never answered every pane"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_pane_that_lost_its_tokens_gets_what_its_file_last_said() {
        let home = tempfile::tempdir().unwrap();
        let herdr = herdr();
        count(&home, "w1:p1", WHO);
        let agents = [agent("w1:p1", Some("session-a"), &[])];
        let mut restorer = restorer(&home, &herdr);
        restorer.observe(&agents, true);
        until_answered(&mut restorer, &agents);
        assert_eq!(reports(&herdr), [full("w1:p1")]);
        assert_eq!(
            held(&herdr, "w1:p1"),
            expect_tokens(&[
                ("hide_hooks", &HOOK_VERSION.to_string()),
                ("hide_sub_working", "1"),
                ("hide_sub_done", "1"),
            ])
        );
    }

    #[test]
    fn the_counts_are_left_out_unless_herdr_names_the_session_the_file_names() {
        let home = tempfile::tempdir().unwrap();
        let herdr = herdr();
        for pane in ["w1:p1", "w1:p2", "w1:p3"] {
            count(&home, pane, WHO);
        }
        // Another session took the pane over, one Herdr names no session for,
        // and the session the file names.
        let agents = [
            agent("w1:p1", Some("session-b"), &[]),
            agent("w1:p2", None, &[]),
            agent("w1:p3", Some("session-a"), &[]),
        ];
        let mut restorer = restorer(&home, &herdr);
        restorer.observe(&agents, true);
        until_answered(&mut restorer, &agents);
        let mut sent = reports(&herdr);
        sent.sort_by_key(|params| params["pane_id"].as_str().unwrap().to_owned());
        assert_eq!(
            sent,
            [version_only("w1:p1"), version_only("w1:p2"), full("w1:p3")]
        );
        // Left out, so a reader sees them as unknown, never as a zero.
        assert!(sent[0]["tokens"].get("hide_sub_working").is_none());
    }

    #[test]
    fn a_file_with_no_session_never_puts_counts_back() {
        let home = tempfile::tempdir().unwrap();
        let herdr = herdr();
        // A runtime whose hook input names no session (the plugin totals).
        let blind = Reporter {
            session: None,
            ..WHO
        };
        count(&home, "w1:p1", blind);
        let agents = [agent("w1:p1", Some("session-a"), &[])];
        let mut restorer = restorer(&home, &herdr);
        restorer.observe(&agents, true);
        until_answered(&mut restorer, &agents);
        assert_eq!(reports(&herdr), [version_only("w1:p1")]);
    }

    #[test]
    fn a_file_another_agent_wrote_is_not_put_on_the_pane() {
        let home = tempfile::tempdir().unwrap();
        let herdr = herdr();
        count(&home, "w1:p1", WHO);
        // The pane was taken over by Codex since the file was written.
        count(
            &home,
            "w1:p2",
            Reporter {
                agent: Some("codex"),
                ..WHO
            },
        );
        let agents = [
            agent("w1:p1", Some("session-a"), &[]),
            agent("w1:p2", Some("session-a"), &[]),
        ];
        let mut restorer = restorer(&home, &herdr);
        restorer.observe(&agents, true);
        until_answered(&mut restorer, &agents);
        assert_eq!(reports(&herdr), [full("w1:p1")]);
    }

    #[test]
    fn a_pane_that_still_has_its_version_is_left_alone() {
        let home = tempfile::tempdir().unwrap();
        let herdr = herdr();
        count(&home, "w1:p1", WHO);
        count(&home, "w1:p2", WHO);
        let version = HOOK_VERSION.to_string();
        let agents = [
            agent("w1:p1", Some("session-a"), &[("hide_hooks", &version)]),
            agent("w1:p2", Some("session-a"), &[]),
        ];
        let mut restorer = restorer(&home, &herdr);
        restorer.observe(&agents, true);
        until_answered(&mut restorer, &agents);
        assert_eq!(reports(&herdr), [full("w1:p2")]);
    }

    #[test]
    fn a_pane_with_no_agent_or_one_hide_has_no_hook_for_is_left_alone() {
        let home = tempfile::tempdir().unwrap();
        let herdr = herdr();
        for pane in ["w1:p1", "w1:p2", "w1:p3"] {
            count(&home, pane, WHO);
        }
        let mut bare = agent("w1:p1", Some("session-a"), &[]);
        bare.agent = None;
        let mut unknown = agent("w1:p2", Some("session-a"), &[]);
        unknown.agent = Some("no-such-agent".into());
        let agents = [bare, unknown, agent("w1:p3", Some("session-a"), &[])];
        let mut restorer = restorer(&home, &herdr);
        restorer.observe(&agents, true);
        until_answered(&mut restorer, &agents);
        assert_eq!(reports(&herdr), [full("w1:p3")]);
    }

    #[test]
    fn a_pane_with_no_file_or_a_file_from_an_older_helper_is_not_guessed_at() {
        let home = tempfile::tempdir().unwrap();
        let herdr = herdr();
        // `w1:p2`'s file is one an older helper wrote: counts, no version.
        change(home.path(), "w1:p2", Change::Started(None), WHO).unwrap();
        let file = hide_agent_hooks::counters::state_directory(home.path()).join("w1_p2.json");
        std::fs::write(&file, br#"{"working":1,"done":0}"#).unwrap();
        count(&home, "w1:p3", WHO);
        let agents = [
            agent("w1:p1", Some("session-a"), &[]),
            agent("w1:p2", Some("session-a"), &[]),
            agent("w1:p3", Some("session-a"), &[]),
        ];
        let mut restorer = restorer(&home, &herdr);
        restorer.observe(&agents, true);
        until_answered(&mut restorer, &agents);
        assert_eq!(reports(&herdr), [full("w1:p3")]);
        // Neither is asked again while the connection holds: the helper's
        // next event reports the token and writes the file in one step.
        until_answered(&mut restorer, &agents);
        assert_eq!(reports(&herdr).len(), 1);
    }

    #[test]
    fn a_settled_pane_is_asked_again_only_after_the_connection_is_new() {
        let home = tempfile::tempdir().unwrap();
        let herdr = herdr();
        count(&home, "w1:p1", WHO);
        let agents = [agent("w1:p1", Some("session-a"), &[])];
        let mut restorer = restorer(&home, &herdr);
        restorer.observe(&agents, true);
        until_answered(&mut restorer, &agents);
        // The list still lacks the token: the read began before the report
        // landed, or the pane's agent has not reported since. Not a loss.
        restorer.observe(&agents, false);
        until_answered(&mut restorer, &agents);
        assert_eq!(reports(&herdr).len(), 1);
        // A new connection may be another server: ask again.
        restorer.observe(&agents, true);
        until_answered(&mut restorer, &agents);
        assert_eq!(reports(&herdr).len(), 2);
    }

    #[test]
    fn a_file_the_hook_changed_while_the_report_was_on_its_way_is_sent_again() {
        let home = tempfile::tempdir().unwrap();
        count(&home, "w1:p1", WHO);
        let events = home.path().to_path_buf();
        let mut answered = 0;
        let herdr = FakeHerdr::start("hook-tokens-race", move |_, _| {
            answered += 1;
            if answered == 1 {
                // The hook's next event lands before Herdr answers.
                change(&events, "w1:p1", Change::Started(None), WHO).unwrap();
            }
            json!({"type": "ok"})
        });
        let agents = [agent("w1:p1", Some("session-a"), &[])];
        let mut restorer = restorer(&home, &herdr);
        restorer.observe(&agents, true);
        until_answered(&mut restorer, &agents);
        let sent = reports(&herdr);
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0]["tokens"]["hide_sub_working"], "1");
        assert_eq!(sent[1]["tokens"]["hide_sub_working"], "2");
        assert_eq!(sent[1]["tokens"]["hide_sub_done"], "1");
    }

    #[test]
    fn a_session_that_took_the_pane_while_the_report_was_on_its_way_loses_the_counts() {
        let home = tempfile::tempdir().unwrap();
        count(&home, "w1:p1", WHO);
        let events = home.path().to_path_buf();
        let mut answered = 0;
        let herdr = FakeHerdr::start("hook-tokens-takeover", move |_, _| {
            answered += 1;
            if answered == 1 {
                // A new session's first event lands before Herdr answers.
                let next = Reporter {
                    session: Some("session-b"),
                    ..WHO
                };
                change(&events, "w1:p1", Change::SessionStarted, next).unwrap();
            }
            json!({"type": "ok"})
        });
        let agents = [agent("w1:p1", Some("session-a"), &[])];
        let mut restorer = restorer(&home, &herdr);
        restorer.observe(&agents, true);
        until_answered(&mut restorer, &agents);
        let sent = reports(&herdr);
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0], full("w1:p1"));
        // A report merges token by token, so leaving the counts out of the
        // second would keep the first session's on the pane.
        assert_eq!(sent[1], cleared("w1:p1"));
        assert_eq!(
            held(&herdr, "w1:p1"),
            expect_tokens(&[("hide_hooks", &HOOK_VERSION.to_string())])
        );
    }

    #[test]
    fn counts_put_on_a_pane_are_cleared_when_another_agent_takes_the_file() {
        let home = tempfile::tempdir().unwrap();
        count(&home, "w1:p1", WHO);
        let events = home.path().to_path_buf();
        let mut answered = 0;
        let herdr = FakeHerdr::start("hook-tokens-other-agent", move |_, _| {
            answered += 1;
            if answered == 1 {
                // Codex starts in the pane before Herdr answers.
                let codex = Reporter {
                    agent: Some("codex"),
                    session: Some("session-c"),
                };
                change(&events, "w1:p1", Change::SessionStarted, codex).unwrap();
            }
            json!({"type": "ok"})
        });
        let agents = [agent("w1:p1", Some("session-a"), &[])];
        let mut restorer = restorer(&home, &herdr);
        restorer.observe(&agents, true);
        until_answered(&mut restorer, &agents);
        assert_eq!(reports(&herdr), [full("w1:p1"), cleared("w1:p1")]);
        assert_eq!(
            held(&herdr, "w1:p1"),
            expect_tokens(&[("hide_hooks", &HOOK_VERSION.to_string())])
        );
    }

    #[test]
    fn a_refused_report_is_asked_again_a_few_times_and_then_left() {
        let home = tempfile::tempdir().unwrap();
        count(&home, "w1:p1", WHO);
        let herdr = FakeHerdr::start_with_errors("hook-tokens-refused", |_, _| {
            Err(("pane_not_found".into(), "no such pane".into()))
        });
        let agents = [agent("w1:p1", Some("session-a"), &[])];
        let mut restorer = restorer(&home, &herdr);
        restorer.observe(&agents, true);
        for _ in 0..5 {
            until_answered(&mut restorer, &agents);
        }
        assert_eq!(herdr.methods().len(), usize::from(ATTEMPTS));
        assert_eq!(
            restorer.standing.get("w1:p1"),
            Some(&Standing::Failed { attempts: ATTEMPTS })
        );
        // The next connection starts the count over.
        restorer.observe(&agents, true);
        for _ in 0..5 {
            until_answered(&mut restorer, &agents);
        }
        assert_eq!(herdr.methods().len(), usize::from(ATTEMPTS) * 2);
    }

    #[test]
    fn more_panes_than_a_batch_holds_are_taken_by_the_next_observations() {
        let home = tempfile::tempdir().unwrap();
        let herdr = herdr();
        let panes = BATCH_LIMIT + 5;
        let agents: Vec<_> = (0..panes)
            .map(|index| {
                let pane = format!("w1:p{index}");
                change(home.path(), &pane, Change::Started(None), WHO).unwrap();
                agent(&pane, Some("session-a"), &[])
            })
            .collect();
        let mut restorer = restorer(&home, &herdr);
        restorer.observe(&agents, true);
        until_answered(&mut restorer, &agents);
        until_answered(&mut restorer, &agents);
        let mut restored: Vec<_> = reports(&herdr)
            .iter()
            .map(|params| params["pane_id"].as_str().unwrap().to_owned())
            .collect();
        restored.sort();
        restored.dedup();
        assert_eq!(restored.len(), panes);
    }

    #[test]
    fn a_pane_herdr_stopped_listing_is_forgotten() {
        let home = tempfile::tempdir().unwrap();
        let herdr = herdr();
        count(&home, "w1:p1", WHO);
        let agents = [agent("w1:p1", Some("session-a"), &[])];
        let mut restorer = restorer(&home, &herdr);
        restorer.observe(&agents, true);
        until_answered(&mut restorer, &agents);
        assert_eq!(restorer.standing.len(), 1);
        restorer.observe(&[], false);
        assert!(restorer.standing.is_empty());
    }
}
