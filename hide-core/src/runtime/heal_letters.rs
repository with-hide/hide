//! What a heal says after a cut (PRD letters-missions-simplify D-06, D-18,
//! D-19, B16 to B19): the one line a resumed agent gets to go on, and what
//! its parent or Conductor hears of the heal.
//!
//! An engine switch, a reboot, a lost holder and a device's reboot or link
//! return all end in the one heal: the engine resumes every agent whose
//! pane was cut, and says each resume once, before it types it
//! (`Engine::take_resumed`). The core takes those in order, in the
//! session-sync pass or, first, in the report of an agent of that engine,
//! and settles what the heal owes from that one event
//! ([`Runtime::heal_resumed`]): the registration saved in the pane, whether
//! it was at work when its engine went (what the core holds of the pane's
//! last turn, `HookFacts`, which covers a device's panes as much as this
//! machine's), and its parent. Nothing waits for the resumed agent's own
//! report, which can come first, late or never.
//!
//! Only a registration that was at work and is a mission's participant or
//! another agent's child is owed the line; an agent at rest is only
//! reopened, and the operator's own agents are never typed into. The line is
//! the same whatever cut the work, in the operator's language. Each parent
//! hears once per heal how many of its children came back
//! (`Body::Resumed`), once each was seen in its pane, could not be woken or
//! is healing no more; a child its engine could not resume is said by name
//! (`Body::NotWoken`), and a child whose registration ended before its line
//! could go is said too (`Body::NotTold`). A mission worker the engine could
//! not resume is its Conductor's `worker_ended` raise already, so it is not
//! said twice.
//!
//! Every line is owed to a registration, never to a pane, so a line never
//! reaches another agent that took the pane; it stays owed until the ledger
//! admits it, so a send that fails, a ledger that is full and a thread that
//! does not start leave it to the next tick, sent again under the same
//! intent, which the ledger takes once. The intent names the heal it belongs
//! to (the moment it was owed), so a second cut of the same conversation is
//! a new line. What is owed is memory only, bounded per machine by
//! [`OWED_PER_MACHINE`]: a line past it waits, as does a parent's count, and
//! is owed on a later tick once a line of that machine was admitted; only a
//! line past the wait's own bound as well is refused in the log.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::json;

use super::Runtime;
use super::delivery::Observation;
use super::hide_notes::{Note, NoteKind};
use crate::coordination::AgentRecord;
use crate::engine::ResumedPane;
use crate::model::InterfaceLanguage;

/// The Hide role that tells a healed agent to go on, and its parent.
pub(crate) const HEAL_ROLE: &str = "heal";

/// The most lines owed at once about one machine's agents, and the most
/// waiting for room: for each pane a node holds, the line to go on, the
/// line its parent may be owed about it, and the count its own children may
/// owe it.
const OWED_PER_MACHINE: usize = 3 * hide_kit::panes::HOLDER_CAP;

/// How long the core reads a pane its engine names healing as healing,
/// for a mission node's warnings and a parent's count: a resumed agent's
/// answer can be lost (a hook that cannot reach a daemon still starting
/// gives up), and an agent at rest says nothing more.
const HEALING_FOR: u64 = 10 * 60 * 1000;

/// The most children a count names; the rest are only counted.
const NAMES_SAID: usize = 8;

#[derive(Default)]
pub(in crate::runtime) struct HealLetters {
    /// The resumes taken while the ledger could not be read, by machine,
    /// settled on a later tick. Bounded by the panes each engine resumes.
    unread: Vec<(String, ResumedPane, u64)>,
    /// The lines owed, by their key (the intent after `heal:`), until the
    /// ledger admits them.
    owed: BTreeMap<String, Owed>,
    /// The lines a machine had no room for yet, oldest first, owed on a
    /// later tick.
    waiting: Vec<(String, Owed)>,
    /// The children each parent's count waits on, by the parent's
    /// registration id.
    resumed: BTreeMap<String, Resumed>,
    /// The keys a thread is sending now; it is the only one.
    sending: BTreeSet<String>,
    /// The panes each machine's engine names healing, by the machine's own
    /// pane ids, from the core's first read of each.
    healing: BTreeMap<String, BTreeMap<String, Healing>>,
}

