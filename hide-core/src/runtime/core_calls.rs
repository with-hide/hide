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
//! A letter is between agents, never panes (PRD letters-missions-simplify
//! D-21): one of this core's agents writes to `core:<device>:agent:<id>`,
//! and a sender Hide holds no registration for is refused. The letter is an
//! ordinary ledger letter whose recipient is that identity. It stays
//! `pending`, with no deadline, until the other core answers: taken, it is
//! delivered; refused, it ends `no_recipient` with the other core's reason
//! (`Ledger::refuse_core`) and its sender's cell about it says why. A reply
//! names the letter it answers by the id its own core gave it
//! (`Letter::core_id`), and is run here as the recipient of that letter.
//!
//! Nothing of a letter is kept here: the ledger holds it, and what another
//! core answers is written there at once.

use std::sync::Arc;

use serde_json::json;

use super::Runtime;
use super::delivery::Observation;
use crate::access::{self, CoreAsk, MACHINE_NOT_PERMITTED};
use crate::address::CoreAddress;
use crate::coordination::AgentRecord;
use crate::core_calls::{
    Agent, Asked, Call, CoreItem, CoreLink, Forward, Keeping, Letter, NO_RECIPIENT, Outbox,
    Outcome, Party, Settle, Taking,
};
use crate::delivery::ledger::{Ledger, State};
use crate::delivery::worker::{Effect, Proven, Start};
use crate::delivery::{Actor, Command};
use crate::peer_machines::PeerMachine;
use crate::workspace_control::Context;

/// The other machine cannot be asked now: it is not connected.
const MACHINE_UNAVAILABLE: &str = "machine_unavailable";

/// Two paired machines say the device id a `core:` name carries.
const DEVICE_AMBIGUOUS: &str = "device_ambiguous";

/// A letter was addressed to a pane of another machine: a letter goes to an
/// agent (D-21).
const RECIPIENT_NOT_AGENT: &str = "recipient_not_agent";

/// The agent that writes to, or asks a start of, another machine is not one
/// Hide knows: it has no registration, and the core cannot make it one.
pub(crate) const SENDER_NOT_REGISTERED: &str = "sender_not_registered";

#[derive(Default)]
pub(in crate::runtime) struct CoreLetters {
    /// The connection to the other cores; none in a core no daemon gave one.
    link: Option<Arc<dyn CoreLink>>,
    /// The daemon told the paired machines at least once, so a machine not
    /// among them is one this core is paired with no longer.
    told: bool,
    /// The letters waiting to go to another core at the last publish.
    outbound: Vec<String>,
}

/// The registration `record` as its core names it to another.
fn agent_of(record: &AgentRecord) -> Agent {
    Agent {
        id: record.id.clone(),
        name: record.wire_name().to_owned(),
    }
}

/// `actor` as its own core names it to another when it asks a read: by
/// its registration, ended or not, while the ledger keeps one, and by its
/// pane otherwise.
fn core_party(ledger: &Ledger, actor: &Actor) -> Party {
    let record = ledger
        .agents
        .iter()
        .rev()
        .filter(|record| record.actor.same_identity(actor))
        .min_by_key(|record| record.ended);
    match record {
        Some(record) => agent_of(record).into(),
        None => Party {
            item: CoreItem::Pane,
            id: actor.pane_id.clone(),
            name: actor.name.clone(),
        },
    }
}

