//! Typed input awaiting the agent's own record (`delivery::typed`): which
//! briefs and bells are unconfirmed, who is told, what is typed again, and
//! which rows show it to the operator. Everything here reads what the
//! runtime already holds, the published ledger and the delivery
//! observations, under its lock and without I/O; the doorbell's thread
//! sends the notices and types the input off the lock.

use std::collections::BTreeMap;

use crate::coordination::{Mutation, SpawnMode, SpawnRecord};
use crate::delivery::typed::{self, InputKind, Standing, TypedInput};
use crate::delivery::worker::Effect;
use crate::model::AskSubject;

use super::Runtime;
use super::delivery::Observation;

/// The most panes that wait at once for their session to be read again
/// (`Runtime::queue_session_reread`): more than one machine's open agents.
pub(crate) const SESSION_REREADS_LIMIT: usize = 256;

/// Which typed input a piece of work is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum InputKey {
    Spawn(String),
    Letter(String),
}

/// A notice owed to the agent that typed the input, or wrote the letter.
#[derive(Clone)]
pub(crate) struct Notice {
    pub target: Observation,
    pub intent: String,
    pub body: String,
    pub attempt: u8,
}

/// One unconfirmed input the doorbell's thread has work for.
#[derive(Clone)]
pub(crate) struct InputWork {
    pub key: InputKey,
    /// The pane the input was typed into, as Hide observes it now.
    pub observation: Observation,
    pub notice: Option<Notice>,
    /// A brief whose agent's composer is to be read: still showing the
    /// brief, it gets its Enter; empty, the brief is typed again once
    /// someone answered the screen.
    pub resubmit: Option<Resubmit>,
    /// The agent recorded the brief (`"session"`: among its session's
    /// person messages; `"turn"`: by starting a turn after its typing,
    /// `AgentId::turn_takes_brief`): the store is told, and nothing else is
    /// done for it.
    pub recorded: Option<&'static str>,
}

/// A brief the doorbell reads the composer for (`InputWork::resubmit`).
#[derive(Clone)]
pub(crate) struct Resubmit {
    pub brief: String,
    /// The typing the brief is at.
    pub attempts: u8,
    /// Typings are left, so an Enter or a typing may be spent.
    pub left: bool,
    /// Someone answered the agent's screen since the last typing and the
    /// agent's record has a source, so an empty composer is typed into.
    pub answered: bool,
}

/// Where an unconfirmed input stands with the fresh session read asked
/// before its verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reread {
    /// Asked and given its time, or not needed: judge the input.
    Settled,
    /// Not asked for this typing yet.
    Ask,
    /// Asked, still within its time.
    Wait,
}

/// An input shown to the operator: the pane whose row shows it, since when,
/// and what it says.
pub(crate) type InputAsk = (String, u64, AskSubject);

impl Runtime {
    /// The pane a spawn's agent was started in, as the delivery observations
    /// key it: bare on this machine, in full on a device.
    pub(super) fn spawn_pane_id(&self, spawn: &SpawnRecord) -> Option<String> {
        let pane = spawn.pane.as_deref()?;
        let node = self.node.as_str();
        Some(crate::address::pane_id_on(
            node,
            spawn.machine.as_deref().unwrap_or(node),
            pane,
        ))
    }

    /// The observation of the pane a spawn's agent runs in, while it still
    /// runs the terminal the spawn recorded (a pane id can name another pane
    /// later).
    pub(super) fn spawn_observation(&self, spawn: &SpawnRecord) -> Option<&Observation> {
        let observation = self
            .delivery_observations
            .get(&self.spawn_pane_id(spawn)?)?;
        match (&spawn.terminal, &observation.terminal_id) {
            (Some(recorded), Some(current)) if recorded != current => None,
            _ => Some(observation),
        }
    }

    /// A prompt hook in `pane_id` reported the digest of the prompt its agent
    /// took. When that is the brief a spawn typed there and is still waiting
    /// for, the brief is confirmed. It is judged before the caller's native
    /// identity is required, because Codex reports its session on that very
    /// turn and Herdr may not show it yet. Queued, never waited for: this
    /// runs under the runtime lock.
    pub(super) fn confirm_brief_by_hook(&self, pane_id: &str, digest: &str) {
        let Ok(ledger) = self.delivery_ledger.as_ref() else {
            return;
        };
        let Some(spawn) = ledger.spawns.iter().find(|spawn| {
            spawn
                .brief_input
                .as_ref()
                .is_some_and(|input| input.confirmed_at_unix_ms.is_none())
                && self.spawn_pane_id(spawn).as_deref() == Some(pane_id)
                && self.spawn_observation(spawn).is_some()
                && spawn.brief.as_deref().is_some_and(|brief| {
                    hide_agent_hooks::delivery::matches_prompt_digest(brief, digest)
                })
        }) else {
            return;
        };
        self.post_brief_confirmed(&spawn.id, "hook");
    }

    /// Queues the store's record that the brief of spawn `id` was taken.
    pub(crate) fn post_brief_confirmed(&self, id: &str, by: &'static str) {
        let posted =
            self.hide_delivery(typed::INPUT_ROLE)
                .and_then(|(client, authority, actor)| {
                    client.post(Effect::Agents {
                        authority,
                        actor,
                        mutation: Box::new(Mutation::BriefConfirmed { id: id.to_owned() }),
                    })
                });
        crate::diagnostic!(serde_json::json!({
            "component": "delivery", "kind": "input.confirmed", "spawn": id, "by": by,
            "queued": posted.is_ok(), "code": posted.err(),
        }));
    }