struct Owed {
    /// The machine of the agent the line is about: what the bound counts.
    machine: String,
    /// The registration the line goes to.
    to: String,
    body: Body,
}

enum Body {
    /// Go on with the work that was cut. Its parent is told when the
    /// registration ends before the line goes.
    GoOn {
        parent: Option<String>,
        name: String,
    },
    /// This many of your children came back, these by name.
    Resumed { count: usize, names: Vec<String> },
    /// This child could not be brought back.
    NotWoken(String),
    /// This child came back but ended before it could be told to go on.
    NotTold(String),
}

/// A parent's count: its children the heal is bringing back, by id.
struct Resumed {
    since: u64,
    machine: String,
    children: BTreeMap<String, Child>,
}

struct Child {
    name: String,
    machine: String,
    /// The pane by its machine's own id.
    pane: String,
    woken: bool,
}

/// A pane its engine names healing.
struct Healing {
    since: u64,
    given_up: bool,
}

impl Body {
    /// The line, in `language`. Its first line is what the operator sees
    /// when it wakes the agent.
    fn text(&self, language: InterfaceLanguage) -> String {
        use InterfaceLanguage::{English, Japanese, Korean, SimplifiedChinese};
        match (self, language) {
            (Self::GoOn { .. }, Korean) => "끊겼던 일을 이어서 하세요.".into(),
            (Self::GoOn { .. }, Japanese) => "中断された作業を続けてください。".into(),
            (Self::GoOn { .. }, SimplifiedChinese) => "请继续之前被中断的工作。".into(),
            (Self::GoOn { .. }, English) => "Carry on with the work that was cut off.".into(),
            (Self::Resumed { count, names }, language) => {
                let more = count.saturating_sub(names.len());
                let mut names = names.join(", ");
                if more > 0 {
                    names.push_str(&match language {
                        Korean => format!(" 외 {more}명"),
                        Japanese => format!(" ほか{more}件"),
                        SimplifiedChinese => format!(" 等另外 {more} 个"),
                        English => format!(" and {more} more"),
                    });
                }
                match language {
                    Korean => format!("되살림: {count}명 · {names}"),
                    Japanese => format!("再開: {count}件 · {names}"),
                    SimplifiedChinese => format!("已恢复：{count} 个 · {names}"),
                    English => format!("Resumed: {count} · {names}"),
                }
            }
            (Self::NotWoken(name), Korean) => format!("깨우지 못함: {name}"),
            (Self::NotWoken(name), Japanese) => format!("再開できませんでした: {name}"),
            (Self::NotWoken(name), SimplifiedChinese) => format!("未能恢复：{name}"),
            (Self::NotWoken(name), English) => format!("Could not wake {name}"),
            (Self::NotTold(name), Korean) => format!("이어서 하라고 전하지 못함: {name}"),
            (Self::NotTold(name), Japanese) => format!("続行を伝えられませんでした: {name}"),
            (Self::NotTold(name), SimplifiedChinese) => format!("未能告知继续：{name}"),
            (Self::NotTold(name), English) => format!("Could not tell {name} to carry on"),
        }
    }
}

impl Runtime {
    /// The resumes the engine on `device` said, each taken once: the
    /// registration saved in each pane joins its parent's count, and is
    /// owed the line when it was at work and is a child or a participant.
    /// A resume met while the ledger cannot be read waits for a tick that
    /// can read it.
    pub(crate) fn heal_resumed(&mut self, device: &str, resumed: &[ResumedPane], now_unix_ms: u64) {
        for resume in resumed {
            if !self.heal_one(device, resume, now_unix_ms) {
                self.heal_letters
                    .unread
                    .push((device.to_owned(), resume.clone(), now_unix_ms));
            }
        }
    }

