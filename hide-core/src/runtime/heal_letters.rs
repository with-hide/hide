//! What a heal says after a cut (PRD letters-missions-simplify D-06, D-18,
//! D-19, B16 to B19): the one line a resumed agent gets to go on, and what
//! its parent or Conductor hears of the heal.
//!
//! An engine switch, a reboot, a lost holder and a device's reboot or link
//! return all end in the one heal: the engine resumes every agent whose
//! pane was cut, and says each resume once, before it types it
//! (`Engine::take_resumed`). The core takes those and settles them in one
//! step under its lock ([`Runtime::take_heal`]), in the session-sync pass
//! and, first, in the report of an agent of that engine, so a report meets
//! either a resume not taken yet or one settled: the registration saved in
//! the pane, whether
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
//! a new line. What is owed is memory only, one entry of each kind per
//! registration (a line to go on, a parent's count, a child said not woken
//! or not told), so it is bounded by the registrations, and an entry whose
//! recipient ended is dropped. A resume whose registration is still owed
//! the line of an earlier heal stays taken and unsettled, its cut unspent,
//! until that line is admitted; so does one met while the ledger cannot be
//! read.

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

/// How long the core reads a pane its engine names healing as healing,
/// for a mission node's warnings and a parent's count: a resumed agent's
/// answer can be lost (a hook that cannot reach a daemon still starting
/// gives up), and an agent at rest says nothing more.
const HEALING_FOR: u64 = 10 * 60 * 1000;

/// The most children a count names; the rest are only counted.
const NAMES_SAID: usize = 8;

#[derive(Default)]
pub(in crate::runtime) struct HealLetters {
    /// The resumes taken and not settled yet, by machine and when taken,
    /// oldest first; one per pane of a machine.
    pending: Vec<(String, ResumedPane, u64)>,
    /// The lines owed, one of each kind per registration (`go-on:<id>`,
    /// `resumed:<parent>`, `not-woken:<id>`, `not-told:<id>`), until the
    /// ledger admits them.
    owed: BTreeMap<String, Owed>,
    /// The children each parent's count waits on, by the parent's
    /// registration id.
    resumed: BTreeMap<String, Resumed>,
    /// The letters a thread is sending now; it is the only one.
    sending: BTreeSet<String>,
    /// The panes each machine's engine names healing, by the machine's own
    /// pane ids, from the core's first read of each.
    healing: BTreeMap<String, BTreeMap<String, Healing>>,
}

struct Owed {
    /// The letter's key, the intent after `heal:`: the entry's key and,
    /// for a heal's own lines, the moment it was owed, so a second heal is
    /// a new letter and a retry is the same.
    letter: String,
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
    /// Takes the resumes `engine` (the engine of `device`) said and settles
    /// them, in one step under the runtime lock, so an agent's report never
    /// falls between the two. Taking them is a move out of the engine's
    /// memory, never a wait on its thread.
    pub(crate) fn take_heal(
        &mut self,
        device: &str,
        engine: &dyn crate::engine::Engine,
        now_unix_ms: u64,
    ) {
        let taken = engine.take_resumed();
        self.heal_resumed(device, &taken, now_unix_ms);
    }

    /// Settles the resumes the engine on `device` said, oldest first: the
    /// registration saved in each pane joins its parent's count, and is
    /// owed the line when it was at work and is a child or a participant.
    /// One that cannot be settled yet stays pending ([`Self::heal_one`]).
    pub(in crate::runtime) fn heal_resumed(
        &mut self,
        device: &str,
        resumed: &[ResumedPane],
        now_unix_ms: u64,
    ) {
        for resume in resumed {
            let pending = &mut self.heal_letters.pending;
            if !pending
                .iter()
                .any(|(at, held, _)| at == device && held.pane == resume.pane)
            {
                pending.push((device.to_owned(), resume.clone(), now_unix_ms));
            }
        }
        self.settle_pending();
    }

    /// Settles the pending resumes that can be, oldest first.
    fn settle_pending(&mut self) {
        let pending = std::mem::take(&mut self.heal_letters.pending);
        for (device, resume, since) in pending {
            if !self.heal_one(&device, &resume, since) {
                self.heal_letters.pending.push((device, resume, since));
            }
        }
    }