    /// The unconfirmed inputs the doorbell has work for now: a notice owed,
    /// or a brief to type again. Bounded by the spawns and open letters the
    /// ledger keeps.
    pub(crate) fn typed_input_work(&mut self, now: u64) -> Vec<InputWork> {
        let Ok(ledger) = self.delivery_ledger.clone() else {
            return Vec::new();
        };
        let ledger = ledger.as_ref();
        let mut work = Vec::new();
        let mut rereads = Vec::new();
        let mut open = std::collections::BTreeSet::new();
        let mut verdicts = BTreeMap::new();
        let mut turn_started = Vec::new();
        let unconfirmed: BTreeMap<&str, u8> = ledger
            .spawns
            .iter()
            .filter_map(|spawn| {
                let input = spawn.brief_input.as_ref()?;
                (input.confirmed_at_unix_ms.is_none() && input.attempts > 0)
                    .then_some((spawn.id.as_str(), input.attempts))
            })
            .collect();
        self.plans
            .input_turn_started
            .retain(|id, attempts| unconfirmed.get(id.as_str()) == Some(attempts));
        for spawn in &ledger.spawns {
            let (Some(input), Some(brief)) = (
                spawn
                    .brief_input
                    .as_ref()
                    .filter(|input| input.confirmed_at_unix_ms.is_none() && input.attempts > 0),
                spawn.brief.as_ref(),
            ) else {
                continue;
            };
            let Some(observation) = self.spawn_observation(spawn) else {
                continue;
            };
            // An agent whose prompt hook reports no digest records the brief
            // in its session file, which the label worker reads on each of
            // its state changes.
            // A message that ends with the brief is its record too: what the
            // composer held before the typing stays in front of it. The row
            // keeps the newest request's words, cut past 8 KiB. Only a
            // message written after the typing, which Hide did not see the
            // operator submit, is: "don't fix tests" typed after a lost "fix
            // tests" is the operator's own.
            if self
                .row_facts_of(&observation.actor.pane_id)
                .is_some_and(|facts| {
                    facts.prompt_digests.iter().any(|digest| {
                        hide_agent_hooks::delivery::matches_prompt_digest(brief, digest)
                    }) || facts
                        .operator_request
                        .iter()
                        .chain(&facts.other_request)
                        .any(|request| {
                            !request.cut
                                && request.requester != crate::labels::facts::Requester::Operator
                                && request
                                    .at_unix_ms
                                    .saturating_add(crate::labels::facts::CLOCK_SLACK_MS)
                                    >= input.first_typed_at()
                                && hide_agent_hooks::delivery::ends_with_typed(&request.text, brief)
                        })
                })
            {
                work.push(InputWork {
                    key: InputKey::Spawn(spawn.id.clone()),
                    observation: observation.clone(),
                    notice: None,
                    resubmit: None,
                    recorded: Some("session"),
                });
                continue;
            }
            // An agent whose session records the brief only at its turn's end
            // took it once the core's turn for it started after the typing.
            if observation.projected_turn().is_some_and(|projected| {
                projected.turn.activity == crate::agent_state::AgentActivity::Working
                    && projected.working_since(observation) >= input.typed_at_unix_ms
            }) && hide_agent_adapter::adapter(&spawn.kind)
                .is_some_and(|row| row.key.turn_takes_brief())
            {
                work.push(InputWork {
                    key: InputKey::Spawn(spawn.id.clone()),
                    observation: observation.clone(),
                    notice: None,
                    resubmit: None,
                    recorded: Some("turn"),
                });
                continue;
            }
            // Any other agent took the brief once its hook reported a turn
            // started after the typing that nothing else could have
            // started; the core remembers that typing as taken, so the turn
            // ending first changes nothing.
            if self.plans.input_turn_started.get(&spawn.id) == Some(&input.attempts)
                || turn_took_brief(ledger, observation, input)
            {
                turn_started.push((spawn.id.clone(), input.attempts));
                work.push(InputWork {
                    key: InputKey::Spawn(spawn.id.clone()),
                    observation: observation.clone(),
                    notice: None,
                    resubmit: None,
                    recorded: Some("turn"),
                });
                continue;
            }
            let standing = typed::standing(input, Some(observation), now);
            self.note_verdict(&mut verdicts, &spawn.id, input.attempts, standing);
            match standing {
                Standing::Unconfirmed { .. } => {}
                // Nothing moves until the turn is laid: the reread asked and
                // the composer read seen stay as they are.
                Standing::Unsettled => {
                    open.insert(spawn.id.clone());
                    continue;
                }
                Standing::Confirmed | Standing::Waiting => continue,
            }
            open.insert(spawn.id.clone());
            match self.reread(&spawn.id, input.attempts, &observation.actor.pane_id, now) {
                Reread::Settled => {}
                Reread::Ask => {
                    rereads.push((
                        spawn.id.clone(),
                        input.attempts,
                        observation.actor.pane_id.clone(),
                    ));
                    continue;
                }
                Reread::Wait => continue,
            }
            let source = self.input_source(&spawn.kind, &observation.actor.pane_id);
            let notice = typed::notice_due(input)
                .then(|| self.notice_to(&spawn.parent, ledger))
                .flatten()
                .map(|target| Notice {
                    intent: format!("input-unconfirmed:{}:{}", spawn.id, input.attempts),
                    body: typed::notice_body(
                        InputKind::Brief,
                        &spawn.name,
                        &observation.actor.pane_id,
                        None,
                        source,
                        input.route,
                    ),
                    attempt: input.attempts,
                    target,
                });
            // The composer is read at rest only: what a working agent draws
            // there is its own.
            // A launch prompt is held by the agent's start and never typed
            // again, so answering its screen cannot send it twice.
            let resubmit = (spawn.briefed
                && input.route.sends_again()
                && typed::composer_readable(observation, now))
            .then(|| Resubmit {
                brief: brief.clone(),
                attempts: input.attempts,
                left: input.attempts < typed::INPUT_ATTEMPTS,
                answered: source.is_ok() && typed::retype_due(input, observation, now),
            });
            if notice.is_some() || resubmit.is_some() {
                work.push(InputWork {
                    key: InputKey::Spawn(spawn.id.clone()),
                    observation: observation.clone(),
                    notice,
                    resubmit,
                    recorded: None,
                });
            }
        }
        for letter in &ledger.letters {
            let Some(observation) = self
                .delivery_observations
                .get(&letter.recipient.pane_id)
                .filter(|observation| observation.actor.same_identity(&letter.recipient))
            else {
                continue;
            };
            let Some(input) = letter.typed_input() else {
                continue;
            };
            if self.bell_line_recorded(letter) {
                continue;
            }
            let standing = typed::standing(&input, Some(observation), now);
            self.note_verdict(&mut verdicts, &letter.id, input.attempts, standing);
            match standing {
                Standing::Unconfirmed { .. } => {}
                Standing::Unsettled => {
                    open.insert(letter.id.clone());
                    continue;
                }
                Standing::Confirmed | Standing::Waiting => continue,
            }
            // Open past its notice too: the row reads the settled fresh read.
            open.insert(letter.id.clone());
            match self.reread(&letter.id, input.attempts, &observation.actor.pane_id, now) {
                Reread::Settled => {}
                Reread::Ask => {
                    rereads.push((
                        letter.id.clone(),
                        input.attempts,
                        observation.actor.pane_id.clone(),
                    ));
                    continue;
                }
                Reread::Wait => continue,
            }
            if !typed::notice_due(&input) {
                continue;
            }
            let Some(target) = self.observed_sender(&letter.sender) else {
                continue;
            };
            work.push(InputWork {
                key: InputKey::Letter(letter.id.clone()),
                observation: observation.clone(),
                notice: Some(Notice {
                    intent: format!("input-unconfirmed:{}:{}", letter.id, input.attempts),
                    body: typed::notice_body(
                        InputKind::Letter,
                        &letter.recipient.name,
                        &observation.actor.pane_id,
                        Some(&letter.id),
                        self.input_source(&letter.recipient.kind, &observation.actor.pane_id),
                        input.route,
                    ),
                    attempt: input.attempts,
                    target,
                }),
                resubmit: None,
                recorded: None,
            });
        }
        self.plans.input_verdicts = verdicts;
        for (id, attempts) in turn_started {
            if self.plans.input_turn_started.insert(id.clone(), attempts) != Some(attempts) {
                crate::diagnostic!(serde_json::json!({
                    "component": "delivery", "kind": "input.turn_started", "spawn": id,
                    "attempt": attempts,
                }));
            }
        }
        self.plans.input_reread.retain(|id, _| open.contains(id));
        self.plans.input_held.retain(|id, _| open.contains(id));
        for (id, attempts, pane) in rereads {
            self.plans.input_reread.insert(id, (attempts, now));
            self.queue_session_reread(pane);
        }
        work
    }

    /// Keeps input `id`'s verdict at typing `attempts` for the next pass: a
    /// settled one replaces the last, and an unsettled one carries the last
    /// over.
    fn note_verdict(
        &self,
        verdicts: &mut BTreeMap<String, (u8, Standing)>,
        id: &str,
        attempts: u8,
        standing: Standing,
    ) {
        let kept = match standing {
            Standing::Unsettled => self
                .plans
                .input_verdicts
                .get(id)
                .filter(|(typing, _)| *typing == attempts)
                .map(|(_, verdict)| *verdict),
            settled => Some(settled),
        };
        if let Some(verdict) = kept {
            verdicts.insert(id.to_owned(), (attempts, verdict));
        }
    }

    /// Where input `id` stands for the pane as observed now, and while the
    /// core has no turn for the pane, where the doorbell's last pass found
    /// this same typing (`plans.input_verdicts`), so the row and `hide agent
    /// show` keep saying what they said. Unsettled only when no pass judged
    /// this typing yet; confirmed once the agent took it by a turn.
    pub(super) fn input_standing(
        &self,
        id: &str,
        input: &TypedInput,
        observation: Option<&Observation>,
        now: u64,
    ) -> Standing {
        // A brief its agent took by a turn is confirmed by that turn, ahead
        // of the store's record of it (`plans.input_turn_started`).
        if self.plans.input_turn_started.get(id) == Some(&input.attempts) {
            return Standing::Confirmed;
        }
        match typed::standing(input, observation, now) {
            Standing::Unsettled => self
                .plans
                .input_verdicts
                .get(id)
                .filter(|(typing, _)| *typing == input.attempts)
                .map_or(Standing::Unsettled, |(_, verdict)| *verdict),
            standing => standing,
        }
    }

