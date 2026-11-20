//! What a heal says after a cut (PRD letters-missions-simplify D-06, D-18,
//! D-19, B16 to B19): the one line a resumed agent gets to go on, and what
//! its parent or Conductor hears of the heal.
//!
//! An engine switch, a reboot, a lost holder and a device's reboot or link
//! return all end in the one heal: the engine resumes every agent whose
//! pane was cut and names each pane healing from before its resume is typed
//! until its hook answers (`ProjectionState::healing`). The core settles
//! what it owes when it first reads a pane so named: the registration
//! running there, whether it was at work when its engine went (what the core
//! holds of the pane's last turn, `HookFacts::cut_pending`, which covers a
//! device's panes as much as this machine's), and its parent. Nothing waits
//! for the resumed agent's first report, which can be lost.
//!
//! Only a registration that was at work and is a mission's participant or
//! another agent's child is owed the line; an agent at rest is only
//! reopened, and the operator's own agents are never typed into. The line is
//! the same whatever cut the work, in the operator's language. Each parent
//! hears once per heal how many of its children came back
//! (`Body::Resumed`), once each was seen in its pane or could not be woken;
//! a child its engine could not resume is said by name (`Body::NotWoken`),
//! and a child whose registration ended before its line could go is said too
//! (`Body::NotTold`). A mission worker the engine could not resume is its
//! Conductor's `worker_ended` raise already, so it is not said twice.
//!
//! Every line is owed to a registration, never to a pane, so a line never
//! reaches another agent that took the pane; it stays owed until the ledger
//! admits it, so a send that fails, a ledger that is full and a thread that
//! does not start leave it to the next tick, sent again under the same
//! intent, which the ledger takes once. The intent names the heal it belongs
//! to (the moment it was owed), so a second cut of the same conversation is
//! a new line. What is owed is memory only, bounded per machine by
//! [`OWED_PER_MACHINE`]: a new line past it is refused in the log and the
//! older ones stay.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::json;

use super::Runtime;
use super::delivery::Observation;
use super::hide_notes::{Note, NoteKind};
use crate::coordination::AgentRecord;
use crate::model::InterfaceLanguage;

/// The Hide role that tells a healed agent to go on, and its parent.
pub(crate) const HEAL_ROLE: &str = "heal";

