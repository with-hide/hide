//! Agents' waits and the clock every wait shares (PRD core-wait).
//!
//! An agent asks with `hide wait` (D-21): the wait is kept in
//! `core-state.json` with its target, deadline and the registration that
//! waits (B26), the agent is marked to sleep at its next rest by the same
//! verdict a report's sleep takes (D-09, `agent_sleep::missions`), and when
//! the wait is met, fails or passes its deadline the agent gets one Hide
//! notice (a status cell, never a letter: Hide sends no letter as itself,
//! `docs/delivery.md`), which wakes it if it sleeps (B17). The notice is the
//! ended wait itself, kept until the agent has seen it.
//!
//! A mission or node target is judged from the mission book on every save
//! of it (`judge_book_waits`, D-11) and a pull request, CI or branch target
//! by the code host reads `runtime/mission_outside.rs` makes for waiting
//! nodes too. The core's tick (`tick_waits`) passes deadlines, ends the waits
//! of agents that ended (D-22) and looks again at every followed project.

use serde_json::{Value, json};

use super::Runtime;
use super::mission_outside::{Found, Watch};
use crate::delivery::Actor;
use crate::waits::{
    AGENT_WAIT_LIMIT, AgentWait, InputRefusal, MACHINE_WAIT_LIMIT, WaitInput, WaitOutcome,
    WaitTarget,
};

/// How often the tick passes deadlines and looks at the followed projects;
/// the coordinator's tick runs every 250 ms.
const TICK_EVERY_MS: u64 = 10_000;

/// A deadline's seven days, given to a waiting node from before deadlines
/// at the first read of this build (D-19).
const DEFAULT_WAIT_MS: u64 = crate::waits::DEFAULT_WAIT_MINUTES as u64 * 60_000;

/// The subject of the notice about agent wait `id`.
pub(crate) fn wait_subject(id: &str) -> String {
    format!("wait:{id}")
}

/// Gives every waiting node with no deadline one from `now` (D-19): a node
/// from before deadlines, read by this build for the first time.
pub(super) fn settle_wait_deadlines(book: &mut crate::missions::MissionBook, now: u64) -> bool {
    let mut changed = false;
    for mission in &mut book.missions {
        for node in &mut mission.nodes {
            if node.kind == crate::missions::NodeKind::Wait && node.wait_until_unix_ms.is_none() {
                node.wait_until_unix_ms = Some(now.saturating_add(DEFAULT_WAIT_MS));
                changed = true;
            }
        }
    }
    changed
}

/// What `hide wait list` and an answer say of one wait.
fn wait_json(wait: &AgentWait) -> Value {
    json!({
        "id": wait.id,
        "target": wait.target,
        "words": wait.target.words(),
        "until_unix_ms": wait.until_unix_ms,
    })
}

impl Runtime {
    /// The core's wait clock (D-11, D-12): every `TICK_EVERY_MS` it passes
    /// deadlines, ends the waits of agents that ended, judges the book's
    /// targets and looks again at the followed projects whose look is due.
    /// It runs with no window attached.
    pub(crate) fn tick_waits(&mut self, now: u64) -> bool {
        if now < self.missions.waits_next_tick_unix_ms {
            return false;
        }
        self.missions.waits_next_tick_unix_ms = now.saturating_add(TICK_EVERY_MS);
        let mut changed = self.end_waits_of_ended_agents(now);
        changed |= self.pass_wait_deadlines(now);
        if self.missions.unreadable.is_none() && self.judge_book_waits(now) {
            self.save_missions();
            changed = true;
        }
        changed | self.look_at_followed(now)
    }

