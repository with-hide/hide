//! Settles the starts Herdr holds open after their agent came to rest.
//!
//! Herdr 0.9.5 keeps an `agent.start` pending (`launch_pending`) until it
//! reconciles the start after the start's settle delay (3 s) with the agent
//! idle, and it reconciles only when the agent's state moves again or the
//! agent is read with `agent.get`; `agent.list` never does. An agent whose
//! first turn ends inside the settle delay, as Pi's and a resumed OpenCode's
//! do, and which then waits for its next prompt is therefore pending for as
//! long as nobody reads it, and a pending agent takes no letter or rename.
//! So every agent list the core reads is scanned for agents at rest with their
//! start pending, and each start Hide made is read with `agent.get` on the
//! sync-read worker, a bounded number of times, until it leaves that state.
//!
//! A read is safe only inside the start's deadline: past it, Herdr applies the
//! deadline on the read and clears the agent's name. Herdr's agent list says
//! neither when a start was made nor what its deadline is, so only Hide's own
//! starts are read: their deadline is Hide's (`AGENT_START_DEADLINE`), and the
//! time Herdr took each is recorded by the start's terminal ([`record_start`]),
//! which no other Herdr server names. A pending start Hide has no record of,
//! one an operator made with `herdr agent start` or one made before this
//! daemon started, is cold: it is left to Herdr and logged once.
//!
//! This is a workaround for herdrdev/herdr#3853: once Herdr settles a pending
//! start when its settle delay or deadline passes, this module and the pieces
//! docs/ARCHITECTURE.md lists for it are deleted.
use super::*;

/// Reads of one start before it is left to Herdr, with one diagnostic.
const READS_PER_PANE: u32 = 8;
/// Panes read in one batch, so a batch holds the worker at most this many
/// read timeouts.
const READS_PER_BATCH: usize = 4;
/// Starts tracked at once; one past it is reported and left unread.
const PANE_LIMIT: usize = 64;
/// The least time between two reads of one start, so an agent list re-asked
/// for a burst of events does not spend its reads at once.
const READ_SPACING: Duration = Duration::from_secs(1);
/// Herdr's settle delay: a start read before it is still pending afterwards,
/// so a read then would spend one of its reads for nothing.
const SETTLE_DELAY: Duration = Duration::from_secs(3);
/// How long after Herdr took a start it is read: well inside the deadline
/// every start Hide makes carries, less however long Herdr took to answer it.
const READ_WINDOW: Duration = Duration::from_secs(90);
const _: () =
    assert!(READ_WINDOW.as_secs() + 30 <= crate::agent_start::AGENT_START_DEADLINE.as_secs());
/// Starts recorded at once; each record is dropped once its read window has
/// passed, so this is reached only by more starts in 90 s than any session makes.
const START_LIMIT: usize = 256;

/// When Herdr took each start Hide made, by the start's terminal.
static STARTS: std::sync::Mutex<Starts> = std::sync::Mutex::new(Starts::new());

/// Records that Herdr took a start Hide made in `terminal_id` at `at`.
pub(crate) fn record_start(terminal_id: &str, at: Instant) {
    STARTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .record(terminal_id, at);
}

/// When Herdr took the start Hide made in `terminal_id`, while its read window
/// lasts.
pub(super) fn recorded_start(terminal_id: &str) -> Option<Instant> {
    STARTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(terminal_id)
}

struct Starts {
    by_terminal: BTreeMap<String, Instant>,
}

impl Starts {
    const fn new() -> Self {
        Self {
            by_terminal: BTreeMap::new(),
        }
    }

    /// Records a start and drops every record whose read window has passed;
    /// one past [`START_LIMIT`] drops the oldest, with a diagnostic.
    fn record(&mut self, terminal_id: &str, at: Instant) {
        self.by_terminal
            .retain(|_, started| at.saturating_duration_since(*started) < READ_WINDOW);
        if self.by_terminal.len() >= START_LIMIT && !self.by_terminal.contains_key(terminal_id) {
            let oldest = self
                .by_terminal
                .iter()
                .min_by_key(|(_, started)| **started)
                .map(|(terminal, _)| terminal.clone());
            if let Some(oldest) = oldest {
                self.by_terminal.remove(&oldest);
                crate::diagnostic!(json!({
                    "component": "session_sync",
                    "kind": "launch_settle.start_limit",
                    "terminal_id": oldest,
                    "limit": START_LIMIT,
                }));
            }
        }
        self.by_terminal.insert(terminal_id.to_owned(), at);
    }

