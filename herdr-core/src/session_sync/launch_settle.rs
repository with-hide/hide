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
//! start pending, and each is read with `agent.get` on the sync-read worker,
//! a bounded number of times, until it leaves that state. A read past the
//! start's deadline is where Herdr applies the deadline and clears the name;
//! that is Herdr's own rule, and this read does not hold it off.
//!
//! This is a workaround for herdrdev/herdr#3853: once Herdr settles a pending
//! start when its settle delay or deadline passes, this module and its read
//! (`SyncRead::Launches`) are deleted.
use super::*;

/// Reads of one pane before it is left to Herdr, with one diagnostic.
const READS_PER_PANE: u32 = 8;
/// Panes read in one batch, so a batch holds the worker at most this many
/// read timeouts.
const READS_PER_BATCH: usize = 4;
/// Panes tracked at once; one past it is reported and left unread.
const PANE_LIMIT: usize = 64;
/// The least time between two reads of one pane, so an agent list re-asked
/// for a burst of events does not spend a pane's reads before Herdr's settle
/// delay has passed.
const READ_SPACING: Duration = Duration::from_secs(1);

#[derive(Default)]
pub(super) struct LaunchSettle {
    /// Each pane the last agent list showed at rest with its start pending.
    panes: BTreeMap<String, Pane>,
    over_limit: bool,
}

#[derive(Default)]
struct Pane {
    reads: u32,
    last_read: Option<Instant>,
    exhausted: bool,
}

impl LaunchSettle {
    /// Takes one agent list: forgets every pane no longer at rest with its
    /// start pending, and hands `ask` the panes due a read now. `ask` answers
    /// false while the previous batch is still out, so no pane is read again
    /// before its last read answered, and the batch counts for none of them.
    pub(super) fn observe(
        &mut self,
        target: &str,
        agents: &[ProjectedAgent],
        now: Instant,
        ask: impl FnOnce(Vec<String>) -> bool,
    ) {
        let stuck: BTreeSet<&str> = agents
            .iter()
            .filter(|agent| {
                agent.launch_pending
                    && matches!(agent.agent_status.as_deref(), Some("idle" | "done"))
            })
            .map(|agent| agent.pane_id.as_str())
            .collect();
        self.panes.retain(|pane_id, pane| {
            let keep = stuck.contains(pane_id.as_str());
            if !keep && pane.reads > 0 {
                crate::diagnostic!(json!({
                    "component": "session_sync",
                    "kind": "launch_settle.settled",
                    "target": target,
                    "pane_id": pane_id,
                    "reads": pane.reads,
                }));
            }
            keep
        });
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
        for pane_id in stuck {
            if self.panes.len() >= PANE_LIMIT {
                break;
            }
            self.panes.entry(pane_id.to_owned()).or_default();
        }

        let mut due = Vec::new();
        for (pane_id, pane) in &mut self.panes {
            if pane
                .last_read
                .is_some_and(|read| now.duration_since(read) < READ_SPACING)
            {
                continue;
            }
            if pane.reads >= READS_PER_PANE {
                if !pane.exhausted {
                    pane.exhausted = true;
                    crate::diagnostic!(json!({
                        "component": "session_sync",
                        "kind": "launch_settle.exhausted",
                        "target": target,
                        "pane_id": pane_id,
                        "reads": pane.reads,
                    }));
                }
                continue;
            }
            if due.len() < READS_PER_BATCH {
                due.push(pane_id.clone());
            }
        }
        if due.is_empty() || !ask(due.clone()) {
            return;
        }
        for pane_id in due {
            if let Some(pane) = self.panes.get_mut(&pane_id) {
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

    /// Observes `agents` at `now` with a worker that takes the batch, and
    /// returns the panes it was asked to read.
    fn tick(settle: &mut LaunchSettle, agents: &[ProjectedAgent], now: Instant) -> Vec<String> {
        let mut asked = Vec::new();
        settle.observe("local", agents, now, |panes| {
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

        assert_eq!(tick(&mut settle, &agents, Instant::now()), ["done", "idle"]);
    }

    #[test]
    fn a_pane_is_read_at_most_once_a_second_until_it_settles() {
        let mut settle = LaunchSettle::default();
        let stuck = [agent("p1", "idle", true)];
        let start = Instant::now();

        assert_eq!(tick(&mut settle, &stuck, start), ["p1"]);
        assert!(tick(&mut settle, &stuck, start + Duration::from_millis(250)).is_empty());
        assert_eq!(tick(&mut settle, &stuck, start + READ_SPACING), ["p1"]);

        let ((), records) = crate::diagnostics::capture(|| {
            tick(
                &mut settle,
                &[agent("p1", "idle", false)],
                start + 2 * READ_SPACING,
            );
        });
        assert!(settle.panes.is_empty());
        assert_eq!(records[0]["kind"], "launch_settle.settled");
        assert_eq!(records[0]["reads"], 2);
    }

    #[test]
    fn a_pane_whose_batch_is_still_out_is_not_read_again() {
        let mut settle = LaunchSettle::default();
        let stuck = [agent("p1", "idle", true)];
        let start = Instant::now();
        let mut offered = Vec::new();
        settle.observe("local", &stuck, start, |panes| {
            offered = panes;
            false
        });

        assert_eq!(offered, ["p1"]);
        assert_eq!(settle.panes["p1"].reads, 0);
        // A refused batch spends no spacing either, so the next list asks.
        assert_eq!(tick(&mut settle, &stuck, start), ["p1"]);
    }

    #[test]
    fn a_pane_that_never_settles_is_reported_once_and_read_no_more() {
        let mut settle = LaunchSettle::default();
        let stuck = [agent("p1", "idle", true)];
        let start = Instant::now();
        for read in 0..READS_PER_PANE {
            assert_eq!(
                tick(&mut settle, &stuck, start + read * READ_SPACING),
                ["p1"]
            );
        }

        let (asked, records) = crate::diagnostics::capture(|| {
            let after = start + READS_PER_PANE * READ_SPACING;
            [
                tick(&mut settle, &stuck, after),
                tick(&mut settle, &stuck, after + READ_SPACING),
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
        let start = Instant::now();

        assert_eq!(tick(&mut settle, &stuck, start), ["p0", "p1", "p2", "p3"]);
        assert_eq!(tick(&mut settle, &stuck, start), ["p4", "p5"]);
    }

    #[test]
    fn more_stuck_panes_than_the_limit_are_reported_once_and_not_tracked() {
        let mut settle = LaunchSettle::default();
        let stuck: Vec<_> = (0..=PANE_LIMIT)
            .map(|n| agent(&format!("p{n:03}"), "idle", true))
            .collect();
        let start = Instant::now();

        let ((), records) = crate::diagnostics::capture(|| {
            tick(&mut settle, &stuck, start);
            tick(&mut settle, &stuck, start);
        });
        assert_eq!(settle.panes.len(), PANE_LIMIT);
        let over: Vec<_> = records
            .iter()
            .filter(|record| record["kind"] == "launch_settle.over_limit")
            .collect();
        assert_eq!(over.len(), 1);
        assert_eq!(over[0]["panes"], PANE_LIMIT + 1);
    }

    #[test]
    fn a_closed_pane_is_forgotten() {
        let mut settle = LaunchSettle::default();
        tick(&mut settle, &[agent("p1", "idle", true)], Instant::now());

        tick(&mut settle, &[], Instant::now());

        assert!(settle.panes.is_empty());
    }
}