    /// Settles one resume; false while the ledger cannot be read.
    fn heal_one(&mut self, device: &str, resume: &ResumedPane, now_unix_ms: u64) -> bool {
        let node = self.node.as_str().to_owned();
        let saved = crate::address::named_pane_id(&node, device, &resume.saved);
        let typed = crate::address::named_pane_id(&node, device, &resume.pane);
        let Ok(ledger) = self.delivery_ledger.clone() else {
            crate::diagnostic!(json!({
                "component": "heal", "kind": "heal.unread", "pane_id": typed,
            }));
            return false;
        };
        let Some(record) = ledger.agents.iter().find(|record| {
            !record.ended && (record.pane_on(&node) == saved || record.pane_on(&node) == typed)
        }) else {
            return true;
        };
        // The cut is read and spent here, once: a later report of the
        // resumed agent, which says no heal is behind it, must not undo it.
        let cut = self
            .hook_facts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take_cut(&saved);
        crate::diagnostic!(json!({
            "component": "heal", "kind": "heal.began",
            "pane_id": typed, "agent_id": record.id, "cut": cut,
        }));
        if let Some(parent) = &record.parent {
            let machine = ledger
                .agents
                .iter()
                .find(|other| !other.ended && &other.id == parent)
                .map_or_else(|| node.clone(), |other| other.machine.clone());
            self.heal_letters
                .resumed
                .entry(parent.clone())
                .or_insert_with(|| Resumed {
                    since: now_unix_ms,
                    machine,
                    children: BTreeMap::new(),
                })
                .children
                .insert(
                    record.id.clone(),
                    Child {
                        name: agent_name(record),
                        machine: device.to_owned(),
                        pane: resume.pane.clone(),
                        woken: true,
                    },
                );
        }
        if !cut {
            return true;
        }
        if !self.participant_or_child(record) {
            crate::diagnostic!(json!({
                "component": "heal", "kind": "heal_letter.operator_root", "pane_id": typed,
            }));
            return true;
        }
        let owed = Owed {
            machine: device.to_owned(),
            to: record.id.clone(),
            body: Body::GoOn {
                parent: record.parent.clone(),
                name: agent_name(record),
            },
        };
        self.owe_heal_line(format!("go-on:{}:{now_unix_ms}", record.id), owed);
        true
    }

    /// The agent in `caller`'s pane on `device` reported after its session
    /// started: `healed` when a heal resumed it there. One the engine did not
    /// heal was never cut, whatever the pane's last turn said, since its
    /// holder kept it running.
    pub(crate) fn hook_after_heal(&mut self, device: &str, caller: &str, healed: bool) {
        let pane = crate::address::named_pane_id(self.node.as_str(), device, caller);
        if healed {
            crate::diagnostic!(json!({
                "component": "heal", "kind": "heal.resumed", "pane_id": pane,
            }));
        } else {
            self.hook_facts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .forget_cut(&pane);
        }
    }

    /// The registrations the engine could not resume: each leaves its
    /// parent's count and its line to go on, and its parent is told by name,
    /// once, unless it is a mission's worker, whose Conductor hears it as
    /// `worker_ended`.
    pub(in crate::runtime) fn heal_not_woken(&mut self, ended: &[String]) {
        let Ok(ledger) = self.delivery_ledger.clone() else {
            return;
        };
        for id in ended {
            let Some(record) = ledger.agents.iter().find(|record| &record.id == id) else {
                continue;
            };
            let letters = &mut self.heal_letters;
            let goes_on = |owed: &Owed| matches!(owed.body, Body::GoOn { .. }) && &owed.to == id;
            letters.owed.retain(|_, owed| !goes_on(owed));
            letters.waiting.retain(|(_, owed)| !goes_on(owed));
            let Some(parent) = &record.parent else {
                continue;
            };
            if let Some(child) = letters
                .resumed
                .get_mut(parent)
                .and_then(|resumed| resumed.children.get_mut(id))
            {
                child.woken = false;
            }
            let worker = self.missions.book.missions.iter().any(|mission| {
                mission
                    .nodes
                    .iter()
                    .any(|node| node.agent_id.as_deref() == Some(id.as_str()))
            });
            if worker {
                continue;
            }
            self.owe_heal_line(
                format!("not-woken:{id}"),
                Owed {
                    machine: record.machine.clone(),
                    to: parent.clone(),
                    body: Body::NotWoken(agent_name(record)),
                },
            );
        }
    }