    fn get(&self, terminal_id: &str) -> Option<Instant> {
        self.by_terminal.get(terminal_id).copied()
    }
}

/// The starts this coordinator has seen at rest and pending, kept across its
/// reconnects: each is keyed by its terminal, so a pane id another server
/// reuses after a restart is never taken for it, and a reconnect neither
/// refills a start's reads nor reports a cold one again.
#[derive(Default)]
pub(super) struct LaunchSettle {
    /// Each start the last agent list showed at rest and pending, by terminal.
    panes: BTreeMap<String, Pane>,
    over_limit: bool,
}

struct Pane {
    pane_id: String,
    first_seen: Instant,
    /// When Herdr took the start, once Hide's record of it is found.
    started: Option<Instant>,
    reads: u32,
    last_read: Option<Instant>,
    /// Out of reads, cold or past its read window, and reported once.
    left_to_herdr: bool,
}

impl LaunchSettle {
    /// Takes one agent list: forgets every start no longer at rest and
    /// pending, and hands `ask` the panes due a read now. `started` is when
    /// Herdr took a start Hide made, by terminal ([`recorded_start`]). `ask`
    /// answers false while the previous batch is still out, so no pane is read
    /// again before its last read answered, and the batch counts for none.
    pub(super) fn observe(
        &mut self,
        target: &str,
        agents: &[ProjectedAgent],
        now: Instant,
        started: impl Fn(&str) -> Option<Instant>,
        ask: impl FnOnce(Vec<String>) -> bool,
    ) {
        let at_rest_pending = |agent: &ProjectedAgent| {
            agent.launch_pending && matches!(agent.agent_status.as_deref(), Some("idle" | "done"))
        };
        // Only a list that follows a tracked start is indexed, so a session
        // with none pays one pass over its list.
        let listed: BTreeMap<&str, &ProjectedAgent> = if self.panes.is_empty() {
            BTreeMap::new()
        } else {
            agents
                .iter()
                .map(|agent| (agent.terminal_id.as_str(), agent))
                .collect()
        };
        self.panes.retain(|terminal, pane| {
            let agent = listed.get(terminal.as_str()).copied();
            if agent.is_some_and(at_rest_pending) {
                return true;
            }
            if pane.reads > 0 {
                // Settled: listed again with Herdr's start closed. Left: closed,
                // or moved off rest with the start still open, which Herdr
                // reconciles on that move itself.
                let kind = if agent.is_some_and(|agent| !agent.launch_pending) {
                    "launch_settle.settled"
                } else {
                    "launch_settle.left"
                };
                crate::diagnostic!(json!({
                    "component": "session_sync",
                    "kind": kind,
                    "target": target,
                    "pane_id": pane.pane_id,
                    "reads": pane.reads,
                    "agent_status": agent.and_then(|agent| agent.agent_status.clone()),
                }));
            }
            false
        });
        let stuck: Vec<&ProjectedAgent> = agents
            .iter()
            .filter(|agent| at_rest_pending(agent))
            .collect();
        let over_limit = stuck.len() > PANE_LIMIT;
        if over_limit && !self.over_limit {
            crate::diagnostic!(json!({
                "component": "session_sync",
                "kind": "launch_settle.over_limit",
                "target": target,
                "panes": stuck.len(),
                "limit": PANE_LIMIT,
            }));
        }
        self.over_limit = over_limit;
        for agent in stuck {
            if self.panes.len() >= PANE_LIMIT {
                break;
            }
            self.panes
                .entry(agent.terminal_id.clone())
                .or_insert_with(|| Pane {
                    pane_id: agent.pane_id.clone(),
                    first_seen: now,
                    started: None,
                    reads: 0,
                    last_read: None,
                    left_to_herdr: false,
                });
        }

        let mut due = Vec::new();
        for (terminal, pane) in &mut self.panes {
            if pane.left_to_herdr {
                continue;
            }
            // Looked up until found: Herdr can list a start before its answer
            // has reached the start that records it.
            pane.started = pane.started.or_else(|| started(terminal));
            let left = match pane.started {
                None if now.saturating_duration_since(pane.first_seen) >= SETTLE_DELAY => {
                    Some("launch_settle.cold")
                }
                None => None,
                Some(at) if now.saturating_duration_since(at) >= READ_WINDOW => {
                    Some("launch_settle.window_passed")
                }
                Some(_) if pane.reads >= READS_PER_PANE => Some("launch_settle.exhausted"),
                Some(_) => None,
            };
            if let Some(kind) = left {
                pane.left_to_herdr = true;
                crate::diagnostic!(json!({
                    "component": "session_sync",
                    "kind": kind,
                    "target": target,
                    "pane_id": pane.pane_id,
                    "reads": pane.reads,
                }));
                continue;
            }
            let Some(at) = pane.started else {
                continue;
            };
            if now.saturating_duration_since(at) < SETTLE_DELAY
                || pane
                    .last_read
                    .is_some_and(|read| now.saturating_duration_since(read) < READ_SPACING)
            {
                continue;
            }
            if due.len() < READS_PER_BATCH {
                due.push((terminal.clone(), pane.pane_id.clone()));
            }
        }
        if due.is_empty() || !ask(due.iter().map(|(_, pane_id)| pane_id.clone()).collect()) {
            return;
        }
        for (terminal, _) in due {
            if let Some(pane) = self.panes.get_mut(&terminal) {
                pane.reads += 1;
                pane.last_read = Some(now);
            }
        }
    }