    /// Where unconfirmed input `id`, at typing `attempts`, stands with the
    /// fresh read of its agent's session that comes before the verdict
    /// (`typed::REREAD_WAIT_MS`). A device's pane is read on its own machine
    /// on its state changes alone, so it is judged at once.
    fn reread(&self, id: &str, attempts: u8, pane_id: &str, now: u64) -> Reread {
        if crate::address::is_device_scoped(pane_id) {
            return Reread::Settled;
        }
        match self.plans.input_reread.get(id) {
            Some((asked, at)) if *asked == attempts => {
                if now.saturating_sub(*at) >= typed::REREAD_WAIT_MS {
                    Reread::Settled
                } else {
                    Reread::Wait
                }
            }
            _ => Reread::Ask,
        }
    }

    /// The panes whose sessions are to be read again, for this machine's
    /// session-sync thread.
    pub(crate) fn take_session_rereads(&mut self) -> Vec<String> {
        self.plans.session_rereads_refused = false;
        std::mem::take(&mut self.plans.session_rereads)
    }

    /// Asks this machine's label reader to read `pane`'s session again on
    /// its next tick. A pane already waiting waits once. A pane not yet
    /// waiting first drops the waiting panes the core no longer projects (a
    /// machine with no layout yet keeps them all), and past
    /// `SESSION_REREADS_LIMIT` it is refused and the refusal logged once
    /// until the reader takes the list: nothing takes it on a machine whose
    /// reader never started, and it holds no more than the limit.
    pub(crate) fn queue_session_reread(&mut self, pane: String) {
        if self.plans.session_rereads.contains(&pane) {
            return;
        }
        let mut queued = std::mem::take(&mut self.plans.session_rereads);
        queued.retain(|waiting| self.pane_still_terminal(waiting));
        self.plans.session_rereads = queued;
        if self.plans.session_rereads.len() >= SESSION_REREADS_LIMIT {
            if !std::mem::replace(&mut self.plans.session_rereads_refused, true) {
                crate::diagnostic!(serde_json::json!({
                    "component": "labels", "kind": "session_reread.capacity",
                    "pane": pane, "limit": SESSION_REREADS_LIMIT,
                }));
            }
            return;
        }
        self.plans.session_rereads.push(pane);
    }

    /// Whether the agent in `pane_id` has a source for its own record of
    /// typed input: a prompt hook Hide hears, for an agent that records
    /// there, else a read of its session file. Without one an unconfirmed
    /// input is not judged lost (`typed::HOOKS_NOT_CONNECTED`,
    /// `typed::SESSION_UNREAD`).
    fn input_source(&self, kind: &str, pane_id: &str) -> Result<(), &'static str> {
        let hooked =
            hide_agent_adapter::adapter(kind).is_some_and(|row| row.key.prompt_hook_records());
        if hooked {
            let not_connected = self
                .pane_children_of(pane_id)
                .and_then(|children| children.connection.as_ref())
                .is_some_and(|connection| !connection.connected);
            return if not_connected {
                Err(typed::HOOKS_NOT_CONNECTED)
            } else {
                Ok(())
            };
        }
        self.row_facts_of(pane_id)
            .map(drop)
            .ok_or(typed::SESSION_UNREAD)
    }

    /// The pane's children projection, which carries its Not connected
    /// judgement, on this machine or a device.
    fn pane_children_of(&self, pane_id: &str) -> Option<&crate::model::PaneChildrenSnapshot> {
        self.snapshot
            .navigator
            .workspaces
            .iter()
            .chain(
                self.snapshot
                    .status
                    .remote
                    .iter()
                    .filter_map(|remote| remote.session.as_ref())
                    .flat_map(|session| session.workspaces.iter()),
            )
            .flat_map(|workspace| workspace.checkouts.iter())
            .flat_map(|checkout| checkout.tabs.iter())
            .flat_map(|tab| tab.panes.iter())
            .find(|pane| pane.id == pane_id)
            .and_then(|pane| pane.children.as_ref())
    }

    /// The doorbell read the composer of input `id` after its typing
    /// `attempts`: `held` when it still showed the input.
    pub(crate) fn note_input_held(&mut self, id: &str, attempts: u8, held: bool) {
        if held {
            self.plans.input_held.insert(id.to_owned(), attempts);
        } else {
            self.plans.input_held.remove(id);
        }
    }

    fn input_held(&self, id: &str, attempts: u8) -> bool {
        self.plans.input_held.get(id) == Some(&attempts)
    }

    /// The label worker's facts on the row of `pane_id`, on this machine or
    /// a device, laid only while the pane's session proves them.
    /// Whether the recipient's session file records `letter`'s last bell line
    /// in a person message written after that bell was typed. It proves the
    /// line was submitted, not that the letter's body was shown: the bell
    /// owes no "not taken" notice and no ask, and takes nothing in, so the
    /// next bell, whose prompt hook prints the body, still rings. An older
    /// message with the same line, such as an earlier letter's from the same
    /// sender, is not this bell's.
    fn bell_line_recorded(&self, letter: &crate::delivery::ledger::Letter) -> bool {
        let (Some(line), Some(typed_at)) =
            (letter.bell_line.as_deref(), letter.bell_typed_at_unix_ms)
        else {
            return false;
        };
        let digest = hide_agent_hooks::delivery::prompt_digest(line);
        self.row_facts_of(&letter.recipient.pane_id)
            .is_some_and(|facts| {
                facts
                    .operator_request
                    .iter()
                    .chain(&facts.other_request)
                    .any(|request| {
                        !request.cut
                            && request
                                .at_unix_ms
                                .saturating_add(crate::labels::facts::CLOCK_SLACK_MS)
                                >= typed_at
                            && hide_agent_hooks::delivery::prompt_digest(&request.text) == digest
                    })
            })
    }

    fn row_facts_of(&self, pane_id: &str) -> Option<&crate::request_view::RowFacts> {
        self.snapshot
            .navigator
            .agents
            .iter()
            .chain(
                self.snapshot
                    .status
                    .remote
                    .iter()
                    .filter_map(|remote| remote.session.as_ref())
                    .flat_map(|session| session.agents.iter()),
            )
            .find(|agent| agent.pane_id == pane_id)
            .and_then(|agent| agent.row_facts.as_ref())
    }

    /// The observed pane of the registration `parent` names, the agent told
    /// of its child's unconfirmed brief.
    fn notice_to(
        &self,
        parent: &str,
        ledger: &crate::delivery::ledger::Ledger,
    ) -> Option<Observation> {
        let actor = crate::coordination::resolve_actor(ledger, parent)?;
        self.observed_sender(actor)
    }

    /// The pane `actor` runs in while it still runs there; a code-owned
    /// identity, which has no pane, is told nothing.
    fn observed_sender(&self, actor: &crate::delivery::Actor) -> Option<Observation> {
        if actor.code_owned() {
            return None;
        }
        self.delivery_observations
            .get(&actor.pane_id)
            .filter(|observation| observation.actor.same_identity(actor))
            .cloned()
    }

    /// The rows that show unconfirmed input to the operator, by pane: once
    /// no parent can clear it, its parent had its grace, or no typing is
    /// left (`typed::shown_to_operator`).
    pub(crate) fn input_asks(&self, now: u64) -> Vec<InputAsk> {
        let Ok(ledger) = self.delivery_ledger.as_ref() else {
            return Vec::new();
        };
        let mut asks = BTreeMap::new();
        for spawn in &ledger.spawns {
            let Some(input) = spawn
                .brief_input
                .as_ref()
                .filter(|input| input.confirmed_at_unix_ms.is_none())
            else {
                continue;
            };
            let Some(observation) = self.spawn_observation(spawn) else {
                continue;
            };
            let Standing::Unconfirmed { since } =
                self.input_standing(&spawn.id, input, Some(observation), now)
            else {
                continue;
            };
            if self.reread(&spawn.id, input.attempts, &observation.actor.pane_id, now)
                != Reread::Settled
            {
                continue;
            }
            let has_parent = spawn.mode == SpawnMode::Delegation
                && self.notice_to(&spawn.parent, ledger).is_some();
            if typed::shown_to_operator(input, has_parent, now) {
                let held = self.input_held(&spawn.id, input.attempts);
                asks.entry(observation.actor.pane_id.clone())
                    .or_insert((since, subject(InputKind::Brief, input, held)));
            }
        }
        for letter in &ledger.letters {
            let Some(input) = letter.typed_input() else {
                continue;
            };
            let Some(observation) = self
                .delivery_observations
                .get(&letter.recipient.pane_id)
                .filter(|observation| observation.actor.same_identity(&letter.recipient))
            else {
                continue;
            };
            if self.bell_line_recorded(letter) {
                continue;
            }
            let Standing::Unconfirmed { since } =
                self.input_standing(&letter.id, &input, Some(observation), now)
            else {
                continue;
            };
            if self.reread(&letter.id, input.attempts, &observation.actor.pane_id, now)
                != Reread::Settled
            {
                continue;
            }
            let has_sender = self.observed_sender(&letter.sender).is_some();
            if typed::shown_to_operator(&input, has_sender, now) {
                asks.entry(observation.actor.pane_id.clone())
                    .or_insert((since, subject(InputKind::Letter, &input, false)));
            }
        }
        asks.into_iter()
            .map(|(pane, (since, subject))| (pane, since, subject))
            .collect()
    }
}