    /// Ends every wait whose deadline passed (D-07): a waiting node is held
    /// with its raise, an agent's wait ends with its notice. A deadline that
    /// passed while Hide was off ends at its first tick (B26).
    fn pass_wait_deadlines(&mut self, now: u64) -> bool {
        let mut book_moved = false;
        if self.missions.unreadable.is_none() {
            book_moved |= settle_wait_deadlines(&mut self.missions.book, now);
            let mut events = Vec::new();
            for mission in &mut self.missions.book.missions {
                if !mission.status.open() {
                    continue;
                }
                for at in 0..mission.nodes.len() {
                    let node = &mission.nodes[at];
                    if !super::mission_outside::judged(node)
                        || node.wait_until_unix_ms.is_none_or(|until| until > now)
                    {
                        continue;
                    }
                    let (made, moved) = super::mission_outside::settle_wait_node(
                        mission,
                        at,
                        Some(WaitOutcome::Expired),
                        now,
                    );
                    if moved {
                        events.push((mission.id.clone(), made));
                        book_moved = true;
                    }
                }
            }
            for (id, made) in events {
                self.stage_mission_events(&id, made);
            }
            if book_moved {
                self.save_missions();
            }
        }
        let expired: Vec<String> = self
            .snapshot
            .ui_state
            .waits
            .open()
            .filter(|wait| wait.until_unix_ms <= now)
            .map(|wait| wait.id.clone())
            .collect();
        let mut ended = false;
        for id in expired {
            ended |= self.end_agent_wait(&id, WaitOutcome::Expired, now);
        }
        book_moved | ended
    }

    /// Judges every waiting node and agent wait on a mission or a node by the
    /// book as it stands (D-03, D-04, D-11), again until nothing moves, since
    /// a node a wait makes done can meet another wait on it; at most as many
    /// rounds as there are waiting nodes, as a loop is refused when a wait
    /// is made (D-22). Runs on every save of the book and changes it in
    /// place, so it saves nothing itself; whether the book moved.
    pub(super) fn judge_book_waits(&mut self, now: u64) -> bool {
        let mut book_moved = false;
        let rounds = self
            .missions
            .book
            .missions
            .iter()
            .flat_map(|mission| &mission.nodes)
            .filter(|node| node.kind == crate::missions::NodeKind::Wait)
            .count()
            + 1;
        for _ in 0..rounds {
            let mut settles = Vec::new();
            for mission in &self.missions.book.missions {
                if !mission.status.open() {
                    continue;
                }
                for (at, node) in mission.nodes.iter().enumerate() {
                    if !super::mission_outside::judged(node) {
                        continue;
                    }
                    let Some(target) = node.wait_target.as_ref().filter(|t| t.mission().is_some())
                    else {
                        continue;
                    };
                    let outcome = crate::waits::judge_book(&self.missions.book, target);
                    // A node not started yet starts waiting at its first
                    // judgment, as a read answered starts a code host wait.
                    if outcome.is_some() || node.status == crate::missions::NodeStatus::Planned {
                        settles.push((mission.id.clone(), at, outcome));
                    }
                }
            }
            let mut moved = false;
            for (id, at, outcome) in settles {
                let Some(mission) = self.missions.book.get_mut(&id) else {
                    continue;
                };
                let (events, changed) =
                    super::mission_outside::settle_wait_node(mission, at, outcome, now);
                if changed {
                    // Staged only: this runs inside the book's save.
                    self.stage_mission_events(&id, events);
                    moved = true;
                }
            }
            book_moved |= moved;
            if !moved {
                break;
            }
        }
        let ends: Vec<(String, WaitOutcome)> = self
            .snapshot
            .ui_state
            .waits
            .open()
            .filter_map(|wait| {
                wait.target.mission()?;
                Some((
                    wait.id.clone(),
                    crate::waits::judge_book(&self.missions.book, &wait.target)?,
                ))
            })
            .collect();
        for (id, outcome) in ends {
            self.end_agent_wait(&id, outcome, now);
        }
        book_moved
    }

