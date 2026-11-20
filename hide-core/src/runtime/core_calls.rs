//! Letters, reads and starts between this core's agents and another
//! machine's (PRD device-connect-iroh D-71, D-72).
//!
//! Another core's agent enters this core where an external tool does: as a
//! code-owned identity of its own (`Actor::core`), one per sending agent,
//! whose commands `access::core_admit` and `access::core_call_admit` judge
//! by the tool table at Act and by how its machine stands, read from the
//! paired machines the daemon told the core (`crate::peer_machines`). The
//! daemon carries the messages and decides nothing.
//!
//! A letter one of this core's agents writes to `core:<device>:agent:<id>`
//! is an ordinary ledger letter whose recipient is that identity. It stays
//! `pending`, on the clock of the connection to that machine, until the
//! other core answers: taken, it is delivered; refused, it ends undelivered
//! and its sender gets one letter from `hide:delivery` with the reason. A
//! reply names the letter it answers by the id its own core gave it
//! (`Letter::core_id`), and is run here as the recipient of that letter.
//!
//! What is owed a refused letter is memory, as a spent bell's is
//! (`spent_bells`): the store ends the letter on its next pass and the note
//! goes once. Bounded by the open letters.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::json;

use super::Runtime;
use super::delivery::Observation;
use super::hide_notes::{Note, NoteKind};
use crate::access::{self, CoreAsk, MACHINE_NOT_PERMITTED};
use crate::address::CoreAddress;
use crate::core_calls::{
    Asked, Call, CoreItem, CoreLink, Forward, Letter, Outcome, Party, Settle, Taking,
};
use crate::delivery::ledger::{Ledger, State};
use crate::delivery::worker::{Effect, Proven};
use crate::delivery::{Actor, Command};
use crate::peer_machines::PeerMachine;
use crate::workspace_control::Context;

/// The other machine cannot be asked now: it is not connected.
const MACHINE_UNAVAILABLE: &str = "machine_unavailable";

/// Two paired machines say the device id a `core:` name carries.
const DEVICE_AMBIGUOUS: &str = "device_ambiguous";

#[derive(Default)]
pub(in crate::runtime) struct CoreLetters {
    /// The connection to the other cores; none in a core no daemon gave one.
    link: Option<Arc<dyn CoreLink>>,
    /// The daemon told the paired machines at least once, so a machine not
    /// among them is one this core is paired with no longer.
    told: bool,
    /// Letters another core refused, with why, until the store ended them
    /// and their sender was told.
    refused: BTreeMap<String, Refused>,
    /// A thread is telling senders; it is the only one.
    noting: bool,
    /// The letters waiting to go to another core at the last publish.
    outbound: Vec<String>,
}

struct Refused {
    reason: String,
    noted: bool,
}

/// `actor` as its own core names it to another: by its registration,
/// ended or not, while the ledger keeps one, and by its pane otherwise.
fn core_party(ledger: &Ledger, actor: &Actor) -> Party {
    let record = ledger
        .agents
        .iter()
        .rev()
        .filter(|record| record.actor.same_identity(actor))
        .min_by_key(|record| record.ended);
    match record {
        Some(record) => Party {
            item: CoreItem::Agent,
            id: record.id.clone(),
            name: record.wire_name().to_owned(),
        },
        None => Party {
            item: CoreItem::Pane,
            id: actor.pane_id.clone(),
            name: actor.name.clone(),
        },
    }
}

impl Runtime {
    /// Gives the core its connection to the other cores.
    pub(crate) fn set_core_link(&mut self, link: Arc<dyn CoreLink>) {
        self.core_letters.link = Some(link);
    }

    /// The daemon told the paired machines.
    pub(super) fn core_machines_told(&mut self) {
        self.core_letters.told = true;
        // A machine that connected takes what waited for it.
        if let Some(link) = &self.core_letters.link
            && !self.core_letters.outbound.is_empty()
        {
            link.outbox_moved();
        }
    }

    /// The machines this core is paired with, by connection id, for the
    /// store's upkeep; `None` until the daemon told them, so a start ends
    /// nothing before it knows the list.
    pub(crate) fn delivery_core_machines(&self) -> Option<Vec<String>> {
        self.core_letters.told.then(|| {
            self.snapshot
                .navigator
                .peers
                .iter()
                .map(|machine| machine.key.clone())
                .collect()
        })
    }