    /// Records each read that failed; what a read settled reaches the core
    /// with the next agent list, so an answer changes nothing here.
    pub(super) fn answered(&self, target: &str, reads: &[(String, Result<(), SessionFetchError>)]) {
        for (pane_id, read) in reads {
            if let Err(error) = read {
                crate::diagnostic!(json!({
                    "component": "session_sync",
                    "kind": "launch_settle.read_failed",
                    "target": target,
                    "pane_id": pane_id,
                    "state": error.state(),
                    "message": error.message(),
                }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(pane_id: &str, status: &str, launch_pending: bool) -> ProjectedAgent {
        ProjectedAgent {
            pane_id: pane_id.into(),
            terminal_id: format!("term-{pane_id}"),
            name: Some(format!("{pane_id}-agent")),
            workspace_id: "w1".into(),
            tab_id: "w1:t1".into(),
            cwd: None,
            agent: Some("pi".into()),
            agent_status: Some(status.into()),
            agent_session: None,
            spawned_from_pane_id: None,
            spawned_from_machine_id: None,
            declared_parent_session: None,
            lineage_session: None,
            state_change_seq: 1,
            completion_seq: None,
            launch_pending,
            tokens: BTreeMap::new(),
        }
    }

    /// Hide made every start at `made`, but those in `cold`.
    fn hide_made(made: Instant, cold: &[&str]) -> impl Fn(&str) -> Option<Instant> {
        let cold: Vec<String> = cold.iter().map(|pane| format!("term-{pane}")).collect();
        move |terminal| (!cold.iter().any(|cold| cold == terminal)).then_some(made)
    }

    /// Observes `agents` at `now`, Hide having made every start at `made`,
    /// with a worker that takes the batch; returns the panes it was asked to
    /// read.
    fn tick(
        settle: &mut LaunchSettle,
        agents: &[ProjectedAgent],
        made: Instant,
        now: Instant,
    ) -> Vec<String> {
        let mut asked = Vec::new();
        settle.observe("local", agents, now, hide_made(made, &[]), |panes| {
            asked = panes;
            true
        });
        asked
    }

    #[test]
    fn reads_only_agents_at_rest_whose_start_is_pending() {
        let mut settle = LaunchSettle::default();
        let agents = [
            agent("idle", "idle", true),
            agent("done", "done", true),
            agent("working", "working", true),
            agent("blocked", "blocked", true),
            agent("settled", "idle", false),
        ];
        let made = Instant::now();

        assert_eq!(
            tick(&mut settle, &agents, made, made + SETTLE_DELAY),
            ["done", "idle"]
        );
    }

    /// Herdr settles nothing inside its settle delay, so a read there would
    /// spend one of the start's reads for nothing.
    #[test]
    fn a_start_is_first_read_once_herdrs_settle_delay_has_passed() {
        let mut settle = LaunchSettle::default();
        let stuck = [agent("p1", "idle", true)];
        let made = Instant::now();

        assert!(tick(&mut settle, &stuck, made, made + Duration::from_millis(500)).is_empty());
        assert!(
            tick(
                &mut settle,
                &stuck,
                made,
                made + Duration::from_millis(2900)
            )
            .is_empty()
        );
        assert_eq!(tick(&mut settle, &stuck, made, made + SETTLE_DELAY), ["p1"]);
    }

    /// herdrdev/herdr#3853's third case: a read past a start's deadline makes
    /// Herdr clear the agent's name. A start Hide has no record of, whose
    /// deadline is unknown, is never read and is reported once; neither is
    /// one whose read window has passed.
    #[test]
    fn a_start_hide_did_not_make_or_past_its_window_is_left_to_herdr_and_reported_once() {
        let mut settle = LaunchSettle::default();
        let stuck = [agent("operator", "idle", true), agent("late", "idle", true)];
        let made = Instant::now();
        let mut asked = Vec::new();
        let ((), records) = crate::diagnostics::capture(|| {
            for now in [
                made + SETTLE_DELAY,
                made + READ_WINDOW,
                made + READ_WINDOW + READ_SPACING,
            ] {
                settle.observe(
                    "local",
                    &stuck,
                    now,
                    hide_made(made, &["operator"]),
                    |panes| {
                        asked.extend(panes);
                        true
                    },
                );
            }
        });

        assert_eq!(asked, ["late"], "the cold start is never read");
        let reported: Vec<_> = records
            .iter()
            .map(|record| (record["kind"].clone(), record["pane_id"].clone()))
            .collect();
        assert_eq!(
            reported,
            [
                (json!("launch_settle.window_passed"), json!("late")),
                (json!("launch_settle.cold"), json!("operator")),
            ]
        );
    }

    #[test]
    fn a_pane_is_read_at_most_once_a_second_until_it_settles() {
        let mut settle = LaunchSettle::default();
        let stuck = [agent("p1", "idle", true)];
        let made = Instant::now();
        let start = made + SETTLE_DELAY;

        assert_eq!(tick(&mut settle, &stuck, made, start), ["p1"]);
        assert!(
            tick(
                &mut settle,
                &stuck,
                made,
                start + Duration::from_millis(250)
            )
            .is_empty()
        );
        assert_eq!(
            tick(&mut settle, &stuck, made, start + READ_SPACING),
            ["p1"]
        );

        let ((), records) = crate::diagnostics::capture(|| {
            tick(
                &mut settle,
                &[agent("p1", "idle", false)],
                made,
                start + 2 * READ_SPACING,
            );
        });
        assert!(settle.panes.is_empty());
        assert_eq!(records[0]["kind"], "launch_settle.settled");
        assert_eq!(records[0]["reads"], 2);
    }

    /// A start that closes, or moves off rest with Herdr's start still open,
    /// is forgotten as left, not as settled.
    #[test]
    fn a_closed_or_working_pane_is_forgotten_as_left_not_settled() {
        let mut settle = LaunchSettle::default();
        let stuck = [agent("closes", "idle", true), agent("works", "idle", true)];
        let made = Instant::now();
        tick(&mut settle, &stuck, made, made + SETTLE_DELAY);

        let ((), records) = crate::diagnostics::capture(|| {
            tick(
                &mut settle,
                &[agent("works", "working", true)],
                made,
                made + SETTLE_DELAY + READ_SPACING,
            );
        });

        assert!(settle.panes.is_empty());
        let left: Vec<_> = records
            .iter()
            .map(|record| {
                (
                    record["kind"].clone(),
                    record["pane_id"].clone(),
                    record["agent_status"].clone(),
                )
            })
            .collect();
        assert_eq!(
            left,
            [
                (json!("launch_settle.left"), json!("closes"), Value::Null),
                (
                    json!("launch_settle.left"),
                    json!("works"),
                    json!("working")
                ),
            ]
        );
    }

    #[test]
    fn a_pane_whose_batch_is_still_out_is_not_read_again() {
        let mut settle = LaunchSettle::default();
        let stuck = [agent("p1", "idle", true)];
        let made = Instant::now();
        let start = made + SETTLE_DELAY;
        let mut offered = Vec::new();
        settle.observe("local", &stuck, start, hide_made(made, &[]), |panes| {
            offered = panes;
            false
        });

        assert_eq!(offered, ["p1"]);
        assert_eq!(settle.panes["term-p1"].reads, 0);
        // A refused batch spends no spacing either, so the next list asks.
        assert_eq!(tick(&mut settle, &stuck, made, start), ["p1"]);
    }

    #[test]
    fn a_pane_that_never_settles_is_reported_once_and_read_no_more() {
        let mut settle = LaunchSettle::default();
        let stuck = [agent("p1", "idle", true)];
        let made = Instant::now();
        let start = made + SETTLE_DELAY;
        for read in 0..READS_PER_PANE {
            assert_eq!(
                tick(&mut settle, &stuck, made, start + read * READ_SPACING),
                ["p1"]
            );
        }

        let (asked, records) = crate::diagnostics::capture(|| {
            let after = start + READS_PER_PANE * READ_SPACING;
            [
                tick(&mut settle, &stuck, made, after),
                tick(&mut settle, &stuck, made, after + READ_SPACING),
            ]
        });
        assert!(asked.iter().all(Vec::is_empty));
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["kind"], "launch_settle.exhausted");
        assert_eq!(records[0]["pane_id"], "p1");
        assert_eq!(records[0]["reads"], READS_PER_PANE);
    }

    #[test]
    fn one_batch_reads_at_most_four_panes_and_the_rest_wait() {
        let mut settle = LaunchSettle::default();
        let stuck: Vec<_> = (0..6)
            .map(|n| agent(&format!("p{n}"), "idle", true))
            .collect();
        let made = Instant::now();
        let start = made + SETTLE_DELAY;

        assert_eq!(
            tick(&mut settle, &stuck, made, start),
            ["p0", "p1", "p2", "p3"]
        );
        assert_eq!(tick(&mut settle, &stuck, made, start), ["p4", "p5"]);
    }

    #[test]
    fn more_stuck_panes_than_the_limit_are_reported_once_and_not_tracked() {
        let mut settle = LaunchSettle::default();
        let stuck: Vec<_> = (0..=PANE_LIMIT)
            .map(|n| agent(&format!("p{n:03}"), "idle", true))
            .collect();
        let made = Instant::now();

        let ((), records) = crate::diagnostics::capture(|| {
            tick(&mut settle, &stuck, made, made + SETTLE_DELAY);
            tick(&mut settle, &stuck, made, made + SETTLE_DELAY);
        });
        assert_eq!(settle.panes.len(), PANE_LIMIT);
        let over: Vec<_> = records
            .iter()
            .filter(|record| record["kind"] == "launch_settle.over_limit")
            .collect();
        assert_eq!(over.len(), 1);
        assert_eq!(over[0]["panes"], PANE_LIMIT + 1);
    }

    /// A pane id names a different start once its server restarted, so a
    /// start is kept by its terminal: a new start in a reused pane id is a
    /// new start with its own reads.
    #[test]
    fn a_reused_pane_id_with_another_terminal_is_another_start() {
        let mut settle = LaunchSettle::default();
        let made = Instant::now();
        let start = made + SETTLE_DELAY;
        let before = agent("p1", "idle", true);
        let mut after = agent("p1", "idle", true);
        after.terminal_id = "term-restarted".into();
        for read in 0..READS_PER_PANE {
            tick(
                &mut settle,
                std::slice::from_ref(&before),
                made,
                start + read * READ_SPACING,
            );
        }

        let later = start + READS_PER_PANE * READ_SPACING;
        assert_eq!(
            tick(&mut settle, &[after], later, later + SETTLE_DELAY),
            ["p1"]
        );
    }

    /// Records live for their read window, and the oldest gives way past the
    /// limit with a diagnostic, so the record never grows past it.
    #[test]
    fn start_records_last_their_window_and_are_capped() {
        let mut starts = Starts::new();
        let at = Instant::now();
        starts.record("term-old", at);
        starts.record("term-new", at + READ_WINDOW);
        assert_eq!(starts.get("term-old"), None);
        assert_eq!(starts.get("term-new"), Some(at + READ_WINDOW));

        let mut starts = Starts::new();
        for n in 0..START_LIMIT {
            starts.record(
                &format!("term-{n:03}"),
                at + Duration::from_millis(n as u64),
            );
        }
        let ((), records) = crate::diagnostics::capture(|| {
            starts.record("term-one-more", at + Duration::from_secs(1));
        });
        assert_eq!(starts.by_terminal.len(), START_LIMIT);
        assert_eq!(starts.get("term-000"), None);
        assert_eq!(records[0]["kind"], "launch_settle.start_limit");
        assert_eq!(records[0]["terminal_id"], "term-000");
    }
}