    /// Ends the waits, and lets go of the ended ones, whose agent's
    /// registration ended (D-22, B20): nobody is told, and the log says so.
    fn end_waits_of_ended_agents(&mut self, now: u64) -> bool {
        let Ok(ledger) = self.delivery_ledger.as_ref() else {
            return false;
        };
        let live = |agent: &str| {
            ledger
                .agents
                .iter()
                .any(|record| record.id == agent && !record.ended)
        };
        let gone: Vec<(String, bool)> = self
            .snapshot
            .ui_state
            .waits
            .waits
            .iter()
            .filter(|wait| !live(&wait.agent_id))
            .map(|wait| (wait.id.clone(), wait.open()))
            .collect();
        if gone.is_empty() {
            return false;
        }
        for (id, open) in &gone {
            if *open {
                crate::diagnostic!(json!({
                    "component": "waits", "kind": "wait.agent_ended",
                    "wait": id, "at_unix_ms": now,
                }));
            }
            self.missions.outside.forget_agent(id);
        }
        self.snapshot
            .ui_state
            .waits
            .waits
            .retain(|wait| !gone.iter().any(|(id, _)| *id == wait.id));
        self.persist_ui_state();
        true
    }

    /// Ends agent wait `id` with `outcome` at `now`: its notice stands for
    /// its agent from now on and wakes it if it sleeps (B17). Whether it
    /// ended.
    pub(super) fn end_agent_wait(&mut self, id: &str, outcome: WaitOutcome, now: u64) -> bool {
        let key = outcome.key();
        if !self.snapshot.ui_state.waits.end(id, outcome, now) {
            return false;
        }
        crate::diagnostic!(json!({
            "component": "waits", "kind": "wait.ended", "wait": id, "outcome": key,
        }));
        self.missions.outside.forget_agent(id);
        for dropped in self.snapshot.ui_state.waits.bound_ended() {
            crate::diagnostic!(json!({
                "component": "waits", "kind": "wait.notice_dropped_full",
                "wait": dropped, "cap": MACHINE_WAIT_LIMIT,
            }));
        }
        // The agent is woken by its notice, not put back to sleep by the
        // wait that ended.
        let agent = self
            .snapshot
            .ui_state
            .waits
            .waits
            .iter()
            .find(|wait| wait.id == id)
            .map(|wait| wait.agent_id.clone());
        if let Some(agent) = agent {
            self.unmark_wait_sleep(&agent, id);
        }
        self.letter_wakes.touch();
        self.persist_ui_state();
        self.wait_rows_moved();
        true
    }

    /// Draws the sleeping rows again: a wait their line reads moved.
    fn wait_rows_moved(&mut self) {
        self.refresh_agent_sleep_rows();
        self.refresh_dormant_rows();
    }

    /// Moves agent wait `id` to what a code host read found for `watch`;
    /// an answer for a wait that ended, was cancelled or names another
    /// target is dropped. Whether the store changed.
    pub(super) fn settle_agent_wait(
        &mut self,
        id: &str,
        watch: &Watch,
        found: &Found,
        now: u64,
    ) -> bool {
        let Watch::Agent { target, .. } = watch else {
            return false;
        };
        let Some(wait) = self
            .snapshot
            .ui_state
            .waits
            .waits
            .iter_mut()
            .find(|wait| wait.id == id && wait.open() && wait.target == *target)
        else {
            return false;
        };
        let mut changed = false;
        if let Found::Ahead(head) = found
            && wait.branch_ahead.as_ref() != Some(head)
        {
            wait.branch_ahead = Some(head.clone());
            changed = true;
        }
        let outcome = match found {
            Found::InMain(_) => Some(WaitOutcome::Merged),
            Found::Ended(outcome) => Some(outcome.clone()),
            _ => None,
        };
        match outcome {
            Some(outcome) => self.end_agent_wait(id, outcome, now),
            None if changed => {
                self.persist_ui_state();
                true
            }
            None => false,
        }
    }

    /// Whether `project`'s code host is near its rate limit's floor now, so
    /// a change request is not read by its number until the next look
    /// (D-13): the watch's floor, which its answer says
    /// (`code_host::WatchAnswer::resting`).
    pub(super) fn code_host_resting(&self, project: &str) -> bool {
        self.github_place(None, project)
            .ok()
            .is_some_and(|place| self.missions.hosts_resting.contains(place.host().unit().key()))
    }