    /// Owes `owed` under `key`: a key owed or waiting already stays as it
    /// is; a machine at its bound keeps it waiting until a later tick finds
    /// room ([`Self::owe_waiting`]). Only a line past the wait's own bound
    /// is refused, in the log.
    fn owe_heal_line(&mut self, key: String, owed: Owed) {
        let letters = &mut self.heal_letters;
        if letters.owed.contains_key(&key) || letters.waiting.iter().any(|(at, _)| *at == key) {
            return;
        }
        if owed_on(&letters.owed, &owed.machine) < OWED_PER_MACHINE {
            letters.owed.insert(key, owed);
            return;
        }
        let waiting = letters
            .waiting
            .iter()
            .filter(|(_, other)| other.machine == owed.machine)
            .count();
        if waiting >= OWED_PER_MACHINE {
            crate::diagnostic!(json!({
                "component": "heal", "kind": "heal_letter.refused",
                "key": key, "machine": owed.machine, "waiting": waiting,
            }));
            return;
        }
        crate::diagnostic!(json!({
            "component": "heal", "kind": "heal_letter.waiting",
            "key": key, "machine": owed.machine,
        }));
        letters.waiting.push((key, owed));
    }

    /// Owes the waiting lines, oldest first, while their machines have room.
    fn owe_waiting(&mut self) {
        let letters = &mut self.heal_letters;
        let waiting = std::mem::take(&mut letters.waiting);
        for (key, owed) in waiting {
            if owed_on(&letters.owed, &owed.machine) < OWED_PER_MACHINE {
                letters.owed.insert(key, owed);
            } else {
                letters.waiting.push((key, owed));
            }
        }
    }

    /// Keeps the panes `device`'s engine named healing in its last read,
    /// each from when the core first saw it so: what holds a mission node's
    /// warnings and a parent's count, never what is owed.
    pub(crate) fn note_healing(&mut self, device: &str, panes: &[String], now_unix_ms: u64) {
        let mut was = self.heal_letters.healing.remove(device).unwrap_or_default();
        let named: BTreeMap<String, Healing> = panes
            .iter()
            .map(|pane| {
                let healing = was.remove(pane).unwrap_or(Healing {
                    since: now_unix_ms,
                    given_up: false,
                });
                (pane.clone(), healing)
            })
            .collect();
        if !named.is_empty() {
            self.heal_letters.healing.insert(device.to_owned(), named);
        }
        self.give_up_healing(now_unix_ms);
    }

    /// Reads a pane named healing past [`HEALING_FOR`] as healing no more.
    fn give_up_healing(&mut self, now_unix_ms: u64) {
        for (device, panes) in &mut self.heal_letters.healing {
            for (pane, healing) in panes.iter_mut() {
                if !healing.given_up && now_unix_ms.saturating_sub(healing.since) > HEALING_FOR {
                    healing.given_up = true;
                    crate::diagnostic!(json!({
                        "component": "heal", "kind": "heal.unconfirmed",
                        "device": device, "pane_id": pane,
                    }));
                }
            }
        }
    }

    /// Whether `device`'s engine names `pane` (its own id) healing and the
    /// core still reads it so.
    fn still_healing(&self, device: &str, pane: &str) -> bool {
        self.heal_letters
            .healing
            .get(device)
            .and_then(|panes| panes.get(pane))
            .is_some_and(|healing| !healing.given_up)
    }