    /// The connections to other cores, as the ledger's clocks name them,
    /// with whether each stands now (`Actor::reach`).
    pub(super) fn core_reach(&self) -> impl Iterator<Item = (String, bool)> + '_ {
        self.snapshot
            .navigator
            .peers
            .iter()
            .filter(|machine| {
                access::core_standing(machine).is_some()
                    || access::core_standing_there(machine).is_some()
            })
            .map(|machine| (Actor::core_reach(&machine.device_id), machine.connected))
    }

    /// Tells the daemon when the letters waiting to go to another core
    /// changed, from the ledger just published.
    pub(super) fn note_core_outbox(&mut self, ledger: &Ledger) {
        let outbound: Vec<String> = ledger
            .letters
            .iter()
            .filter(|letter| {
                letter.state == State::Pending && letter.recipient.core_address().is_some()
            })
            .map(|letter| letter.id.clone())
            .collect();
        if outbound == self.core_letters.outbound {
            return;
        }
        let more = outbound
            .iter()
            .any(|id| !self.core_letters.outbound.contains(id));
        self.core_letters.outbound = outbound;
        if more && let Some(link) = &self.core_letters.link {
            link.outbox_moved();
        }
    }

    fn peer_keyed(&self, key: &str) -> Result<&PeerMachine, String> {
        self.snapshot
            .navigator
            .peers
            .iter()
            .find(|machine| machine.key == key)
            .ok_or_else(|| MACHINE_NOT_PERMITTED.to_owned())
    }

    /// The one paired machine that says device id `device`. A device id is
    /// what a machine says of itself, so two connections may say the same
    /// one, as `hide device list` shows them; a `core:` name then names
    /// neither, and what was stored for one of them stays its own
    /// (`core_machine`).
    pub(super) fn peer_on(&self, device: &str) -> Result<&PeerMachine, String> {
        let mut named = self
            .snapshot
            .navigator
            .peers
            .iter()
            .filter(|machine| machine.device_id == device);
        match (named.next(), named.next()) {
            (Some(machine), None) => Ok(machine),
            (Some(_), Some(_)) => Err(DEVICE_AMBIGUOUS.into()),
            (None, _) => Err(MACHINE_NOT_PERMITTED.into()),
        }
    }

    /// The paired machine `actor` is an agent of: the one on the connection
    /// its identity was made on (`Actor::core_of`), while it says the
    /// device id the identity names.
    fn core_machine(&self, actor: &Actor) -> Option<&PeerMachine> {
        let address = actor.core_address()?;
        self.snapshot
            .navigator
            .peers
            .iter()
            .find(|machine| machine.device_id == address.device && actor.core_of(&machine.key))
    }

    /// The agent `party` of `machine`, as this core knows it.
    pub(super) fn core_actor(&self, machine: &PeerMachine, party: &Party) -> Actor {
        Actor::core(
            CoreAddress::new(&machine.device_id, party.item, &party.id),
            &machine.key,
            &machine.name,
            &party.name,
            self.node.as_str(),
        )
    }

    /// The caller `actor`, an agent of the core on `machine`, acts as
    /// (`Actor::core_caller`): kept with what it asked until that applies.
    fn core_caller(machine: &PeerMachine, actor: &Actor) -> String {
        Actor::core_caller(&machine.key, &actor.pane_id)
    }

    /// No pane's context holds another core's authority: its machine's
    /// standing does, read again when the command applies.
    pub(super) fn core_context(&self, actor: &Actor) -> Context {
        Context {
            device_id: self.node.as_str().to_owned(),
            workspace_id: actor.pane_id.clone(),
            checkout_id: actor.pane_id.clone(),
            checkout_path: String::new(),
        }
    }

    /// Whether another core's agent still acts here: the machine on its
    /// connection stands in at least one direction
    /// (`Runtime::code_owned_authority_current`). What it may do there is
    /// `core_command_current`'s to say.
    pub(super) fn core_actor_current(&self, actor: &Actor) -> bool {
        self.core_machine(actor)
            .is_some_and(|machine| access::core_standing(machine).is_some())
    }

    /// What `command` of an agent of the core on `machine` is, to a
    /// machine that only answers.
    fn core_ask(&self, machine: &PeerMachine, command: &Command) -> CoreAsk {
        match command {
            // The store lets only a letter's recipient answer it.
            Command::Reply { .. } => CoreAsk::Reply,
            // The result report of a start this core's agent asked that
            // connection for.
            Command::Send { target, kind, .. }
                if kind == "report"
                    && self.delivery_ledger.as_ref().is_ok_and(|ledger| {
                        ledger.core_starts.iter().any(|start| {
                            start.key == machine.key
                                && core_party(ledger, &start.origin).id == *target
                        })
                    }) =>
            {
                CoreAsk::ResultReport
            }
            _ => CoreAsk::First,
        }
    }

    /// Judges `command` of an agent of the core on `machine` by how that
    /// machine stands now. The one table for the moment a command is
    /// prepared and the moment it applies (D-72): a grant paused or taken
    /// back in between refuses what it alone allowed.
    fn core_inbound_admit(&self, machine: &PeerMachine, command: &Command) -> Result<(), String> {
        let standing = access::core_standing(machine).ok_or(MACHINE_NOT_PERMITTED)?;
        match command {
            Command::Agents { .. } => Ok(access::core_call_admit(
                standing,
                &Asked::Delivery(Box::new(command.clone())),
            )?),
            _ => Ok(access::core_admit(
                standing,
                command,
                self.core_ask(machine, command),
            )?),
        }
    }

    /// Whether another core's agent `actor` still runs `command` here,
    /// read again just before it applies.
    pub(crate) fn core_command_current(
        &self,
        actor: &Actor,
        command: &Command,
    ) -> Result<(), String> {
        let machine = self.core_machine(actor).ok_or(MACHINE_NOT_PERMITTED)?;
        self.core_inbound_admit(machine, command)
    }

    /// Whether another core's agent `actor` still changes this core's
    /// agents with `mutation`, read again just before it applies: a start
    /// by the table that admitted it, and every other change, a step of
    /// such a start, only from a machine that opens this one.
    pub(crate) fn core_mutation_current(
        &self,
        actor: &Actor,
        mutation: &crate::coordination::Mutation,
    ) -> Result<(), String> {
        let machine = self.core_machine(actor).ok_or(MACHINE_NOT_PERMITTED)?;
        match mutation {
            crate::coordination::Mutation::Reserve { command, .. } => self.core_inbound_admit(
                machine,
                &Command::Agents {
                    command: command.clone(),
                },
            ),
            _ if access::core_standing(machine) == Some(access::CoreStanding::Opens) => Ok(()),
            _ => Err(MACHINE_NOT_PERMITTED.into()),
        }
    }

    /// Whether `sender`'s letter `command` still goes to `recipient`, an
    /// agent of another core, read again just before the letter is stored.
    pub(crate) fn core_recipient_current(
        &self,
        sender: &Actor,
        recipient: &Actor,
        command: &Command,
    ) -> bool {
        self.core_send_admitted(sender, recipient, command).is_ok()
    }

    fn core_send_admitted(
        &self,
        sender: &Actor,
        recipient: &Actor,
        command: &Command,
    ) -> Result<(), String> {
        // Only an agent writes to another core: a tool hears only what it
        // asked for here, and another core's agent is not passed on.
        if sender.code_owned() {
            return Err(MACHINE_NOT_PERMITTED.into());
        }
        // The machine on the connection the recipient's identity was made
        // on, so a letter kept for one connection never turns to another
        // that says the same device id.
        let machine = self.core_machine(recipient).ok_or(MACHINE_NOT_PERMITTED)?;
        let standing = access::core_standing_there(machine).ok_or(MACHINE_NOT_PERMITTED)?;
        // The result report of a start that machine's agent asked for
        // here: that agent is the origin of the work, registered when it
        // started it.
        let reports = matches!(command, Command::Send { kind, .. } if kind == "report")
            && self.delivery_ledger.as_ref().is_ok_and(|ledger| {
                ledger
                    .agents
                    .iter()
                    .any(|record| record.actor.same_identity(recipient))
            });
        let ask = if reports {
            CoreAsk::ResultReport
        } else {
            CoreAsk::First
        };
        Ok(access::core_admit(standing, command, ask)?)
    }

    /// Who a send that names another core's agent goes to (`core:<device>:
    /// agent|pane:<id>`): that agent as this core knows it, with no pane to
    /// observe. `None` for any other target.
    pub(super) fn core_send_target(
        &self,
        sender: &Actor,
        command: &Command,
    ) -> Result<Option<Observation>, String> {
        let Command::Send { target, force, .. } = command else {
            return Ok(None);
        };
        let Some(address) = CoreAddress::parse(target) else {
            return Ok(None);
        };
        let address = address.map_err(|_| "target_unavailable".to_owned())?;
        let machine = self.peer_on(address.device)?;
        let recipient = Actor::core(
            address,
            &machine.key,
            &machine.name,
            address.id,
            self.node.as_str(),
        );
        self.core_send_admitted(sender, &recipient, command)?;
        if *force {
            // No composer of another core's pane is read from here.
            return Err("force_target_asleep".into());
        }
        Ok(Some(super::agent_sleep::letters::asleep_observation(
            recipient,
        )))
    }

    /// The letters waiting to go to machine `key`, as its core takes them.
    pub(crate) fn core_outbox(&mut self, key: &str) -> Vec<Letter> {
        if self.peer_keyed(key).is_err() {
            return Vec::new();
        }
        let Ok(ledger) = self.delivery_state() else {
            return Vec::new();
        };
        let mut letters = Vec::new();
        for letter in &ledger.letters {
            if letter.state != State::Pending || self.core_letters.refused.contains_key(&letter.id)
            {
                continue;
            }
            // Only what was written to an agent of this connection.
            let Some(address) = letter
                .recipient
                .core_address()
                .filter(|_| letter.recipient.core_of(key))
            else {
                continue;
            };
            // A reply names the letter it answers as its sender's core
            // does; one whose original this ledger no longer keeps cannot
            // say which letter it answers.
            let reply_to = match &letter.reply_to {
                None => None,
                Some(local) => match ledger
                    .letters
                    .iter()
                    .find(|original| original.id == *local)
                    .and_then(|original| original.core_id.clone())
                {
                    Some(theirs) => Some(theirs),
                    None => {
                        self.core_letter_refused(&letter.id, "letter_unavailable");
                        continue;
                    }
                },
            };
            letters.push(Letter {
                id: letter.id.clone(),
                from: core_party(&ledger, &letter.sender),
                to: address.id.to_owned(),
                reply_to,
                kind: letter.kind.clone(),
                intent: letter.intent.clone(),
                body: letter.body.clone(),
                now: letter.immediate,
            });
        }
        letters
    }

    fn core_letter_refused(&mut self, id: &str, reason: &str) {
        crate::diagnostic!(json!({
            "component": "delivery", "kind": "core_letter.refused",
            "letter_id": id, "reason": reason,
        }));
        self.core_letters.refused.insert(
            id.to_owned(),
            Refused {
                reason: reason.to_owned(),
                noted: false,
            },
        );
    }

    /// The letters another core refused, with why, for the store's pass
    /// (`Runtime::delivery_undeliverable`).
    pub(super) fn core_letters_refused(&self) -> impl Iterator<Item = (&String, &String)> {
        self.core_letters
            .refused
            .iter()
            .map(|(id, refused)| (id, &refused.reason))
    }

    /// What machine `key`'s core answered for this core's letter `id`:
    /// taken, the write that delivers it; refused, nothing to write now,
    /// since the store ends it and its sender is told
    /// (`Runtime::drive_core_refusals`).
    pub(crate) fn core_letter_answered(
        &mut self,
        key: &str,
        id: &str,
        outcome: Outcome,
    ) -> Option<Settle> {
        let machine = self.peer_keyed(key).ok()?.clone();
        match outcome {
            Outcome::Taken { recipient } => Some(Settle {
                client: self.delivery_client.clone()?,
                effect: Effect::CoreSettled {
                    id: id.to_owned(),
                    key: machine.key,
                    name: Actor::core_name(&machine.name, &recipient.name),
                },
            }),
            Outcome::Refused { reason } => {
                // Only a letter still on its way to that machine.
                let waits = self.delivery_ledger.as_ref().is_ok_and(|ledger| {
                    ledger.letters.iter().any(|letter| {
                        letter.id == id
                            && letter.state == State::Pending
                            && letter.recipient.core_of(&machine.key)
                    })
                });
                if waits {
                    self.core_letter_refused(id, &reason);
                }
                None
            }
        }
    }

    /// A letter machine `key`'s core sent for one of its agents, as the
    /// command that agent runs here.
    pub(crate) fn prepare_core_letter(
        &mut self,
        key: &str,
        letter: Letter,
    ) -> Result<Taking, String> {
        let machine = self.peer_keyed(key)?.clone();
        let ledger = self.delivery_state()?;
        let (actor, command, answered) = match letter.reply_to {
            Some(original) => {
                // It answers a letter this core sent that machine, whose
                // recipient is the agent that answers now.
                let stored = ledger
                    .letters
                    .iter()
                    .find(|stored| stored.id == original)
                    .filter(|stored| stored.recipient.core_of(&machine.key))
                    .ok_or("letter_unavailable")?;
                let actor = Actor {
                    name: Actor::core_name(&machine.name, &letter.from.name),
                    ..stored.recipient.clone()
                };
                let command = Command::Reply {
                    id: original,
                    intent: letter.intent,
                    body: letter.body,
                    now: letter.now,
                };
                (actor, command, Some(stored.sender.clone()))
            }
            None => {
                let actor = self.core_actor(&machine, &letter.from);
                let command = Command::Send {
                    target: letter.to,
                    intent: letter.intent,
                    body: letter.body,
                    kind: letter.kind,
                    now: letter.now,
                    force: false,
                };
                (actor, command, None)
            }
        };
        self.core_inbound_admit(&machine, &command)?;
        let intent = match &command {
            Command::Send { intent, .. } | Command::Reply { intent, .. } => intent.clone(),
            _ => String::new(),
        };
        let context = self.core_context(&actor);
        let caller = Self::core_caller(&machine, &actor);
        let node = self.node.as_str().to_owned();
        let prepared =
            self.prepare_as(&node, &caller, context, Proven::Now, actor.clone(), command)?;
        // Who took it: the recipient of the same letter kept before, the
        // sender of the letter a reply answers, or the agent the send
        // found.
        let recipient = crate::delivery::mailbox::existing_intent(
            &ledger,
            &actor,
            &intent,
            crate::delivery::worker::now(),
        )
        .map(|stored| stored.recipient.clone())
        .or(answered)
        .or_else(|| prepared.target_actor().cloned())
        .ok_or("target_unavailable")?;
        Ok(Taking {
            prepared,
            client: self.delivery_client.clone().ok_or("delivery_unavailable")?,
            core_id: letter.id,
            recipient: core_party(&ledger, &recipient),
        })
    }

    /// A read or a start machine `key`'s core asked for one of its agents.
    pub(crate) fn prepare_core_call(
        &mut self,
        key: &str,
        from: &Party,
        asked: Asked,
    ) -> Result<Call, String> {
        let machine = self.peer_keyed(key)?.clone();
        let standing = access::core_standing(&machine).ok_or(MACHINE_NOT_PERMITTED)?;
        access::core_call_admit(standing, &asked)?;
        let actor = self.core_actor(&machine, from);
        let context = self.core_context(&actor);
        let node = self.node.as_str().to_owned();
        match asked {
            Asked::Delivery(command) => {
                let caller = Self::core_caller(&machine, &actor);
                self.prepare_as(&node, &caller, context, Proven::Now, actor, *command)
                    .map(|prepared| Call::Delivery(Box::new(prepared)))
            }
            Asked::Pane(command) => self
                .pane_command_as(&node, "", &context, command)
                .map(|prepared| Call::Pane(Box::new(prepared))),
        }
    }

    /// The connection a command of `actor` goes over to the core on
    /// `device`, which this machine opens and is connected to now.
    fn core_forward(&self, device: &str, actor: &Actor, asked: Asked) -> Result<Forward, String> {
        // A tool's work is its core's, and another core's agent is not
        // passed on.
        if actor.code_owned() {
            return Err(MACHINE_NOT_PERMITTED.into());
        }
        let machine = self.peer_on(device)?;
        let standing = access::core_standing_there(machine).ok_or(MACHINE_NOT_PERMITTED)?;
        access::core_call_admit(standing, &asked)?;
        if !machine.connected {
            return Err(MACHINE_UNAVAILABLE.into());
        }
        let link = self.core_letters.link.clone().ok_or(MACHINE_UNAVAILABLE)?;
        let from = match self.delivery_ledger.as_ref() {
            Ok(ledger) => core_party(ledger, actor),
            Err(code) => return Err(code.clone()),
        };
        Ok(Forward {
            link,
            key: machine.key.clone(),
            from,
            asked,
        })
    }

    /// What of `command` another core runs for `actor` (D-59, D-72): an
    /// agent it shows or lists, or a start with no parent on a machine that
    /// has a core of its own, which is then that machine's work. `None`
    /// for a command this core runs itself: a start under a parent, a plan
    /// step's or a mission worker's stays this core's work on that machine.
    pub(super) fn core_delivery_forward(
        &self,
        actor: &Actor,
        command: &Command,
    ) -> Result<Option<(Forward, Option<String>)>, String> {
        use crate::coordination::Command as Agents;
        let Command::Agents { command } = command else {
            return Ok(None);
        };
        let (device, asked, start) = match command {
            Agents::Show { id } => match CoreAddress::parse(id) {
                None => return Ok(None),
                Some(address) => {
                    let address = address.map_err(|_| "agent_unavailable".to_owned())?;
                    (
                        address.device.to_owned(),
                        Agents::Show {
                            id: address.id.to_owned(),
                        },
                        false,
                    )
                }
            },
            Agents::List {
                live,
                machine: Some(machine),
            } if machine != self.node.as_str() => (
                machine.clone(),
                Agents::List {
                    live: *live,
                    machine: None,
                },
                false,
            ),
            Agents::Spawn {
                parent: None,
                machine: Some(machine),
                plan_node: None,
                beside: false,
                ..
            } if !actor.code_owned()
                && actor.device_id == self.node.as_str()
                && self.missions.book.conducted_open(actor).is_none()
                && self
                    .device_registration(machine)
                    .is_some_and(|registration| registration.origin.peer_key().is_some()) =>
            {
                let mut there = command.clone();
                if let Agents::Spawn { machine, .. } = &mut there {
                    *machine = None;
                }
                (machine.clone(), there, true)
            }
            _ => return Ok(None),
        };
        let forward = self.core_forward(
            &device,
            actor,
            Asked::Delivery(Box::new(Command::Agents { command: asked })),
        )?;
        // A start is remembered by the connection it was asked on.
        let start = start.then(|| forward.key.clone());
        Ok(Some((forward, start)))
    }

    /// What of a pane command another core runs for the agent in
    /// `caller_pane`: a read of a pane or an agent it names as that core's
    /// (`core:<device>:pane|agent:<id>`). Typing into one is refused: a
    /// machine asks another's agents by letter.
    pub(super) fn core_pane_forward(
        &self,
        caller_pane: &str,
        command: &crate::pane_command::PaneCommand,
    ) -> Result<Option<Forward>, String> {
        use crate::pane_command::PaneCommand;
        let named = match command {
            PaneCommand::Read { pane, .. }
            | PaneCommand::ReadScreen { pane }
            | PaneCommand::Send { pane, .. } => pane,
            PaneCommand::AgentRead { agent, .. } => agent,
            PaneCommand::Open { .. } | PaneCommand::Purpose { .. } => return Ok(None),
        };
        let Some(address) = CoreAddress::parse(named) else {
            return Ok(None);
        };
        let address = address.map_err(|_| "pane_unavailable".to_owned())?;
        let there = match command {
            PaneCommand::Read { lines, .. } if address.item == CoreItem::Pane => {
                PaneCommand::Read {
                    pane: address.id.to_owned(),
                    lines: *lines,
                }
            }
            PaneCommand::ReadScreen { .. } if address.item == CoreItem::Pane => {
                PaneCommand::ReadScreen {
                    pane: address.id.to_owned(),
                }
            }
            // An agent named where a pane is read is read as the agent.
            PaneCommand::Read { lines, .. } | PaneCommand::AgentRead { lines, .. } => {
                PaneCommand::AgentRead {
                    agent: address.id.to_owned(),
                    lines: *lines,
                }
            }
            PaneCommand::ReadScreen { .. } => return Err("pane_unavailable".into()),
            _ => return Err(MACHINE_NOT_PERMITTED.into()),
        };
        let actor = self
            .delivery_observations
            .get(caller_pane)
            .ok_or("agent_pane_required")?
            .actor
            .clone();
        self.core_forward(address.device, &actor, Asked::Pane(there))
            .map(Some)
    }

    /// Tells each sender whose letter another core refused, once the store
    /// ended the letter undelivered. Run by the coordinator's tick; memory
    /// only.
    pub(crate) fn drive_core_refusals(&mut self) {
        if self.core_letters.refused.is_empty() || self.effects_held() {
            return;
        }
        let Ok(ledger) = self.delivery_ledger.as_ref().map(Arc::clone) else {
            return;
        };
        self.core_letters.refused.retain(|id, refused| {
            ledger
                .letters
                .iter()
                .find(|letter| &letter.id == id)
                .is_some_and(|letter| {
                    letter.state == State::Pending
                        || (letter.state == State::Undelivered && !refused.noted)
                })
        });
        if self.core_letters.noting {
            return;
        }
        let due: Vec<(String, &crate::delivery::ledger::Letter)> = self
            .core_letters
            .refused
            .iter_mut()
            .filter(|(_, refused)| !refused.noted)
            .filter_map(|(id, refused)| {
                let letter = ledger.letters.iter().find(|letter| &letter.id == id)?;
                (letter.state == State::Undelivered).then(|| {
                    refused.noted = true;
                    (refused.reason.clone(), letter)
                })
            })
            .collect();
        let mut notes = Vec::new();
        for (reason, letter) in due {
            let target = self
                .delivery_observations
                .get(&letter.sender.pane_id)
                .filter(|observation| observation.actor.same_identity(&letter.sender))
                .cloned()
                .or_else(|| {
                    self.asleep_route(&letter.sender).map(|_| {
                        super::agent_sleep::letters::asleep_observation(letter.sender.clone())
                    })
                });
            let Some(target) = target else {
                crate::diagnostic!(json!({
                    "component": "delivery", "kind": "core_letter.refusal_note_unsent",
                    "letter_id": letter.id, "code": "sender_unavailable",
                }));
                continue;
            };
            notes.push(Note {
                body: note_body(&letter.recipient.name, &letter.kind, &letter.id, &reason),
                letter: letter.id.clone(),
                target,
            });
        }
        if notes.is_empty() {
            return;
        }
        let kind = NoteKind {
            role: super::unreadable_letters::DELIVERY_ROLE,
            thread: "hide-core-letter-notes",
            intent: "letter-undelivered",
            component: "delivery",
            unsent: "core_letter.refusal_note_unsent",
            now: true,
        };
        self.core_letters.noting = self.send_hide_notes(kind, notes, |runtime| {
            runtime.core_letters.noting = false;
        });
    }
}

/// What Hide tells a sender whose letter another machine's core did not
/// take: who, why in plain words, and what to do.
fn note_body(recipient: &str, kind: &str, letter: &str, reason: &str) -> String {
    let why = match reason {
        "capacity" => "that machine holds as many letters as it can",
        "intent_conflict" => "that machine already holds another letter of yours under this intent",
        MACHINE_NOT_PERMITTED => "that machine does not let this one write to its agents first",
        "target_unavailable" | "target_ambiguous" | "target_identity_changed" => {
            "that machine found no single agent by that name"
        }
        "letter_unavailable" => "the letter it answers is no longer kept",
        "update_needed" => "that machine's Hide is too old to take it",
        _ => "that machine refused it",
    };
    format!(
        "Your {kind} {letter} to {recipient} was not delivered: {why} ({reason}). It ended undelivered and will not be sent again. Check the agent with `hide agent show`, then send again under a new intent if it still matters."
    )
}