    /// The change request a pull request or CI target names, as `project`'s
    /// code host writes and addresses it (its label, `#12` or `!12`, and its
    /// page), so no screen spells a number or an address itself; none for
    /// another target and while the project's repository is not known.
    pub(crate) fn wait_reference(
        &self,
        device: &str,
        project: &str,
        target: &WaitTarget,
    ) -> Option<crate::code_host::ChangeRequestRef> {
        let number = target.change_request()?;
        let machine = (device != self.node.as_str()).then_some(device);
        let place = self.github_place(machine, project).ok()?;
        // A checkout's place names its repository once a read found it.
        let repository = place.repository().cloned().or_else(|| {
            self.github_project_on(device, project)?
                .repository
                .as_ref()?
                .repository
                .clone()
        })?;
        Some(crate::code_host::change_request(
            place.host().unit(),
            repository,
            number,
        ))
    }

    /// Keeps whether `host`'s watch rests at its rate limit's floor, as its
    /// last answer said (D-13).
    pub(crate) fn note_host_resting(&mut self, host: crate::code_host::Host, resting: bool) {
        let key = host.unit().key();
        let moved = if resting {
            self.missions.hosts_resting.insert(key.to_owned())
        } else {
            self.missions.hosts_resting.remove(key)
        };
        if moved {
            crate::diagnostic!(json!({
                "component": "waits", "kind": "wait.host_resting", "host": key,
                "resting": resting,
            }));
        }
    }

    /// Open waits on this machine now, every agent's and every waiting
    /// node's still judged (D-13).
    pub(super) fn open_wait_count(&self) -> usize {
        let nodes = self
            .missions
            .book
            .missions
            .iter()
            .filter(|mission| mission.status.open())
            .flat_map(|mission| &mission.nodes)
            .filter(|node| super::mission_outside::judged(node))
            .count();
        nodes + self.snapshot.ui_state.waits.open().count()
    }