    /// Whether the agent `record` names is being healed: its engine has yet
    /// to bring it back, or it is still owed its line. Nothing it does or
    /// leaves undone in that time is its own (B19).
    pub(crate) fn healing(&self, record: &AgentRecord) -> bool {
        let goes_on = |owed: &Owed| matches!(owed.body, Body::GoOn { .. }) && owed.to == record.id;
        self.still_healing(&record.machine, &record.pane)
            || self.heal_letters.owed.values().any(goes_on)
            || self
                .heal_letters
                .waiting
                .iter()
                .any(|(_, owed)| goes_on(owed))
    }

    /// Sends each owed line whose agent can be reached
    /// ([`Self::heal_notes_due`]). Run by the coordinator's tick; a tick with
    /// nothing unread, owed, waiting or counted is one test.
    pub(crate) fn drive_heal_letters(&mut self, now_unix_ms: u64) {
        self.give_up_healing(now_unix_ms);
        if self.delivery_ledger.is_ok() && !self.heal_letters.unread.is_empty() {
            for (device, resume, at) in std::mem::take(&mut self.heal_letters.unread) {
                self.heal_resumed(&device, std::slice::from_ref(&resume), at);
            }
        }
        let letters = &self.heal_letters;
        if (letters.owed.is_empty() && letters.waiting.is_empty() && letters.resumed.is_empty())
            || !letters.sending.is_empty()
            || self.effects_held()
        {
            return;
        }
        let notes = self.heal_notes_due();
        if notes.is_empty() {
            return;
        }
        let keys: BTreeSet<String> = notes.iter().map(|note| note.letter.clone()).collect();
        let kind = NoteKind {
            role: HEAL_ROLE,
            thread: "hide-heal-letters",
            intent: "heal",
            component: "heal",
            unsent: "heal_letter.unsent",
            now: false,
        };
        if self.send_hide_notes(kind, notes, |runtime, admitted| {
            runtime.heal_lines_admitted(admitted);
            runtime.heal_letters.sending.clear();
        }) {
            self.heal_letters.sending = keys;
        }
    }

    /// Settles the lines the ledger admitted.
    pub(in crate::runtime) fn heal_lines_admitted(&mut self, admitted: &[String]) {
        for key in admitted {
            self.heal_letters.owed.remove(key);
        }
    }