    /// Settles one resume taken at `since`; false while it cannot be: the
    /// ledger cannot be read, or the registration is still owed the line
    /// of an earlier heal. The cut is spent only when it settles.
    fn heal_one(&mut self, device: &str, resume: &ResumedPane, since: u64) -> bool {
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
        let mut facts = self
            .hook_facts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let cut = facts.cut_pending(&saved);
        let told = cut && self.participant_or_child(record);
        let key = format!("go-on:{}", record.id);
        if told && self.heal_letters.owed.contains_key(&key) {
            crate::diagnostic!(json!({
                "component": "heal", "kind": "heal.pending",
                "pane_id": typed, "agent_id": record.id,
            }));
            return false;
        }
        facts.take_cut(&saved);
        drop(facts);
        crate::diagnostic!(json!({
            "component": "heal", "kind": "heal.began",
            "pane_id": typed, "agent_id": record.id, "cut": cut,
        }));
        if let Some(parent) = &record.parent {
            self.heal_letters
                .resumed
                .entry(parent.clone())
                .or_insert_with(|| Resumed {
                    since,
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
        if cut && !told {
            crate::diagnostic!(json!({
                "component": "heal", "kind": "heal_letter.operator_root", "pane_id": typed,
            }));
        }
        if told {
            let owed = Owed {
                letter: format!("{key}:{since}"),
                to: record.id.clone(),
                body: Body::GoOn {
                    parent: record.parent.clone(),
                    name: agent_name(record),
                },
            };
            self.heal_letters.owed.insert(key, owed);
        }
        true
    }

    /// The agent in `caller`'s pane on `device` reported after its session
    /// started: `healed` when a heal resumed it there. One the engine did not
    /// heal was never cut, whatever the pane's last turn said, since its
    /// holder kept it running; a pane whose resume is pending keeps its cut
    /// for that resume.
    pub(crate) fn hook_after_heal(&mut self, device: &str, caller: &str, healed: bool) {
        let node = self.node.as_str().to_owned();
        let pane = crate::address::named_pane_id(&node, device, caller);
        if healed {
            crate::diagnostic!(json!({
                "component": "heal", "kind": "heal.resumed", "pane_id": pane,
            }));
            return;
        }
        let pending = self.heal_letters.pending.iter().any(|(at, resume, _)| {
            at == device
                && [&resume.pane, &resume.saved]
                    .iter()
                    .any(|held| crate::address::named_pane_id(&node, device, held) == pane)
        });
        if !pending {
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
            letters.owed.remove(&format!("go-on:{id}"));
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
            let key = format!("not-woken:{id}");
            self.heal_letters.owed.entry(key.clone()).or_insert(Owed {
                letter: key,
                to: parent.clone(),
                body: Body::NotWoken(agent_name(record)),
            });
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
    /// to bring it back, its resume is not settled yet, or it is still owed
    /// its line. Nothing it does or leaves undone in that time is its own
    /// (B19).
    pub(crate) fn healing(&self, record: &AgentRecord) -> bool {
        let letters = &self.heal_letters;
        self.still_healing(&record.machine, &record.pane)
            || letters.owed.contains_key(&format!("go-on:{}", record.id))
            || letters.pending.iter().any(|(device, resume, _)| {
                *device == record.machine
                    && (resume.pane == record.pane || resume.saved == record.pane)
            })
    }

    /// Sends each owed line whose agent can be reached
    /// ([`Self::heal_notes_due`]). Run by the coordinator's tick; a tick with
    /// nothing pending, owed or counted is one test.
    pub(crate) fn drive_heal_letters(&mut self, now_unix_ms: u64) {
        self.give_up_healing(now_unix_ms);
        let letters = &self.heal_letters;
        if (letters.owed.is_empty() && letters.pending.is_empty() && letters.resumed.is_empty())
            || !letters.sending.is_empty()
            || self.effects_held()
        {
            return;
        }
        let notes = self.heal_notes_due();
        if notes.is_empty() {
            return;
        }
        let letters: BTreeSet<String> = notes.iter().map(|note| note.letter.clone()).collect();
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
            self.heal_letters.sending = letters;
        }
    }

    /// Settles the lines the ledger admitted, by their letters.
    pub(in crate::runtime) fn heal_lines_admitted(&mut self, admitted: &[String]) {
        self.heal_letters
            .owed
            .retain(|_, owed| !admitted.contains(&owed.letter));
    }

    /// The notes to send now. The pending resumes are settled first, as
    /// their earlier lines allow; each parent's count is owed once each of
    /// its children was seen in its pane, could not be woken or is healing
    /// no more, and stays counted while the parent's last count is still
    /// owed. An entry whose recipient ended is dropped: a line to go on
    /// becomes its parent's line that the child could not be told, unless
    /// the parent ended too, and a line to go on for an agent no longer a
    /// child or a participant is given up in the log. A line stays owed
    /// until the ledger admits it.
    pub(in crate::runtime) fn heal_notes_due(&mut self) -> Vec<Note> {
        self.settle_pending();
        let settled: Vec<String> = self
            .heal_letters
            .resumed
            .iter()
            .filter(|(parent, resumed)| {
                !self
                    .heal_letters
                    .owed
                    .contains_key(&format!("resumed:{parent}"))
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
            let key = format!("resumed:{parent}");
            let owed = Owed {
                letter: format!("{key}:{}", resumed.since),
                to: parent,
                body: Body::Resumed {
                    count: names.len(),
                    names: names.into_iter().take(NAMES_SAID).collect(),
                },
            };
            self.heal_letters.owed.insert(key, owed);
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
                "letter_id": owed.letter, "ended": ended,
            }));
            if let (
                true,
                Body::GoOn {
                    parent: Some(parent),
                    name,
                },
            ) = (ended, owed.body)
                && self.registration(&parent).is_some()
            {
                let key = format!("not-told:{}", owed.to);
                let letter = format!("not-told:{}", owed.letter.trim_start_matches("go-on:"));
                self.heal_letters.owed.entry(key).or_insert(Owed {
                    letter,
                    to: parent,
                    body: Body::NotTold(name),
                });
            }
        }

        let language = self.interface_language();
        self.heal_letters
            .owed
            .values()
            .filter_map(|owed| {
                let target = match owed.body {
                    // The line to go on waits for the agent in its pane.
                    Body::GoOn { .. } => self.observed_registration(&owed.to),
                    _ => self.registered_target(&owed.to),
                }?;
                Some(Note {
                    letter: owed.letter.clone(),
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
    /// The letters owed, in order.
    pub(in crate::runtime) fn heal_letters_owed(&self) -> Vec<String> {
        let mut letters: Vec<String> = self
            .heal_letters
            .owed
            .values()
            .map(|owed| owed.letter.clone())
            .collect();
        letters.sort();
        letters
    }

    /// The resumes taken and not settled, by their pane.
    pub(in crate::runtime) fn heal_pending(&self) -> Vec<String> {
        self.heal_letters
            .pending
            .iter()
            .map(|(_, resume, _)| resume.pane.clone())
            .collect()
    }

    /// Whether a thread is sending lines.
    pub(in crate::runtime) fn heal_letters_sending(&self) -> bool {
        !self.heal_letters.sending.is_empty()
    }
}