/// The most lines owed at once about one machine's agents: for each pane a
/// node holds, the line to go on, the line its parent may be owed about it,
/// and the count its own children may owe it.
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
    /// The lines owed, by their key (the intent after `heal:`), until the
    /// ledger admits them.
    owed: BTreeMap<String, Owed>,
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
    /// The machine of the agent the line is about: what the cap counts.
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
    /// What the heal owes for it is settled; not while the ledger is not
    /// read yet.
    settled: bool,
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
    /// Keeps the panes `device`'s engine named healing in its last read,
    /// each from when the core first saw it so, and settles what the heal
    /// owes for each pane named for the first time ([`Self::heal_began`]).
    pub(crate) fn note_healing(&mut self, device: &str, panes: &[String], now_unix_ms: u64) {
        let mut was = self.heal_letters.healing.remove(device).unwrap_or_default();
        let mut named = BTreeMap::new();
        for pane in panes {
            let mut healing = was.remove(pane).unwrap_or(Healing {
                since: now_unix_ms,
                given_up: false,
                settled: false,
            });
            if !healing.settled {
                healing.settled = self.heal_began(device, pane, now_unix_ms);
            }
            named.insert(pane.clone(), healing);
        }
        if !named.is_empty() {
            self.heal_letters.healing.insert(device.to_owned(), named);
        }
        self.give_up_healing(now_unix_ms);
    }

    /// The engine on `device` began healing `pane` (its own id): the
    /// registration running there joins its parent's count, and is owed the
    /// line when it was at work and is a child or a participant. Answers
    /// false, to be asked again on the next read, while the ledger is not
    /// read yet.
    fn heal_began(&mut self, device: &str, pane: &str, now_unix_ms: u64) -> bool {
        let named = crate::address::named_pane_id(self.node.as_str(), device, pane);
        let Ok(ledger) = self.delivery_ledger.clone() else {
            return false;
        };
        let node = self.node.as_str().to_owned();
        let Some(record) = ledger
            .agents
            .iter()
            .find(|record| !record.ended && record.pane_on(&node) == named)
        else {
            return true;
        };
        let facts = self.hook_facts.clone();
        let facts = || {
            facts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        };
        let cut = facts().cut_pending(&named);
        crate::diagnostic!(json!({
            "component": "heal", "kind": "heal.began",
            "pane_id": named, "agent_id": record.id, "cut": cut,
        }));
        if let Some(parent) = &record.parent {
            self.heal_letters
                .resumed
                .entry(parent.clone())
                .or_insert_with(|| Resumed {
                    since: now_unix_ms,
                    children: BTreeMap::new(),
                })
                .children
                .insert(
                    record.id.clone(),
                    Child {
                        name: agent_name(record),
                        machine: device.to_owned(),
                        pane: pane.to_owned(),
                        woken: true,
                    },
                );
        }
        if !cut {
            return true;
        }
        if !self.participant_or_child(record) {
            facts().take_cut(&named);
            crate::diagnostic!(json!({
                "component": "heal", "kind": "heal_letter.operator_root", "pane_id": named,
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
        if self.owe_heal_line(format!("go-on:{}:{now_unix_ms}", record.id), owed) {
            facts().take_cut(&named);
        }
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
    /// parent's count, and its parent is told by name, once, unless it is a
    /// mission's worker, whose Conductor hears it as `worker_ended`.
    pub(in crate::runtime) fn heal_not_woken(&mut self, ended: &[String]) {
        let Ok(ledger) = self.delivery_ledger.clone() else {
            return;
        };
        for id in ended {
            let Some(record) = ledger.agents.iter().find(|record| &record.id == id) else {
                continue;
            };
            let Some(parent) = &record.parent else {
                continue;
            };
            if let Some(child) = self
                .heal_letters
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

    /// Owes `owed` under `key` and answers whether it is owed: a key owed
    /// already stays as it is; a machine at its cap keeps the lines it owes
    /// and refuses the new one.
    fn owe_heal_line(&mut self, key: String, owed: Owed) -> bool {
        let letters = &mut self.heal_letters;
        if letters.owed.contains_key(&key) {
            return true;
        }
        let on_machine = letters
            .owed
            .values()
            .filter(|other| other.machine == owed.machine)
            .count();
        if on_machine >= OWED_PER_MACHINE {
            crate::diagnostic!(json!({
                "component": "heal", "kind": "heal_letter.refused",
                "key": key, "machine": owed.machine, "owed": on_machine,
            }));
            return false;
        }
        letters.owed.insert(key, owed);
        true
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
        self.still_healing(&record.machine, &record.pane)
            || self
                .heal_letters
                .owed
                .values()
                .any(|owed| matches!(owed.body, Body::GoOn { .. }) && owed.to == record.id)
    }

    /// Sends each owed line whose agent can be reached
    /// ([`Self::heal_notes_due`]). Run by the coordinator's tick; a tick with
    /// nothing owed or counted is one test.
    pub(crate) fn drive_heal_letters(&mut self, now_unix_ms: u64) {
        self.give_up_healing(now_unix_ms);
        let letters = &self.heal_letters;
        if (letters.owed.is_empty() && letters.resumed.is_empty())
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

    /// The notes to send now: each parent's count once each of its children
    /// was seen in its pane, could not be woken or is healing no more, and
    /// each owed line whose registration can be reached. A line to go on
    /// whose registration ended becomes its parent's line that it could not
    /// go; a line to a parent whose registration ended is given up in the
    /// log, as is a line to go on for an agent no longer a child or a
    /// participant. A line stays owed until the ledger admits it.
    pub(in crate::runtime) fn heal_notes_due(&mut self) -> Vec<Note> {
        let settled: Vec<String> = self
            .heal_letters
            .resumed
            .iter()
            .filter(|(_, resumed)| {
                resumed.children.iter().all(|(id, child)| {
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
            let machine = self.registration(&parent).map_or_else(
                || self.node.as_str().to_owned(),
                |record| record.machine.clone(),
            );
            let owed = Owed {
                machine,
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

    /// Whether a thread is sending lines.
    pub(in crate::runtime) fn heal_letters_sending(&self) -> bool {
        !self.heal_letters.sending.is_empty()
    }
}