/// The agent that runs now in the place of the registration `id` names, by
/// its id or the name it registered under: itself, or the last agent that
/// took its pane over. None when no agent runs there now, or for an
/// identity Hide owns, which another machine writes nothing to.
fn agent_now<'a>(ledger: &'a Ledger, id: &str) -> Option<&'a AgentRecord> {
    crate::coordination::registered_record(ledger, id)
        .or_else(|| crate::coordination::live_successor(ledger, id)?.ok())
        .filter(|record| !record.actor.code_owned())
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
                        // To the agent that asked, as it runs now.
                        agent_now(ledger, target).is_some_and(|to| {
                            ledger
                                .core_starts
                                .iter()
                                .any(|start| start.key == machine.key && start.origin == to.id)
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

    /// Who a send that names another core's agent goes to: that agent as
    /// this core knows it, with no pane to observe. It is named by its
    /// address (`core:<device>:agent:<id>`), read on the one paired machine
    /// that says the device id, or by the exact id of the registration this
    /// ledger keeps of it, which is the agent on the connection it was
    /// registered on whatever says its device id now. A pane of another
    /// machine is no recipient, and neither is a sender Hide holds no
    /// registration for and cannot make one (D-21). `None` for any other
    /// target.
    pub(super) fn core_send_target(
        &self,
        sender: &Actor,
        command: &Command,
    ) -> Result<Option<Observation>, String> {
        let Command::Send { target, force, .. } = command else {
            return Ok(None);
        };
        let recipient = match CoreAddress::parse(target) {
            Some(address) => {
                let address = address.map_err(|_| "target_unavailable".to_owned())?;
                if address.item != CoreItem::Agent {
                    return Err(RECIPIENT_NOT_AGENT.into());
                }
                let machine = self.peer_on(address.device)?;
                Actor::core(
                    address,
                    &machine.key,
                    &machine.name,
                    address.id,
                    self.node.as_str(),
                )
            }
            None => {
                let kept = self.delivery_ledger.as_ref().ok().and_then(|ledger| {
                    ledger
                        .agents
                        .iter()
                        .find(|record| record.id == *target)
                        .filter(|record| record.actor.core_address().is_some())
                });
                match kept {
                    Some(record) => record.actor.clone(),
                    None => return Ok(None),
                }
            }
        };
        // Its machine is read from the recipient's own connection here, so
        // a kept registration whose connection is paired no longer is
        // refused and never turns to another that says its device id.
        self.core_send_admitted(sender, &recipient, command)?;
        self.core_asker_registration(sender)?;
        if *force {
            // No composer of another core's pane is read from here.
            return Err("force_target_asleep".into());
        }
        Ok(Some(super::agent_sleep::letters::asleep_observation(
            recipient,
        )))
    }

    /// The registration `actor` is made with before it writes to, or asks
    /// a start of, another machine, when it has none; refused when Hide
    /// holds none for it and reads none from its pane, since the other
    /// machine answers an agent.
    fn core_asker_registration(&self, actor: &Actor) -> Result<Option<AgentRecord>, String> {
        let ledger = self.delivery_ledger.as_ref().map_err(String::clone)?;
        if crate::coordination::registered_id(ledger, actor).is_some() {
            return Ok(None);
        }
        self.party_registration(ledger, actor)
            .map(Some)
            .ok_or_else(|| SENDER_NOT_REGISTERED.to_owned())
    }

    /// The letters waiting to go to machine `key`, as its core takes them,
    /// and the ends of those that cannot go: a reply whose original this
    /// ledger no longer names, and a letter whose sender has no
    /// registration.
    pub(crate) fn core_outbox(&mut self, key: &str) -> Outbox {
        let mut outbox = Outbox::default();
        if self.peer_keyed(key).is_err() {
            return outbox;
        }
        let Ok(ledger) = self.delivery_state() else {
            return outbox;
        };
        for letter in &ledger.letters {
            // Only what was written to an agent of this connection.
            let Some(address) = letter
                .recipient
                .core_address()
                .filter(|_| letter.state == State::Pending && letter.recipient.core_of(key))
            else {
                continue;
            };
            let worded = (|| {
                let from = letter
                    .from
                    .as_deref()
                    .and_then(|id| ledger.agents.iter().find(|record| record.id == id))
                    .map(agent_of)
                    .ok_or(SENDER_NOT_REGISTERED)?;
                // A reply names the letter it answers as its sender's core
                // does; one whose original this ledger no longer keeps
                // cannot say which letter it answers.
                let reply_to = match &letter.reply_to {
                    None => None,
                    Some(local) => Some(
                        ledger
                            .letters
                            .iter()
                            .find(|original| original.id == *local)
                            .and_then(|original| original.core_id.clone())
                            .ok_or("letter_unavailable")?,
                    ),
                };
                Ok::<_, &str>(Letter {
                    id: letter.id.clone(),
                    from,
                    to: address.id.to_owned(),
                    reply_to,
                    kind: letter.kind.clone(),
                    intent: letter.intent.clone(),
                    body: letter.body.clone(),
                    now: letter.immediate,
                })
            })();
            match worded {
                Ok(letter) => outbox.letters.push(letter),
                Err(reason) => outbox
                    .ended
                    .extend(self.core_letter_refused(letter, key, reason)),
            }
        }
        outbox
    }

    /// Letter `letter` to an agent of the core on connection `key` does not
    /// go, and why: its sender's cell about it says so, once, and the write
    /// that ends it is handed back (`Ledger::refuse_core`).
    fn core_letter_refused(
        &mut self,
        letter: &crate::delivery::ledger::Letter,
        key: &str,
        reason: &str,
    ) -> Option<Settle> {
        crate::diagnostic!(json!({
            "component": "delivery", "kind": "core_letter.refused",
            "letter_id": letter.id, "reason": reason,
        }));
        // News of a fact that does not change: the letter's own time is its
        // revision, so the same refusal found again is the same cell. A
        // cell refused for room is not owed again; the letter keeps the
        // reason for `hide inbox`.
        let _ = self.post_status(
            &letter.sender,
            super::status_cells::letter_subject(&letter.id),
            note_body(&letter.recipient.name, &letter.kind, &letter.id, reason),
            true,
            letter.created_at_unix_ms,
        );
        Some(Settle {
            client: self.delivery_client.clone()?,
            effect: Effect::CoreRefused {
                id: letter.id.clone(),
                key: key.to_owned(),
                reason: reason.to_owned(),
            },
        })
    }

    /// What machine `key`'s core answered for this core's letter `id`, as
    /// the write that delivers or ends it; nothing for a letter no longer
    /// on its way to that machine.
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
                let ledger = self.delivery_ledger.as_ref().map(Arc::clone).ok()?;
                // Only a letter still on its way to that machine.
                let letter = ledger.letters.iter().find(|letter| {
                    letter.id == id
                        && letter.state == State::Pending
                        && letter.recipient.core_of(&machine.key)
                })?;
                self.core_letter_refused(letter, &machine.key, &reason)
            }
        }
    }

    /// A letter machine `key`'s core sent for one of its agents, as the
    /// command that agent runs here, or the answer this core already has
    /// for it.
    pub(crate) fn prepare_core_letter(
        &mut self,
        key: &str,
        letter: Letter,
    ) -> Result<Taking, String> {
        let machine = self.peer_keyed(key)?.clone();
        let ledger = self.delivery_state()?;
        // A letter this core already keeps is answered as it stands, so one
        // asked again after its agent ended is not judged as a new letter.
        if let Some(kept) = ledger.letters.iter().find(|kept| {
            kept.core_id.as_deref() == Some(&letter.id) && kept.sender.core_of(&machine.key)
        }) {
            let recipient = kept
                .to
                .as_deref()
                .filter(|_| kept.state != State::NoRecipient)
                .and_then(|to| ledger.agents.iter().find(|record| record.id == to));
            return Ok(Taking::Kept(match recipient {
                Some(record) => Outcome::Taken {
                    recipient: agent_of(record),
                },
                None => Outcome::Refused {
                    reason: NO_RECIPIENT.into(),
                },
            }));
        }
        let (actor, command, recipient) = match letter.reply_to {
            Some(original) => {
                // It answers a letter this core sent that machine, whose
                // recipient is the agent that answers now. The answer goes
                // to the agent that runs in its sender's place.
                let stored = ledger
                    .letters
                    .iter()
                    .find(|stored| stored.id == original)
                    .filter(|stored| stored.recipient.core_of(&machine.key))
                    .ok_or("letter_unavailable")?;
                let sender = stored
                    .from
                    .as_deref()
                    .and_then(|from| crate::coordination::live_successor(&ledger, from))
                    .map(|line| match line {
                        Ok(record) | Err(record) => agent_of(record),
                    })
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
                (actor, command, sender)
            }
            None => {
                // A letter names an agent: a registration, or the agent
                // that took its pane over, never a pane.
                let to = agent_now(&ledger, &letter.to).ok_or("target_unavailable")?;
                let actor = self.core_actor(&machine, &letter.from.clone().into());
                let command = Command::Send {
                    target: to.id.clone(),
                    intent: letter.intent,
                    body: letter.body,
                    kind: letter.kind,
                    now: letter.now,
                    force: false,
                };
                (actor, command, agent_of(to))
            }
        };
        self.core_inbound_admit(&machine, &command)?;
        let context = self.core_context(&actor);
        let caller = Self::core_caller(&machine, &actor);
        let node = self.node.as_str().to_owned();
        let prepared = self.prepare_as(&node, &caller, context, Proven::Now, actor, command)?;
        Ok(Taking::Keep(Box::new(Keeping {
            prepared,
            client: self.delivery_client.clone().ok_or("delivery_unavailable")?,
            core_id: letter.id,
            recipient,
        })))
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
        // What starts here reports to its asker by letter, and a letter
        // goes to an agent.
        if from.item != CoreItem::Agent
            && matches!(
                &asked,
                Asked::Delivery(command) if matches!(
                    **command,
                    Command::Agents { command: crate::coordination::Command::Spawn { .. } }
                )
            )
        {
            return Err(SENDER_NOT_REGISTERED.into());
        }
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
    ) -> Result<Option<(Forward, Option<Start>)>, String> {
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
        // A start is remembered by the connection it was asked on, for the
        // agent its asker is: what starts there reports to it by letter.
        let start = match start {
            true => Some(Start {
                key: forward.key.clone(),
                registers: self.core_asker_registration(actor)?.map(Box::new),
            }),
            false => None,
        };
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
}

/// What Hide tells a sender whose letter did not reach another machine's
/// agent: who, why in plain words, and what to do.
fn note_body(recipient: &str, kind: &str, letter: &str, reason: &str) -> String {
    let why = match reason {
        "capacity" => "that machine holds as many letters as it can",
        MACHINE_NOT_PERMITTED => "that machine does not let this one write to its agents first",
        NO_RECIPIENT => "its agent ended and no agent took its place",
        "target_unavailable" | "target_ambiguous" | "target_identity_changed" => {
            "that machine found no single agent by that id or name"
        }
        "letter_unavailable" => "the letter it answers is no longer kept",
        SENDER_NOT_REGISTERED => "its sender is not an agent Hide knows",
        "update_needed" => "that machine's Hide is too old to take it",
        _ => "that machine refused it",
    };
    format!(
        "Your {kind} letter {letter} to {recipient} was not delivered: {why} ({reason}). It ended and will not be sent again.\nRead that machine's agents with `hide agent list --machine <device>`, then send again under a new --intent if it still matters."
    )
}