impl Runtime {
    /// Keeps the rows that show unconfirmed input current, on each doorbell
    /// pass: a change relays the core's questions on the rows shown.
    /// Whether any row moved.
    pub(super) fn publish_input_asks(&mut self, now: u64) -> bool {
        let asks = self.input_asks(now);
        if asks == self.plans.input_asks {
            return false;
        }
        for (pane, _, subject) in &asks {
            if !self
                .plans
                .input_asks
                .iter()
                .any(|(shown, ..)| shown == pane)
            {
                crate::diagnostic!(serde_json::json!({"component":"delivery",
                    "kind":"input.shown","pane_id":pane,"subject":subject}));
            }
        }
        self.plans.input_asks = asks;
        self.plans.input_edit += 1;
        self.refresh_plan_reviews()
    }
}

/// Whether the agent took the brief's latest typing by starting a turn: its
/// own hook reports the core's turn working (Herdr's reading alone never
/// counts) since after that typing, and nothing else could have started it,
/// since no input reached the pane after the typing (the brief's own
/// `agent.prompt` is no noted input) and no letter's bell was typed there
/// since. A turn this misses (one shorter than a pass, input during it, a
/// restart, an agent whose hook reports no turn) leaves the brief judged by
/// its record alone.
fn turn_took_brief(
    ledger: &crate::delivery::ledger::Ledger,
    observation: &Observation,
    input: &TypedInput,
) -> bool {
    let typed_at = input.typed_at_unix_ms;
    let started = observation.projected_turn().is_some_and(|projected| {
        projected.turn.activity == crate::agent_state::AgentActivity::Working
            && projected.turn.reported
            && projected.working_since(observation) >= typed_at
    });
    let pane = &observation.actor.pane_id;
    started
        && observation.last_input_at_unix_ms <= typed_at
        && !ledger.letters.iter().any(|letter| {
            &letter.recipient.pane_id == pane
                && letter
                    .bell_typed_at_unix_ms
                    .is_some_and(|at| at >= typed_at)
        })
}