    /// The notes to send now. The waiting lines are owed first, as room
    /// allows; each parent's count is owed once each of its children was
    /// seen in its pane, could not be woken or is healing no more, and
    /// stays counted until its machine has room for its line. A line to go
    /// on whose registration ended becomes its parent's line that it could
    /// not go; a line to a parent whose registration ended is given up in
    /// the log, as is a line to go on for an agent no longer a child or a
    /// participant. A line stays owed until the ledger admits it.
    pub(in crate::runtime) fn heal_notes_due(&mut self) -> Vec<Note> {
        self.owe_waiting();
        let settled: Vec<String> = self
            .heal_letters
            .resumed
            .iter()
            .filter(|(_, resumed)| {
                owed_on(&self.heal_letters.owed, &resumed.machine) < OWED_PER_MACHINE
                    && resumed.children.iter().all(|(id, child)| {
                        !child.woken
                            || !self.still_healing(&child.machine, &child.pane)
                            || self.observed_registration(id).is_some()
                    })
            })
            .map(|(parent, _)| parent.clone())
            .collect();
        for parent in settled {
            let Some(resumed) = self.heal_letters.resumed.remove(&parent) else {
                continue;
            };
            // A child that ended since is not one that came back.
            let names: Vec<String> = resumed
                .children
                .into_iter()
                .filter(|(id, child)| child.woken && self.registration(id).is_some())
                .map(|(_, child)| child.name)
                .collect();
            if names.is_empty() {
                continue;
            }
            let owed = Owed {
                machine: resumed.machine,
                to: parent.clone(),
                body: Body::Resumed {
                    count: names.len(),
                    names: names.into_iter().take(NAMES_SAID).collect(),
                },
            };
            self.owe_heal_line(format!("resumed:{parent}:{}", resumed.since), owed);
        }

        let unreachable: Vec<String> = self
            .heal_letters
            .owed
            .iter()
            .filter(|(_, owed)| {
                self.registration(&owed.to).is_none_or(|record| {
                    matches!(owed.body, Body::GoOn { .. }) && !self.participant_or_child(record)
                })
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in unreachable {
            let Some(owed) = self.heal_letters.owed.remove(&key) else {
                continue;
            };
            let ended = self.registration(&owed.to).is_none();
            crate::diagnostic!(json!({
                "component": "heal", "kind": "heal_letter.unaddressed",
                "key": key, "ended": ended,
            }));
            if let (
                true,
                Body::GoOn {
                    parent: Some(parent),
                    name,
                },
            ) = (ended, owed.body)
            {
                let key = format!("not-told:{}", key.trim_start_matches("go-on:"));
                let owed = Owed {
                    machine: owed.machine,
                    to: parent,
                    body: Body::NotTold(name),
                };
                self.owe_heal_line(key, owed);
            }
        }

        let language = self.interface_language();
        self.heal_letters
            .owed
            .iter()
            .filter_map(|(key, owed)| {
                let target = match owed.body {
                    // The line to go on waits for the agent in its pane.
                    Body::GoOn { .. } => self.observed_registration(&owed.to),
                    _ => self.registered_target(&owed.to),
                }?;
                Some(Note {
                    letter: key.clone(),
                    target,
                    body: owed.body.text(language),
                })
            })
            .collect()
    }

    /// The live registration `id`.
    fn registration(&self, id: &str) -> Option<&AgentRecord> {
        self.delivery_ledger
            .as_ref()
            .ok()?
            .agents
            .iter()
            .find(|record| !record.ended && record.id == id)
    }

    /// The agent registered as `id`, observed in its pane.
    fn observed_registration(&self, id: &str) -> Option<Observation> {
        let record = self.registration(id)?;
        let observed = self
            .delivery_observations
            .get(&record.pane_on(self.node.as_str()))?;
        observed
            .actor
            .same_identity(&record.actor)
            .then(|| observed.clone())
    }

    /// Where a line to the agent registered as `id` goes: the agent observed
    /// in its pane, else that pane's agent asleep, whose mailbox keeps it
    /// for its next wake.
    fn registered_target(&self, id: &str) -> Option<Observation> {
        if let Some(observed) = self.observed_registration(id) {
            return Some(observed);
        }
        let record = self.registration(id)?;
        let ledger = self.delivery_ledger.as_ref().ok()?;
        self.sleeping_target(
            ledger,
            Some(&record.actor),
            &record.pane_on(self.node.as_str()),
        )
    }

    /// Whether the agent `record` registers is another agent's child or the
    /// Conductor of an open mission (a worker or checker is the Conductor's
    /// child): the ones a heal tells to go on (D-19).
    fn participant_or_child(&self, record: &AgentRecord) -> bool {
        record.parent.is_some()
            || self.missions.book.missions.iter().any(|mission| {
                mission.status.open()
                    && (mission.conductor.agent_id.as_deref() == Some(record.id.as_str())
                        || mission.conductor.actor.same_identity(&record.actor))
            })
    }
}

/// How many lines are owed about `machine`'s agents.
fn owed_on(owed: &BTreeMap<String, Owed>, machine: &str) -> usize {
    owed.values().filter(|owed| owed.machine == machine).count()
}

/// The name a parent knows its child by.
fn agent_name(record: &AgentRecord) -> String {
    record.name.clone().unwrap_or_else(|| record.id.clone())
}

#[cfg(test)]
impl Runtime {
    /// The keys owed, in order.
    pub(in crate::runtime) fn heal_letters_owed(&self) -> Vec<String> {
        self.heal_letters.owed.keys().cloned().collect()
    }

    /// The keys waiting for room, oldest first.
    pub(in crate::runtime) fn heal_letters_waiting(&self) -> Vec<String> {
        self.heal_letters
            .waiting
            .iter()
            .map(|(key, _)| key.clone())
            .collect()
    }

    /// Whether a thread is sending lines.
    pub(in crate::runtime) fn heal_letters_sending(&self) -> bool {
        !self.heal_letters.sending.is_empty()
    }
}