    /// Why `target` cannot be waited on from mission `from` (the caller's
    /// own mission, if any) in `project` on `device`: a mission or node this
    /// book does not hold (another machine's included, D-10), the caller's
    /// own mission or one that waits on it (D-22), or a pull request or CI
    /// the project's code host cannot read by number (D-14). `extra` are
    /// waits the same change adds, `(waiting mission, target mission)`.
    pub(super) fn wait_refusal(
        &self,
        target: &WaitTarget,
        from: Option<&str>,
        device: &str,
        project: Option<&str>,
        extra: &[(String, String)],
    ) -> Option<&'static str> {
        if let Some(id) = target.mission() {
            let book = &self.missions.book;
            let loaded = book.get(id);
            if loaded.is_none() && !book.shelved.iter().any(|line| line.id == id) {
                return Some("wait_mission_unknown");
            }
            if let (WaitTarget::Node { node, .. }, Some(mission)) = (target, loaded)
                && mission.node(node).is_none()
            {
                return Some("wait_mission_unknown");
            }
            if let Some(from) = from
                && (id == from
                    || crate::waits::waits_on(book, id, extra).contains(from))
            {
                return Some("wait_self");
            }
            return None;
        }
        if target.change_request().is_some()
            && let Some(project) = project
        {
            let machine = (device != self.node.as_str()).then_some(device);
            if self
                .github_place(machine, project)
                .is_ok_and(|place| place.host().by_number().is_none())
            {
                return Some("wait_unsupported");
            }
        }
        None
    }

    /// Runs `hide wait …` for the agent `actor` in `project` on `device`
    /// (D-21, B14, B15, B19). Returns the answer and whether anything
    /// changed.
    pub(super) fn wait_command(
        &mut self,
        device: &str,
        actor: &Actor,
        project: Option<&str>,
        command: crate::missions::MissionCommand,
        now: u64,
    ) -> Result<(Value, bool), String> {
        use crate::missions::MissionCommand;
        let agent = self
            .delivery_ledger
            .as_ref()
            .ok()
            .and_then(|ledger| {
                ledger
                    .agents
                    .iter()
                    .rev()
                    .find(|record| !record.ended && record.actor.same_identity(actor))
            })
            .map(|record| record.id.clone())
            .ok_or("wait_agent_unregistered")?;
        match command {
            MissionCommand::WaitList => {
                let waits: Vec<Value> = self
                    .snapshot
                    .ui_state
                    .waits
                    .open()
                    .filter(|wait| wait.agent_id == agent)
                    .map(wait_json)
                    .collect();
                Ok((json!({"waits": waits}), false))
            }
            MissionCommand::WaitCancel { id } => {
                let wait = self
                    .snapshot
                    .ui_state
                    .waits
                    .open()
                    .find(|wait| wait.id == id)
                    .ok_or("wait_unknown")?;
                if wait.agent_id != agent {
                    return Err("wait_not_yours".into());
                }
                self.snapshot.ui_state.waits.waits.retain(|wait| wait.id != id);
                self.missions.outside.forget_agent(&id);
                self.unmark_wait_sleep(&agent, &id);
                crate::diagnostic!(json!({
                    "component": "waits", "kind": "wait.cancelled", "wait": id,
                    "agent_id": agent,
                }));
                self.persist_ui_state();
                self.wait_rows_moved();
                Ok((json!({"cancelled": id}), true))
            }
            MissionCommand::Wait { wait } => self.start_agent_wait(device, &agent, actor, project, &wait, now),
            _ => Err("mission_text_invalid".into()),
        }
    }

    fn start_agent_wait(
        &mut self,
        device: &str,
        agent: &str,
        actor: &Actor,
        project: Option<&str>,
        input: &WaitInput,
        now: u64,
    ) -> Result<(Value, bool), String> {
        let target = input.target().map_err(|refusal| match refusal {
            InputRefusal::Target => "wait_target_invalid",
            InputRefusal::Deadline => "wait_deadline_invalid",
        })?;
        let minutes = input.minutes().map_err(|_| "wait_deadline_invalid")?;
        // Asked again, the same wait answers (engineering #11).
        if let Some(open) = self.snapshot.ui_state.waits.same(agent, &target) {
            let mut answer = wait_json(open);
            answer["existing"] = json!(true);
            return Ok((answer, false));
        }
        if target.code_host() && project.is_none() {
            return Err("wait_project_unknown".into());
        }
        let own = self
            .missions
            .book
            .missions
            .iter()
            .find(|mission| {
                mission.status.open()
                    && (mission.conductor.agent_id.as_deref() == Some(agent)
                        || mission.nodes.iter().any(|node| {
                            node.agent_id.as_deref() == Some(agent)
                                || node.checker_agents().any(|id| id == agent)
                        }))
            })
            .map(|mission| mission.id.clone());
        if let Some(code) = self.wait_refusal(&target, own.as_deref(), device, project, &[]) {
            return Err(code.into());
        }
        let held = self
            .snapshot
            .ui_state
            .waits
            .open()
            .filter(|wait| wait.agent_id == agent)
            .count();
        if held >= AGENT_WAIT_LIMIT {
            crate::diagnostic!(json!({
                "component": "waits", "kind": "wait.refused_full", "agent_id": agent,
                "cap": AGENT_WAIT_LIMIT,
            }));
            return Err("wait_agent_full".into());
        }
        if self.open_wait_count() >= MACHINE_WAIT_LIMIT {
            crate::diagnostic!(json!({
                "component": "waits", "kind": "wait.refused_full", "cap": MACHINE_WAIT_LIMIT,
            }));
            return Err("wait_full".into());
        }
        let store = &mut self.snapshot.ui_state.waits;
        let id = store.allocate();
        let wait = AgentWait {
            id: id.clone(),
            agent_id: agent.to_owned(),
            actor: actor.clone(),
            device: device.to_owned(),
            project: target.code_host().then(|| project.map(str::to_owned)).flatten(),
            target: target.clone(),
            created_at_unix_ms: now,
            until_unix_ms: crate::waits::deadline(now, minutes),
            branch_ahead: None,
            ended: None,
        };
        let answer = wait_json(&wait);
        store.waits.push(wait);
        crate::diagnostic!(json!({
            "component": "waits", "kind": "wait.started", "wait": id, "agent_id": agent,
            "target": target.words(),
        }));
        self.mark_wait_sleep(agent, &id);
        self.persist_ui_state();
        self.wait_rows_moved();
        // Judged at once: a mission already closed ends it now, and a code
        // host target is read without waiting for the next look.
        if target.mission().is_some() {
            if let Some(outcome) = crate::waits::judge_book(&self.missions.book, &target) {
                self.end_agent_wait(&id, outcome, now);
            }
        } else if let Some(project) = project {
            self.follow_outside_stored(device, project);
        }
        Ok((answer, true))
    }

    /// The open wait an asleep agent's row says it waits on (D-15, B18):
    /// its first open wait by id, and its deadline.
    pub(crate) fn sleeping_wait_of(&self, agent_id: &str) -> Option<&AgentWait> {
        self.snapshot
            .ui_state
            .waits
            .open()
            .find(|wait| wait.agent_id == agent_id)
    }

    /// The line under the row of sleeping agent `agent_id` (D-15, B18): its
    /// first open wait, with the code host's reference for a pull request
    /// or CI target.
    pub(crate) fn wait_line_of(&self, agent_id: &str) -> Option<crate::waits::WaitLine> {
        let wait = self.sleeping_wait_of(agent_id)?;
        Some(crate::waits::WaitLine {
            target: wait.target.clone(),
            until_unix_ms: wait.until_unix_ms,
            reference: wait
                .project
                .as_deref()
                .and_then(|project| self.wait_reference(&wait.device, project, &wait.target)),
        })
    }

    /// Lays the wait line on each sleeping row: the open wait of the live
    /// registration in its pane.
    pub(super) fn lay_wait_lines(&self, agents: &mut [crate::model::SidebarAgentSnapshot]) {
        if self.snapshot.ui_state.waits.open().next().is_none() {
            return;
        }
        let Ok(ledger) = self.delivery_ledger.as_ref() else {
            return;
        };
        for agent in agents {
            let Some(sleep) = agent.sleep.as_mut() else {
                continue;
            };
            sleep.wait = ledger
                .agents
                .iter()
                .rev()
                .find(|record| {
                    !record.ended && record.actor.pane_id == agent.pane_id
                })
                .and_then(|record| self.wait_line_of(&record.id));
        }
    }

    /// Asks agent `agent` to sleep at its next rest for wait `id` (D-09):
    /// never an open mission's Conductor, which waits through its waiting
    /// nodes, as a report never marks one.
    fn mark_wait_sleep(&mut self, agent: &str, id: &str) {
        let conducts = self.missions.book.missions.iter().any(|mission| {
            mission.status.open() && mission.conductor.agent_id.as_deref() == Some(agent)
        });
        if conducts {
            return;
        }
        let origin = super::agent_sleep::missions::Origin::Wait {
            wait: id.to_owned(),
        };
        if super::agent_sleep::missions::SleepMark::ask(
            &mut self.missions.sleep_marks,
            agent,
            origin,
        ) {
            crate::diagnostic!(json!({
                "component": "agent_sleep", "kind": "agent_sleep.wait_marked",
                "agent_id": agent, "wait": id,
            }));
        }
    }

    /// Drops the sleep mark wait `id` gave agent `agent`: the wait ended or
    /// was cancelled.
    fn unmark_wait_sleep(&mut self, agent: &str, id: &str) {
        self.missions.sleep_marks.retain(|mark| {
            !(mark.agent_id == agent
                && matches!(&mark.origin, super::agent_sleep::missions::Origin::Wait { wait } if wait == id))
        });
    }

    /// Marks again, after a restart, each agent with an open wait (the marks
    /// are memory only).
    pub(super) fn mark_waiting_agents(&mut self) {
        let open: Vec<(String, String)> = self
            .snapshot
            .ui_state
            .waits
            .open()
            .map(|wait| (wait.agent_id.clone(), wait.id.clone()))
            .collect();
        for (agent, id) in open {
            self.mark_wait_sleep(&agent, &id);
        }
    }

    /// The notices of agents' ended waits, one news cell each for its
    /// agent (B17): its first line the target and the outcome.
    pub(super) fn wait_cells(
        &self,
        ledger: Option<&crate::delivery::ledger::Ledger>,
    ) -> Vec<super::status_cells::Addressed> {
        self.snapshot
            .ui_state
            .waits
            .waits
            .iter()
            .filter_map(|wait| {
                let ended = wait.ended.as_ref()?;
                let actor = ledger
                    .and_then(|ledger| {
                        ledger
                            .agents
                            .iter()
                            .find(|record| record.id == wait.agent_id && !record.ended)
                    })
                    .map_or_else(|| wait.actor.clone(), |record| record.actor.clone());
                Some(super::status_cells::Addressed {
                    recipient: wait.agent_id.clone(),
                    actor: Some(actor),
                    cell: super::status_cells::Cell {
                        subject: wait_subject(&wait.id),
                        line: format!(
                            "{} ({}).\nYour wait ended; `hide wait list` shows any others.",
                            ended.outcome.sentence(&wait.target),
                            wait.id
                        ),
                        news: true,
                        revision: ended.seq,
                        episode: ended.seq,
                        mid_turn: false,
                    },
                })
            })
            .collect()
    }

    /// Lets go of the ended waits whose notice `recipient` was just given.
    pub(super) fn drop_seen_wait_notices(&mut self, recipient: &str, subjects: &[&str]) {
        let before = self.snapshot.ui_state.waits.waits.len();
        self.snapshot.ui_state.waits.waits.retain(|wait| {
            wait.open()
                || wait.agent_id != recipient
                || !subjects.contains(&wait_subject(&wait.id).as_str())
        });
        if self.snapshot.ui_state.waits.waits.len() != before {
            self.persist_ui_state();
        }
    }

    /// The agents with an ended wait's notice they have not seen, each with
    /// the newest such notice's revision: a notice wakes its sleeping agent
    /// as a worker's cell does (`worker_cells_unseen`).
    pub(super) fn wait_notices_unseen(
        &self,
        seen: &std::collections::BTreeMap<String, std::collections::BTreeMap<String, u64>>,
    ) -> Vec<(String, Actor, u64)> {
        let ledger = self.delivery_ledger.as_ref().ok();
        let mut newest: std::collections::BTreeMap<String, (Actor, u64)> =
            std::collections::BTreeMap::new();
        for wait in &self.snapshot.ui_state.waits.waits {
            let Some(ended) = wait.ended.as_ref() else {
                continue;
            };
            let looked = seen
                .get(&wait.agent_id)
                .and_then(|seen| seen.get(&wait_subject(&wait.id)))
                .copied()
                .unwrap_or(0);
            if ended.seq <= looked {
                continue;
            }
            let Some(actor) = ledger.and_then(|ledger| {
                ledger
                    .agents
                    .iter()
                    .find(|record| record.id == wait.agent_id && !record.ended)
                    .map(|record| record.actor.clone())
            }) else {
                continue;
            };
            let entry = newest
                .entry(wait.agent_id.clone())
                .or_insert((actor, 0));
            entry.1 = entry.1.max(ended.seq);
        }
        newest
            .into_iter()
            .map(|(agent, (actor, revision))| (agent, actor, revision))
            .collect()
    }
}