fn subject(input: InputKind, typed_input: &TypedInput, held: bool) -> AskSubject {
    AskSubject::InputUnconfirmed {
        input,
        spent: typed_input.attempts >= typed::INPUT_ATTEMPTS,
        held,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::coordination::{AgentRecord, Command};
    use crate::delivery::Actor;
    use crate::delivery::ledger::Ledger;
    use crate::delivery::typed::{PARENT_GRACE_MS, RECORD_WAIT_MS, REREAD_WAIT_MS};
    use crate::node::TEST_NODE;
    use hide_node_link::panes::HookTurn;

    pub(crate) const BRIEF: &str = "Node api: add the call.\nReport by letter.";

    /// A parent in pane `sender` that spawned a Codex into pane `child` with a
    /// brief, typed once at `typed_at`; the Codex has no session yet, as
    /// before its first turn. Returns the runtime and the spawn's id.
    pub(crate) fn spawned(handoff: bool, typed_at: u64) -> (tempfile::TempDir, Runtime, String) {
        spawned_as("codex", handoff, typed_at)
    }

    /// [`spawned`] with a child of `kind`.
    fn spawned_as(
        kind: &str,
        handoff: bool,
        typed_at: u64,
    ) -> (tempfile::TempDir, Runtime, String) {
        spawned_with(kind, BRIEF, handoff, typed_at, false)
    }

    /// [`spawned_as`] with `brief` typed, or with `launched`, carried by the
    /// start's prompt argument.
    fn spawned_with(
        kind: &str,
        brief: &str,
        handoff: bool,
        typed_at: u64,
        launched: bool,
    ) -> (tempfile::TempDir, Runtime, String) {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, _) = crate::runtime::delivery::tests::fixture(root.path());
        let mut runtime = Arc::try_unwrap(runtime).ok().unwrap().into_inner().unwrap();
        let parent_session = crate::engine::tokens::session_digest("parent-native");
        let payload: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({
        "panes": [
            {"pane_id": "sender", "terminal_id": "term-sender"},
            {"pane_id": "child", "terminal_id": "term-child"},
        ],
        "agents": [
            {"id": "sender", "pane_id": "sender", "agent": "codex", "agent_status": "working",
             "state_change_seq": 1, "lineage_session": parent_session},
            {"id": "child", "pane_id": "child", "agent": kind, "agent_status": "idle",
             "state_change_seq": 2},
        ]}))
        .unwrap();
        runtime.observe_delivery_laid(TEST_NODE, &payload, None, None);
        let child = runtime.delivery_observations.get_mut("child").unwrap();
        child.status_changed_at_unix_ms = 0;
        child.last_input_at_unix_ms = 0;
        let parent = Actor {
            pane_id: "sender".into(),
            name: "sender".into(),
            kind: "codex".into(),
            device_id: TEST_NODE.into(),
            session: parent_session,
        };
        let mut ledger = Ledger::default();
        let apply = |ledger: &mut Ledger, mutation: Mutation, now: u64| {
            crate::coordination::apply(ledger, &parent, &mutation, now).unwrap()
        };
        let parent_id = apply(
            &mut ledger,
            Mutation::Register {
                record: AgentRecord {
                    id: String::new(),
                    name: Some("lead".into()),
                    machine: TEST_NODE.into(),
                    host_scope: "fixture-scope".into(),
                    native_machine: "fixture-machine".into(),
                    session: "parent-native".into(),
                    instance: "term-sender".into(),
                    pane: "sender".into(),
                    parent: None,
                    origin: None,
                    project: None,
                    actor: parent.clone(),
                    ended: false,
                    terminal_id: None,
                    forked: false,
                },
                check: false,
            },
            1,
        )["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let spawn = apply(
            &mut ledger,
            Mutation::Reserve {
                parent: parent_id.clone(),
                command: Command::Spawn {
                    parent: (!handoff).then(|| parent_id.clone()),
                    machine: None,
                    name: "worker".into(),
                    intent: "task-1".into(),
                    kind: kind.into(),
                    repo: "/fixture".into(),
                    branch: "topic".into(),
                    path: None,
                    args: Vec::new(),
                    launch: Default::default(),
                    task: None,
                    brief: Some(brief.into()),
                    beside: false,
                    mission: None,
                    plan_node: None,
                    plan: None,
                },
            },
            1,
        )["id"]
            .as_str()
            .unwrap()
            .to_owned();
        for mutation in [
            Mutation::Advance {
                seed_tab: None,
                id: spawn.clone(),
                path: Some("/fixture/topic".into()),
                pane: Some("child".into()),
                terminal: Some("term-child".into()),
                shell: None,
                child: None,
            },
            Mutation::Typed { id: spawn.clone() },
        ] {
            apply(&mut ledger, mutation, 1);
        }
        if launched {
            apply(
                &mut ledger,
                Mutation::BriefLaunched {
                    id: spawn.clone(),
                    words: vec!["--".into(), "brief".into()],
                },
                typed_at,
            );
        } else {
            apply(
                &mut ledger,
                Mutation::BriefTyping { id: spawn.clone() },
                typed_at,
            );
            apply(
                &mut ledger,
                Mutation::Briefed {
                    id: spawn.clone(),
                    route: Default::default(),
                },
                typed_at,
            );
        }
        runtime.publish_delivery(Arc::new(ledger), false);
        (root, runtime, spawn)
    }

    /// A brief that rode the start's prompt argument is never typed: while
    /// its child sits unrecorded at a start screen its parent hears that
    /// the start holds it, and answering that screen has nothing typed.
    #[test]
    fn a_launched_brief_is_never_typed_and_its_parent_hears_the_start_holds_it() {
        const T: u64 = 100_000;
        let (_root, mut runtime, spawn) = spawned_with("codex", BRIEF, false, T, true);
        assert_eq!(input_of(&runtime, &spawn).route, typed::SendRoute::Launch);
        runtime.typed_input_work(T + RECORD_WAIT_MS);
        let at = T + RECORD_WAIT_MS + REREAD_WAIT_MS;
        let work = runtime.typed_input_work(at);
        assert_eq!(work.len(), 1);
        let notice = work[0].notice.as_ref().expect("the parent is told");
        assert!(notice.body.starts_with(
            "worker has not taken its brief yet: read its screen with `hide pane read child`."
        ));
        assert!(notice.body.contains("Hide never types the brief"));
        assert!(work[0].resubmit.is_none(), "nothing is read to retype");

        runtime.note_delivery_key_at("child", at + 1_000);
        let quiet = at + 1_000 + crate::delivery::INPUT_QUIET_MS;
        assert!(
            runtime
                .typed_input_work(quiet)
                .iter()
                .all(|work| work.resubmit.is_none()),
            "an answered screen has nothing typed"
        );
        assert_eq!(input_of(&runtime, &spawn).attempts, 1);
    }

    /// A launched brief is owed its record from the moment its agent takes
    /// input, as a typed brief from its typing: a cold start that took
    /// twenty seconds raises no notice, and the wait then runs as before.
    #[test]
    fn a_launched_brief_whose_agent_is_ready_late_raises_no_notice() {
        const T: u64 = 100_000;
        const READY: u64 = T + 20_000;
        let (_root, mut runtime, spawn) = spawned_with("codex", BRIEF, false, T, true);
        let parent = Actor {
            pane_id: "sender".into(),
            name: "sender".into(),
            kind: "codex".into(),
            device_id: TEST_NODE.into(),
            session: crate::engine::tokens::session_digest("parent-native"),
        };
        let mut ledger = (**runtime.delivery_ledger.as_ref().unwrap()).clone();
        crate::coordination::apply(
            &mut ledger,
            &parent,
            &Mutation::BriefLaunched {
                id: spawn.clone(),
                words: Vec::new(),
            },
            READY,
        )
        .unwrap();
        runtime.publish_delivery(Arc::new(ledger), false);
        let input = input_of(&runtime, &spawn);
        assert_eq!(
            (input.typed_at_unix_ms, input.first_typed_at_unix_ms),
            (READY, T)
        );
        runtime.typed_input_work(T + RECORD_WAIT_MS);
        assert!(
            runtime
                .typed_input_work(T + RECORD_WAIT_MS + REREAD_WAIT_MS)
                .iter()
                .all(|work| work.notice.is_none()),
            "no notice counted from the spawn"
        );
        runtime.typed_input_work(READY + RECORD_WAIT_MS);
        assert!(
            runtime
                .typed_input_work(READY + RECORD_WAIT_MS + REREAD_WAIT_MS)
                .iter()
                .any(|work| work.notice.is_some()),
            "the wait runs from the agent's readiness"
        );
    }

    /// A launched brief's record is the prompt argument as the agent kept
    /// it, line breaks as U+2028 and a tab as four spaces (Codex, agy): the
    /// same canonical rule reads it as the brief, so it is taken.
    #[test]
    fn a_launched_brief_with_a_tab_is_taken_by_the_record_of_its_argument() {
        const T: u64 = 100_000;
        let brief = "Steps:\n\tadd the call\nReport by letter.";
        let (_root, mut runtime, _) = spawned_with("codex", brief, false, T, true);
        record_request(
            &mut runtime,
            "child",
            "codex",
            "Steps:\u{2028}    add the call\u{2028}Report by letter.",
            T + 1_000,
        );
        let work = runtime.typed_input_work(T + 1_000);
        assert_eq!(work.len(), 1);
        assert_eq!(work[0].recorded, Some("session"));
    }

    /// The agent row of `pane` whose session file's newest message from
    /// another party is `text`, written at `at`.
    pub(crate) fn record_request(
        runtime: &mut Runtime,
        pane: &str,
        kind: &str,
        text: &str,
        at: u64,
    ) {
        let mut row = crate::sidebar::project_agents(
            serde_json::from_value(json!({"agents": [
                {"pane_id": pane, "agent": kind, "agent_status": "idle", "state_change_seq": 2}
            ]}))
            .unwrap(),
        )
        .agents
        .remove(0);
        row.row_facts = Some(crate::request_view::RowFacts {
            other_request: Some(crate::labels::facts::Request {
                text: text.into(),
                offset: Some(1),
                cut: false,
                images: 0,
                at_unix_ms: at,
                requester: crate::labels::facts::Requester::Named("sender".into()),
                first: false,
            }),
            ..Default::default()
        });
        runtime.snapshot.navigator.agents.push(row);
    }

    fn input_of(runtime: &Runtime, spawn: &str) -> TypedInput {
        let ledger = runtime.delivery_ledger.as_ref().unwrap();
        ledger
            .spawns
            .iter()
            .find(|record| record.id == spawn)
            .unwrap()
            .brief_input
            .clone()
            .unwrap()
    }

    /// A brief the child never recorded owes its parent one notice naming
    /// the child's pane once the child rested past the record wait; it is
    /// typed again only after input reached the pane and went quiet, and the
    /// operator's row shows it only after the parent's grace.
    #[test]
    fn an_unrecorded_brief_tells_the_parent_then_waits_for_its_screen_to_be_answered() {
        // Typed well after the pane's last input, so its composer is readable
        // at rest.
        const T: u64 = 100_000;
        let (_root, mut runtime, spawn) = spawned(false, T);
        assert!(runtime.typed_input_work(T + RECORD_WAIT_MS - 1).is_empty());
        // Past the record wait, a fresh read of the child's session is asked
        // and given its time before the brief is judged.
        let asked = T + RECORD_WAIT_MS;
        assert!(runtime.typed_input_work(asked).is_empty());
        assert_eq!(runtime.take_session_rereads(), vec!["child".to_owned()]);
        assert!(
            runtime
                .typed_input_work(asked + REREAD_WAIT_MS - 1)
                .is_empty()
        );
        let at = asked + REREAD_WAIT_MS;
        let work = runtime.typed_input_work(at);
        assert_eq!(work.len(), 1);
        assert_eq!(work[0].key, InputKey::Spawn(spawn.clone()));
        let notice = work[0].notice.as_ref().expect("the parent is told");
        assert_eq!(notice.target.actor.pane_id, "sender");
        assert_eq!(notice.intent, format!("input-unconfirmed:{spawn}:1"));
        assert!(notice.body.starts_with(
            "worker has not confirmed its brief: read its screen with `hide pane read child`."
        ));
        let resubmit = work[0].resubmit.as_ref().expect("its composer is read");
        assert!(!resubmit.answered, "nobody answered the screen");
        assert!(resubmit.left);
        assert!(
            runtime.input_asks(at).is_empty(),
            "the parent clears it first"
        );

        // Someone answered the screen through Hide.
        runtime.note_delivery_key_at("child", at + 1_000);
        let quiet = at + 1_000 + crate::delivery::INPUT_QUIET_MS;
        assert!(
            runtime.typed_input_work(quiet - 1)[0].resubmit.is_none(),
            "nothing is read while someone types there"
        );
        let resubmit = runtime.typed_input_work(quiet)[0].resubmit.clone().unwrap();
        assert!(resubmit.answered);
        assert_eq!(resubmit.brief, BRIEF);

        // Told, the parent has its grace before the operator sees the row.
        let mut ledger = (**runtime.delivery_ledger.as_ref().unwrap()).clone();
        let input = Actor::hide(typed::INPUT_ROLE, TEST_NODE);
        crate::coordination::apply(
            &mut ledger,
            &input,
            &Mutation::BriefNoticed {
                id: spawn.clone(),
                attempt: 1,
            },
            at,
        )
        .unwrap();
        runtime.publish_delivery(Arc::new(ledger), false);
        assert!(
            runtime.typed_input_work(at).is_empty(),
            "one notice per typing"
        );
        assert!(runtime.typed_input_work(quiet)[0].notice.is_none());
        // A composer read showing the brief is the proof it was not taken.
        runtime.note_input_held(&spawn, 1, true);
        assert!(runtime.input_asks(at + PARENT_GRACE_MS - 1).is_empty());
        let shown = runtime.input_asks(at + PARENT_GRACE_MS);
        assert_eq!(
            shown,
            vec![(
                "child".to_owned(),
                T,
                AskSubject::InputUnconfirmed {
                    input: InputKind::Brief,
                    spent: false,
                    held: true,
                },
            )]
        );
        assert_eq!(input_of(&runtime, &spawn).attempts, 1);
    }

    /// An agent Hide has no source for its record of the brief, here a Pi
    /// whose session Hide has not read, is not judged to have lost it: its
    /// parent hears that it cannot be confirmed, and an answered screen does
    /// not have the brief typed again.
    #[test]
    fn a_brief_with_no_source_for_its_record_is_never_typed_again() {
        let (_root, mut runtime, _) = spawned_as("pi", false, 1_000);
        runtime.typed_input_work(1_000 + RECORD_WAIT_MS);
        let at = 1_000 + RECORD_WAIT_MS + REREAD_WAIT_MS;
        let work = runtime.typed_input_work(at);
        assert!(work[0].notice.as_ref().unwrap().body.starts_with(
            "worker cannot confirm its brief (Hide cannot read its session): read its screen"
        ));
        runtime.note_delivery_key_at("child", at + 1_000);
        let quiet = at + 1_000 + crate::delivery::INPUT_QUIET_MS;
        let resubmit = runtime.typed_input_work(quiet)[0].resubmit.clone().unwrap();
        assert!(
            !resubmit.answered,
            "only a composer still showing it gets its Enter"
        );
    }

    /// A handed-off child has no parent to clear its screen: its row shows
    /// the unrecorded brief as soon as it is unconfirmed.
    #[test]
    fn a_handed_off_childs_unrecorded_brief_reaches_the_operator_at_once() {
        let (_root, mut runtime, _) = spawned(true, 1_000);
        runtime.typed_input_work(1_000 + RECORD_WAIT_MS);
        let at = 1_000 + RECORD_WAIT_MS + REREAD_WAIT_MS;
        assert_eq!(runtime.input_asks(at).len(), 1);
    }

    /// An omp that started a turn after its brief was typed took the brief,
    /// confirmed at once however long the turn runs; an earlier turn or
    /// another agent's turn confirms nothing.
    #[test]
    fn an_omp_turn_after_its_brief_takes_the_brief_however_long_it_runs() {
        const T: u64 = 100_000;
        // omp writes the brief to its session only when its turn ends; a
        // first turn of ten minutes is no lost brief.
        let (_root, mut runtime, spawn) = spawned_as("omp", false, T);
        child_reads(&mut runtime, "working", None, T + 6_000);
        let work = runtime.typed_input_work(T + 6_000);
        assert_eq!(work.len(), 1);
        assert_eq!(work[0].key, InputKey::Spawn(spawn.clone()));
        assert_eq!(work[0].recorded, Some("turn"));
        assert!(work[0].notice.is_none() && work[0].resubmit.is_none());
        let late = runtime.typed_input_work(T + 600_000);
        assert_eq!(late[0].recorded, Some("turn"));
        assert!(runtime.input_asks(T + 600_000).is_empty());

        // A turn that began before the typing is not this brief's.
        let (_root, mut runtime, _) = spawned_as("omp", false, T);
        child_reads(&mut runtime, "working", None, T - 1);
        assert!(runtime.typed_input_work(T + 600_000).is_empty());

        // The core's turn decides, not Herdr's status: omp's hook starting
        // a turn after the typing takes the brief while Herdr still reads
        // the pane idle.
        let (_root, mut runtime, _) = spawned_as("omp", false, T);
        child_reads(&mut runtime, "idle", Some(HookTurn::Working), T + 6_000);
        assert_eq!(
            runtime.typed_input_work(T + 6_000)[0].recorded,
            Some("turn")
        );

        // Another session-file agent records the person's message at the
        // take, so its turn alone confirms nothing; it waits for its rest.
        let (_root, mut runtime, _) = spawned_as("pi", false, T);
        child_reads(&mut runtime, "working", None, T + 6_000);
        assert!(runtime.typed_input_work(T + 600_000).is_empty());
    }

    /// The child's pane as one pass lays it: Herdr reporting `herdr` since
    /// `at`, the hook of its session having reported `hook`, and the core's
    /// turn decided from both at `at`.
    fn child_reads(runtime: &mut Runtime, herdr: &str, hook: Option<HookTurn>, at: u64) {
        let child = runtime.delivery_observations.get_mut("child").unwrap();
        child.status = herdr.into();
        child.status_changed_at_unix_ms = at;
        if herdr == "working" {
            child.entered_working_at_unix_ms = at;
        }
        *child = child.clone().laid(hook, None);
        child.projected.as_mut().unwrap().changed_at_unix_ms = Some(at);
    }

    /// The hook says the child works on its brief while Herdr reads its
    /// pane idle: the core's turn, the one the row and the doorbell read,
    /// says it works, so no record is owed. Its parent is not told, the row
    /// shows nothing, and an answered screen does not have the brief typed
    /// into a working agent a second time.
    #[test]
    fn a_child_whose_hook_says_it_works_owes_no_record_while_herdr_reads_it_idle() {
        const T: u64 = 100_000;
        let (_root, mut runtime, _) = spawned(true, T);
        child_reads(&mut runtime, "idle", Some(HookTurn::Working), T + 1_000);
        runtime.note_delivery_key_at("child", T + 2_000);
        for now in [T + RECORD_WAIT_MS, T + 10 * RECORD_WAIT_MS] {
            assert!(runtime.typed_input_work(now).is_empty(), "at {now}");
            assert!(runtime.input_asks(now).is_empty(), "at {now}");
        }
        assert!(runtime.take_session_rereads().is_empty());
    }

    /// The hook says the child's turn ended while Herdr still reads it
    /// working: the child rests by the core's turn, so after the record wait
    /// its parent hears, and once someone answered its screen the brief is
    /// typed again, as for any child at rest.
    #[test]
    fn a_child_whose_hook_says_it_rests_is_unconfirmed_while_herdr_reads_it_working() {
        const T: u64 = 100_000;
        let (_root, mut runtime, _) = spawned(false, T);
        child_reads(&mut runtime, "working", Some(HookTurn::Idle), T + 1_000);
        let asked = T + 1_000 + RECORD_WAIT_MS;
        assert!(
            runtime.typed_input_work(asked - 1).is_empty(),
            "its rest counts from the turn's end"
        );
        assert!(runtime.typed_input_work(asked).is_empty());
        assert_eq!(runtime.take_session_rereads(), vec!["child".to_owned()]);
        let at = asked + REREAD_WAIT_MS;
        let work = runtime.typed_input_work(at);
        assert!(work[0].notice.is_some(), "the parent is told");
        runtime.note_delivery_key_at("child", at + 1_000);
        let quiet = at + 1_000 + crate::delivery::INPUT_QUIET_MS;
        let resubmit = runtime.typed_input_work(quiet)[0].resubmit.clone().unwrap();
        assert!(resubmit.answered);
    }

    /// A child whose record of the brief never matches, but whose hook
    /// reported a turn started after the typing with nothing else to start
    /// it, took the brief: the turn is its record, as for omp. Its parent is
    /// never told, its row asks nothing, `hide agent show` lists no pending
    /// brief, and nothing is typed again, during the turn and after it,
    /// whoever answers its screen.
    #[test]
    fn a_brief_whose_hook_reports_a_turn_after_the_typing_is_confirmed_by_that_turn() {
        const T: u64 = 100_000;
        let (_root, mut runtime, spawn) = spawned(true, T);
        child_reads(&mut runtime, "idle", Some(HookTurn::Working), T + 1_000);
        let work = runtime.typed_input_work(T + 1_000);
        assert_eq!(work.len(), 1);
        assert_eq!(work[0].recorded, Some("turn"));
        assert!(work[0].notice.is_none() && work[0].resubmit.is_none());

        // The turn ends with no record of the brief, and someone answers.
        child_reads(&mut runtime, "idle", Some(HookTurn::Done), T + 60_000);
        runtime.note_delivery_key_at("child", T + 61_000);
        let late = T + 600_000;
        let work = runtime.typed_input_work(late);
        assert_eq!(work.len(), 1);
        assert_eq!(work[0].recorded, Some("turn"));
        assert!(work[0].notice.is_none() && work[0].resubmit.is_none());
        assert!(runtime.take_session_rereads().is_empty());
        assert!(runtime.input_asks(late).is_empty(), "the row asks nothing");
        assert_eq!(shown_phase(&runtime, &spawn), serde_json::Value::Null);
    }

    /// A turn that only Herdr reads, or one something else could have
    /// started (a key, a letter's bell after the typing), is no record: the
    /// brief is judged by its record alone, and its parent hears once it
    /// rests unconfirmed.
    #[test]
    fn a_turn_herdr_alone_reads_or_other_input_started_is_no_record_of_the_brief() {
        const T: u64 = 100_000;
        let judged = |runtime: &mut Runtime| {
            assert!(runtime.typed_input_work(T + 1_000).is_empty(), "it works");
            child_reads(runtime, "idle", Some(HookTurn::Done), T + 60_000);
            let asked = T + 60_000 + RECORD_WAIT_MS;
            runtime.typed_input_work(asked);
            runtime.typed_input_work(asked + REREAD_WAIT_MS)[0]
                .notice
                .is_some()
        };

        let (_root, mut runtime, _) = spawned(false, T);
        child_reads(&mut runtime, "working", None, T + 1_000);
        assert!(judged(&mut runtime), "Herdr's reading alone");

        let (_root, mut runtime, _) = spawned(false, T);
        runtime.note_delivery_key_at("child", T + 500);
        child_reads(&mut runtime, "idle", Some(HookTurn::Working), T + 1_000);
        assert!(judged(&mut runtime), "a key after the typing");

        let (_root, mut runtime, _) = spawned(false, T);
        let mut ledger = (**runtime.delivery_ledger.as_ref().unwrap()).clone();
        let sender = ledger.agents[0].actor.clone();
        let recipient = Actor {
            pane_id: "child".into(),
            name: "worker".into(),
            kind: "codex".into(),
            device_id: TEST_NODE.into(),
            session: crate::engine::tokens::session_digest("child-native"),
        };
        let letter = crate::delivery::mailbox::send(
            &mut ledger,
            &sender,
            &recipient,
            "ring",
            "hello",
            "request",
            None,
            T,
        )
        .unwrap();
        ledger
            .letters
            .iter_mut()
            .find(|stored| stored.id == letter.id)
            .unwrap()
            .bell_typed_at_unix_ms = Some(T + 500);
        runtime.publish_delivery(Arc::new(ledger), false);
        child_reads(&mut runtime, "idle", Some(HookTurn::Working), T + 1_000);
        assert!(judged(&mut runtime), "a letter's bell after the typing");
    }

    /// The phase `hide agent show` gives the spawn's brief, as its parent
    /// asks.
    fn shown_phase(runtime: &Runtime, spawn: &str) -> serde_json::Value {
        let parent = Actor {
            pane_id: "sender".into(),
            name: "sender".into(),
            kind: "codex".into(),
            device_id: TEST_NODE.into(),
            session: crate::engine::tokens::session_digest("parent-native"),
        };
        crate::coordination::executor::Shown::read(runtime, &parent, spawn)
            .unwrap()
            .answer(TEST_NODE)["state"]["pending_inputs"][0]["phase"]
            .clone()
    }

    /// While the child's read is stale the core has no turn for it, and the
    /// brief's verdict stands however long that lasts: the row keeps the
    /// brief's ask, `hide agent show` still says unconfirmed, nothing is
    /// sent or typed, and the fresh session read already given its time is
    /// not asked again. A fresh read's turn takes the verdict on from where
    /// it was, and a pass that observed the pane before laying its turn
    /// holds it the same way.
    #[test]
    fn a_stale_read_keeps_the_briefs_verdict_until_a_fresh_turn_is_laid() {
        let (_root, mut runtime, spawn) = spawned(true, 1_000);
        runtime.typed_input_work(1_000 + RECORD_WAIT_MS);
        let _ = runtime.take_session_rereads();
        let at = 1_000 + RECORD_WAIT_MS + REREAD_WAIT_MS;
        runtime.typed_input_work(at);
        runtime.publish_input_asks(at);
        let asked = runtime.input_asks(at);
        assert_eq!(asked.len(), 1);
        assert_eq!(shown_phase(&runtime, &spawn), "unconfirmed");

        runtime
            .delivery_observations
            .get_mut("child")
            .unwrap()
            .telemetry_stale = true;
        let later = at + 600_000;
        assert!(runtime.typed_input_work(later).is_empty());
        assert_eq!(runtime.input_asks(later), asked, "the row keeps the ask");
        assert_eq!(shown_phase(&runtime, &spawn), "unconfirmed");
        assert!(runtime.take_session_rereads().is_empty());

        // The read is fresh again and its pass lays the turn.
        let child = runtime.delivery_observations.get_mut("child").unwrap();
        child.telemetry_stale = false;
        *child = child.clone().laid(None, None);
        assert_eq!(runtime.input_asks(later), asked);
        assert!(runtime.typed_input_work(later)[0].resubmit.is_some());
        assert!(runtime.take_session_rereads().is_empty());

        // A pass observed the pane and has not laid its turn yet.
        runtime
            .delivery_observations
            .get_mut("child")
            .unwrap()
            .observed_generation += 1;
        assert!(runtime.typed_input_work(later + 1).is_empty());
        assert_eq!(runtime.input_asks(later + 1), asked);
        assert_eq!(shown_phase(&runtime, &spawn), "unconfirmed");

        // A pane Herdr no longer lists takes its input with it.
        runtime.delivery_observations.remove("child");
        assert!(runtime.typed_input_work(later + 2).is_empty());
        assert!(runtime.input_asks(later + 2).is_empty());
    }

    /// An agent whose session file records the brief among its user
    /// messages confirms it: the work says so and owes no notice.
    #[test]
    fn a_brief_in_the_agents_session_file_is_its_record() {
        let (_root, mut runtime, spawn) = spawned(false, 1_000);
        let mut row = crate::sidebar::project_agents(
            serde_json::from_value(json!({"agents": [
                {"pane_id": "child", "agent": "codex", "agent_status": "idle", "state_change_seq": 2}
            ]}))
            .unwrap(),
        )
        .agents
        .remove(0);
        row.row_facts = Some(crate::request_view::RowFacts {
            prompt_digests: vec![hide_agent_hooks::delivery::prompt_digest(BRIEF)],
            ..Default::default()
        });
        runtime.snapshot.navigator.agents.push(row);
        let work = runtime.typed_input_work(1_000 + RECORD_WAIT_MS);
        assert_eq!(work.len(), 1);
        assert_eq!(work[0].key, InputKey::Spawn(spawn));
        assert_eq!(work[0].recorded, Some("session"));
        assert!(work[0].notice.is_none());
    }

    /// A message that holds the brief after what the composer already held,
    /// such as the terminal report Antigravity on Windows took as typed
    /// (`27;1$y`, agent journey 2026-11-01), is the brief's record; a message
    /// that holds only part of it, or more after it, is not.
    #[test]
    fn a_message_that_ends_with_the_brief_is_its_record() {
        for (text, recorded) in [
            (format!("27;1$y{BRIEF}"), true),
            (BRIEF.lines().last().unwrap().to_owned(), false),
            (format!("{BRIEF} and more"), false),
        ] {
            let (_root, mut runtime, spawn) = spawned_as("antigravity", false, 1_000);
            let mut row = crate::sidebar::project_agents(
                serde_json::from_value(json!({"agents": [
                    {"pane_id": "child", "agent": "antigravity", "agent_status": "idle", "state_change_seq": 2}
                ]}))
                .unwrap(),
            )
            .agents
            .remove(0);
            row.row_facts = Some(crate::request_view::RowFacts {
                other_request: Some(crate::labels::facts::Request {
                    text: text.clone(),
                    offset: Some(1),
                    cut: false,
                    images: 0,
                    at_unix_ms: 1_500,
                    requester: crate::labels::facts::Requester::Agent,
                    first: true,
                }),
                ..Default::default()
            });
            runtime.snapshot.navigator.agents.push(row);
            let work = runtime.typed_input_work(1_000 + RECORD_WAIT_MS);
            let found = work
                .iter()
                .find(|work| work.key == InputKey::Spawn(spawn.clone()))
                .and_then(|work| work.recorded);
            assert_eq!(found == Some("session"), recorded, "{text:?}");
        }
    }

    /// The record Antigravity on Windows wrote for a brief Hide pasted, with
    /// the terminal report its composer took first (agent journey run
    /// 41278776284), is the brief's record; the same words stamped before the
    /// typing, past the two clocks' slack, or a message Hide saw the operator
    /// submit, are not.
    #[test]
    fn antigravitys_record_after_a_terminal_report_is_the_brief_only_after_the_typing() {
        use crate::labels::facts::CLOCK_SLACK_MS;
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/antigravity-windows-brief-record.json"
        ))
        .unwrap();
        let typed = fixture["typed"].as_str().unwrap();
        let record = &fixture["record"];
        let at = record["at_unix_ms"].as_u64().unwrap();
        let sender = record["sender"].clone();
        for (typed_at, requester, recorded) in [
            (at - 500, sender.clone(), true),
            // The session stamps whole seconds.
            (at + 900, sender.clone(), true),
            (at + CLOCK_SLACK_MS + 1, sender.clone(), false),
            (at - 500, json!({"kind": "operator"}), false),
        ] {
            let (_root, mut runtime, spawn) =
                spawned_with("antigravity", typed, false, typed_at, false);
            let mut row = crate::sidebar::project_agents(
                serde_json::from_value(json!({"agents": [
                    {"pane_id": "child", "agent": "antigravity", "agent_status": "idle", "state_change_seq": 2}
                ]}))
                .unwrap(),
            )
            .agents
            .remove(0);
            row.row_facts = Some(crate::request_view::RowFacts {
                other_request: Some(
                    serde_json::from_value(json!({
                        "text": record["text"], "cut": record["cut"], "images": 0,
                        "at_unix_ms": at, "requester": requester, "first": true,
                    }))
                    .unwrap(),
                ),
                ..Default::default()
            });
            runtime.snapshot.navigator.agents.push(row);
            let found = runtime
                .typed_input_work(typed_at + RECORD_WAIT_MS)
                .into_iter()
                .find(|work| work.key == InputKey::Spawn(spawn.clone()))
                .and_then(|work| work.recorded);
            assert_eq!(found == Some("session"), recorded, "{typed_at} {requester}");
        }
    }

    /// A record the agent wrote after the first typing stays the brief's
    /// record once Hide typed it again later (each typing moves
    /// `typed_at_unix_ms`): Antigravity's record at `at`, then a second
    /// typing well past the clocks' slack.
    #[test]
    fn a_record_after_the_first_typing_survives_a_later_typing() {
        use crate::labels::facts::CLOCK_SLACK_MS;
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/antigravity-windows-brief-record.json"
        ))
        .unwrap();
        let typed = fixture["typed"].as_str().unwrap();
        let record = &fixture["record"];
        let at = record["at_unix_ms"].as_u64().unwrap();
        let (_root, mut runtime, spawn) =
            spawned_with("antigravity", typed, false, at - 500, false);
        let retyped = at + CLOCK_SLACK_MS + 10_000;
        let mut ledger = (**runtime.delivery_ledger.as_ref().unwrap()).clone();
        crate::coordination::apply(
            &mut ledger,
            &Actor::hide(typed::INPUT_ROLE, TEST_NODE),
            &Mutation::BriefTyping { id: spawn.clone() },
            retyped,
        )
        .unwrap();
        runtime.publish_delivery(Arc::new(ledger), false);
        let input = input_of(&runtime, &spawn);
        assert_eq!(
            (
                input.attempts,
                input.first_typed_at(),
                input.typed_at_unix_ms
            ),
            (2, at - 500, retyped)
        );
        let mut row = crate::sidebar::project_agents(
            serde_json::from_value(json!({"agents": [
                {"pane_id": "child", "agent": "antigravity", "agent_status": "idle", "state_change_seq": 2}
            ]}))
            .unwrap(),
        )
        .agents
        .remove(0);
        row.row_facts = Some(crate::request_view::RowFacts {
            other_request: Some(
                serde_json::from_value(json!({
                    "text": record["text"], "cut": record["cut"], "images": 0,
                    "at_unix_ms": at, "requester": record["sender"], "first": true,
                }))
                .unwrap(),
            ),
            ..Default::default()
        });
        runtime.snapshot.navigator.agents.push(row);
        let found = runtime
            .typed_input_work(retyped + RECORD_WAIT_MS)
            .into_iter()
            .find(|work| work.key == InputKey::Spawn(spawn.clone()))
            .and_then(|work| work.recorded);
        assert_eq!(found, Some("session"));
    }
}
