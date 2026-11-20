//! What a heal says after a cut (PRD letters-missions-simplify D-06, D-18,
//! D-19, B16 to B19): the one line a resumed agent gets to go on, and what
//! the agent that answers for it, its parent or else its mission's Conductor
//! (`Runtime::answerer`), hears of the heal.
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
//! machine's), and who answers for it. Nothing waits for the resumed agent's
//! own report, which can come first, late or never.
//!
//! Only a registration that was at work and that someone answers for (a
//! child, or a worker or checker of an open mission, the agent that took the
//! pane of one over included) or that conducts an open mission is owed the
//! line; an agent at rest is only reopened, and the operator's own agents
//! are never typed into. The line is the same whatever cut the work, in the
//! operator's language. Each agent that answers for others (their parent,
//! else their Conductor; "parent" below) hears once per heal what became of
//! them, in its one entry
//! (`Body::Children`), once each has an outcome: it came back
//! (`Lost::Back`: it reported after its resume, or its engine is done with
//! it and its registration stands) or its engine could not resume it
//! (`Lost::NotWoken`); a child whose registration ended before its line
//! could go is said there too (`Lost::NotTold`). A mission worker the
//! engine could not resume is its Conductor's `worker_ended` raise already,
//! so it is not said twice.
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

use std::collections::BTreeMap;

use serde_json::json;

use super::Runtime;
use super::delivery::Observation;
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

#[derive(Default)]
pub(in crate::runtime) struct HealLetters {
    /// The resumes taken and not settled yet, by machine and when taken,
    /// oldest first; one per pane of a machine.
    pending: Vec<Pending>,
    /// The lines owed, one of each kind per live registration: the line to
    /// go on (`go-on:<id>`), and for a parent what became of its children
    /// (`children:<id>`), until the ledger admits them.
    owed: BTreeMap<String, Owed>,
    /// The children each parent's count waits on, by the parent's
    /// registration id.
    resumed: BTreeMap<String, Resumed>,
    /// The last batch a parent's lost children were named in: each batch is
    /// a new letter, later than every one before, even after a restart.
    last_batch: u64,
    /// The panes each machine's engine names healing, by the machine's own
    /// pane ids, from the core's first read of each.
    healing: BTreeMap<String, BTreeMap<String, Healing>>,
}

/// A resume taken and not settled yet.
struct Pending {
    device: String,
    resume: ResumedPane,
    since: u64,
    /// The registration cut in the pane, once the core read it: a retry
    /// settles that one or none, never another agent that took the pane.
    cut: Option<Cut>,
}

/// A line due now: the key it is owed under, the agent it goes to, and its
/// text.
pub(in crate::runtime) struct Note {
    pub(in crate::runtime) letter: String,
    pub(in crate::runtime) target: Observation,
    pub(in crate::runtime) body: String,
}

impl Note {
    /// The line's cell: its subject, one per heal and entry, and its revision,
    /// the heal's own moment and, for a parent's entry, how many children it
    /// has said, so a name added updates the cell and a retry is the same.
    fn cell(&self) -> (String, u64) {
        let (key, count) = self
            .letter
            .rsplit_once(':')
            .expect("a line's key ends with its moment or its count");
        let count: u64 = count.parse().unwrap_or_default();
        match key.strip_prefix("children:") {
            Some(entry) => {
                let batch = entry
                    .rsplit_once(':')
                    .and_then(|(_, batch)| batch.parse::<u64>().ok())
                    .unwrap_or_default();
                (format!("heal:{key}"), batch.saturating_add(count))
            }
            None => (format!("heal:{key}"), count),
        }
    }
}

struct Cut {
    id: String,
    parent: Option<String>,
    name: String,
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
    /// What became of these children in the heal, oldest first; the ones past
    /// [`CHILDREN_SAID`] only counted. `batch` names this entry's letters
    /// apart from every earlier entry's to the same parent; `told` is how
    /// many of its children were said before.
    Children {
        batch: u64,
        told: usize,
        lost: Vec<(Lost, String)>,
        more: usize,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lost {
    /// The child came back.
    Back,
    /// The child could not be brought back.
    NotWoken,
    /// The child came back but ended before it could be told to go on.
    NotTold,
}

/// The most children a parent's entry names; the rest are counted.
const CHILDREN_SAID: usize = 32;

/// A parent's count: its children the heal is bringing back, by id.
struct Resumed {
    children: BTreeMap<String, Child>,
}

struct Child {
    name: String,
    machine: String,
    /// The pane by its machine's own id.
    pane: String,
    outcome: Outcome,
}

/// What is known of a child in its parent's count.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// Its resume is said; its engine has not named its pane healing yet.
    Said,
    /// Its engine named its pane healing: once it names it no more (or the
    /// core gives the pane up), the engine is done with it.
    Named,
    /// It reported after its resume: it came back.
    Back,
    /// Its engine could not resume it.
    NotWoken,
    /// A mission's worker its engine could not resume: its Conductor hears
    /// it as `worker_ended`, so the count does not say it.
    Raised,
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
            (Self::Children { lost, more, .. }, language) => {
                let names = |kind: Lost| -> String {
                    lost.iter()
                        .filter(|(at, _)| *at == kind)
                        .map(|(_, name)| name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let mut lines = Vec::new();
                let back = lost.iter().filter(|(at, _)| *at == Lost::Back).count();
                if back > 0 {
                    let names = names(Lost::Back);
                    lines.push(match language {
                        Korean => format!("되살림: {back}명 · {names}"),
                        Japanese => format!("再開: {back}件 · {names}"),
                        SimplifiedChinese => format!("已恢复：{back} 个 · {names}"),
                        English => format!("Resumed: {back} · {names}"),
                    });
                }
                let (not_woken, not_told) = (names(Lost::NotWoken), names(Lost::NotTold));
                if !not_woken.is_empty() {
                    lines.push(match language {
                        Korean => format!("깨우지 못함: {not_woken}"),
                        Japanese => format!("再開できませんでした: {not_woken}"),
                        SimplifiedChinese => format!("未能恢复：{not_woken}"),
                        English => format!("Could not wake {not_woken}"),
                    });
                }
                if !not_told.is_empty() {
                    lines.push(match language {
                        Korean => format!("이어서 하라고 전하지 못함: {not_told}"),
                        Japanese => format!("続行を伝えられませんでした: {not_told}"),
                        SimplifiedChinese => format!("未能告知继续：{not_told}"),
                        English => format!("Could not tell {not_told} to carry on"),
                    });
                }
                if *more > 0 {
                    lines.push(match language {
                        Korean => format!("그 밖에 {more}명"),
                        Japanese => format!("ほか{more}件"),
                        SimplifiedChinese => format!("另外 {more} 个"),
                        English => format!("and {more} more"),
                    });
                }
                lines.join("\n")
            }
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
                .any(|held| held.device == device && held.resume.pane == resume.pane)
            {
                pending.push(Pending {
                    device: device.to_owned(),
                    resume: resume.clone(),
                    since: now_unix_ms,
                    cut: None,
                });
            }
        }
        self.settle_pending();
    }

    /// Settles the pending resumes that can be, oldest first.
    fn settle_pending(&mut self) {
        let pending = std::mem::take(&mut self.heal_letters.pending);
        for mut held in pending {
            if !self.heal_one(&mut held) {
                self.heal_letters.pending.push(held);
            }
        }
    }

    /// Settles one resume; false while it cannot be: the ledger cannot be
    /// read, or the registration is still owed the line of an earlier heal,
    /// which it then keeps in `held` as the one this resume is about. The
    /// cut is spent only when it settles.
    fn heal_one(&mut self, held: &mut Pending) -> bool {
        let (device, resume, since) = (held.device.clone(), held.resume.clone(), held.since);
        let device = device.as_str();
        let node = self.node.as_str().to_owned();
        let saved = crate::address::named_pane_id(&node, device, &resume.saved);
        let typed = crate::address::named_pane_id(&node, device, &resume.pane);
        let Ok(ledger) = self.delivery_ledger.clone() else {
            crate::diagnostic!(json!({
                "component": "heal", "kind": "heal.unread", "pane_id": typed,
            }));
            return false;
        };
        let in_pane =
            |record: &AgentRecord| record.pane_on(&node) == saved || record.pane_on(&node) == typed;
        let found = ledger.agents.iter().find(|record| {
            !record.ended
                && in_pane(record)
                && held.cut.as_ref().is_none_or(|cut| cut.id == record.id)
        });
        let answerer = found.and_then(|record| self.answerer(record));
        let Some(record) = found else {
            // The agent it was cut for ended or left the pane before its
            // line could be owed: its parent hears it could not be told.
            if let Some(cut) = held.cut.take() {
                self.hook_facts
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take_cut(&saved);
                crate::diagnostic!(json!({
                    "component": "heal", "kind": "heal.gone",
                    "pane_id": typed, "agent_id": cut.id,
                }));
                if let Some(parent) = cut.parent {
                    self.tell_parent(&parent, Lost::NotTold, cut.name);
                }
            }
            return true;
        };
        let mut facts = self
            .hook_facts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let cut = facts.cut_pending(&saved);
        let told = cut && self.owed_go_on(record);
        let key = format!("go-on:{}", record.id);
        if told && self.heal_letters.owed.contains_key(&key) {
            crate::diagnostic!(json!({
                "component": "heal", "kind": "heal.pending",
                "pane_id": typed, "agent_id": record.id,
            }));
            held.cut = Some(Cut {
                id: record.id.clone(),
                parent: answerer,
                name: agent_name(record),
            });
            return false;
        }
        facts.take_cut(&saved);
        drop(facts);
        crate::diagnostic!(json!({
            "component": "heal", "kind": "heal.began",
            "pane_id": typed, "agent_id": record.id, "cut": cut,
        }));
        if let Some(parent) = &answerer {
            self.heal_letters
                .resumed
                .entry(parent.clone())
                .or_insert_with(|| Resumed {
                    children: BTreeMap::new(),
                })
                .children
                .insert(
                    record.id.clone(),
                    Child {
                        name: agent_name(record),
                        machine: device.to_owned(),
                        pane: resume.pane.clone(),
                        outcome: Outcome::Said,
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
                    parent: answerer,
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
            for child in self
                .heal_letters
                .resumed
                .values_mut()
                .flat_map(|resumed| resumed.children.values_mut())
            {
                if child.machine == device
                    && crate::address::named_pane_id(&node, device, &child.pane) == pane
                    && child.outcome != Outcome::NotWoken
                {
                    child.outcome = Outcome::Back;
                }
            }
            return;
        }
        let pending = self.heal_letters.pending.iter().any(|held| {
            held.device == device
                && [&held.resume.pane, &held.resume.saved]
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
    /// parent's count and its line to go on, and the agent that answers for
    /// it is told by name, once, unless it is a mission's worker, whose
    /// Conductor hears it as `worker_ended`.
    pub(in crate::runtime) fn heal_not_woken(&mut self, ended: &[String]) {
        let Ok(ledger) = self.delivery_ledger.clone() else {
            return;
        };
        for id in ended {
            let Some(record) = ledger.agents.iter().find(|record| &record.id == id) else {
                continue;
            };
            let answerer = self.answerer(record);
            let letters = &mut self.heal_letters;
            letters.owed.remove(&format!("go-on:{id}"));
            let Some(parent) = &answerer else {
                continue;
            };
            let worker = self.missions.book.missions.iter().any(|mission| {
                mission
                    .nodes
                    .iter()
                    .any(|node| node.agent_id.as_deref() == Some(id.as_str()))
            });
            // A child in its parent's count is said with the count.
            if let Some(child) = letters
                .resumed
                .get_mut(parent)
                .and_then(|resumed| resumed.children.get_mut(id))
            {
                child.outcome = if worker {
                    Outcome::Raised
                } else {
                    Outcome::NotWoken
                };
                continue;
            }
            if worker {
                continue;
            }
            self.tell_parent(parent, Lost::NotWoken, agent_name(record));
        }
    }

    /// Adds the child `name`, lost as `lost`, to the parent's one entry of
    /// lost children: a new letter, so one being sent says the ones before.
    fn tell_parent(&mut self, parent: &str, lost: Lost, name: String) {
        let key = format!("children:{parent}");
        let letters = &mut self.heal_letters;
        if !letters.owed.contains_key(&key) {
            letters.last_batch = crate::links::now_ms().max(letters.last_batch + 1);
        }
        let batch = letters.last_batch;
        let owed = letters.owed.entry(key).or_insert_with(|| Owed {
            letter: String::new(),
            to: parent.to_owned(),
            body: Body::Children {
                batch,
                told: 0,
                lost: Vec::new(),
                more: 0,
            },
        });
        if let Body::Children {
            lost: said, more, ..
        } = &mut owed.body
        {
            if said.iter().any(|(at, named)| *at == lost && *named == name) {
                return;
            }
            if said.len() < CHILDREN_SAID {
                said.push((lost, name));
            } else {
                *more += 1;
            }
        }
        owed.letter = children_letter(owed);
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
        for child in self
            .heal_letters
            .resumed
            .values_mut()
            .flat_map(|resumed| resumed.children.values_mut())
        {
            if child.outcome == Outcome::Said
                && child.machine == device
                && named.contains_key(&child.pane)
            {
                child.outcome = Outcome::Named;
            }
        }
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
            || letters.pending.iter().any(|held| {
                held.device == record.machine
                    && (held.resume.pane == record.pane || held.resume.saved == record.pane)
            })
    }

    /// Sends each owed line whose agent can be reached
    /// ([`Self::heal_notes_due`]). Run by the coordinator's tick; a tick with
    /// nothing pending, owed or counted is one test.
    pub(crate) fn drive_heal_letters(&mut self, now_unix_ms: u64) {
        self.give_up_healing(now_unix_ms);
        let letters = &self.heal_letters;
        if (letters.owed.is_empty() && letters.pending.is_empty() && letters.resumed.is_empty())
            || self.effects_held()
        {
            return;
        }
        // Each line is a news cell of the agent it goes to (PRD
        // letters-missions-simplify D-16, B30): posted, it is admitted; one
        // refused for room stays owed for the next tick.
        let mut admitted = Vec::new();
        for note in self.heal_notes_due() {
            let (subject, revision) = note.cell();
            match self.post_status(&note.target.actor, subject, note.body, true, revision) {
                Ok(()) => admitted.push(note.letter),
                Err(_) => crate::diagnostic!(json!({
                    "component": "heal", "kind": "heal_letter.unsent",
                    "letter_id": note.letter, "code": "status_full",
                })),
            }
        }
        if !admitted.is_empty() {
            self.heal_lines_admitted(&admitted);
        }
    }

    /// Settles the lines the ledger admitted, by their letters. A parent's
    /// lost children named since its letter left stay owed.
    pub(in crate::runtime) fn heal_lines_admitted(&mut self, admitted: &[String]) {
        self.heal_letters.owed.retain(|key, owed| {
            if admitted.contains(&owed.letter) {
                return false;
            }
            let Body::Children {
                batch,
                told,
                lost,
                more,
            } = &mut owed.body
            else {
                return true;
            };
            let Some(said) = admitted.iter().find_map(|letter| {
                let (of, said) = letter
                    .strip_prefix(key.as_str())?
                    .strip_prefix(':')?
                    .split_once(':')?;
                (of.parse::<u64>().ok()? == *batch).then_some(())?;
                said.parse::<usize>().ok()
            }) else {
                return true;
            };
            let mut gone = said.saturating_sub(*told);
            let named = gone.min(lost.len());
            lost.drain(..named);
            gone -= named;
            *more = more.saturating_sub(gone);
            *told = said.max(*told);
            owed.letter = children_letter(owed);
            true
        });
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
            .filter(|(_, resumed)| {
                resumed.children.values().all(|child| match child.outcome {
                    Outcome::Said => false,
                    Outcome::Named => !self.still_healing(&child.machine, &child.pane),
                    Outcome::Back | Outcome::NotWoken | Outcome::Raised => true,
                })
            })
            .map(|(parent, _)| parent.clone())
            .collect();
        for parent in settled {
            let Some(resumed) = self.heal_letters.resumed.remove(&parent) else {
                continue;
            };
            for (id, child) in resumed.children {
                match child.outcome {
                    // A child that ended since is not one that came back.
                    Outcome::Back | Outcome::Named if self.registration(&id).is_some() => {
                        self.tell_parent(&parent, Lost::Back, child.name);
                    }
                    Outcome::NotWoken => self.tell_parent(&parent, Lost::NotWoken, child.name),
                    _ => {}
                }
            }
        }

        let unreachable: Vec<String> = self
            .heal_letters
            .owed
            .iter()
            .filter(|(_, owed)| {
                self.registration(&owed.to).is_none_or(|record| {
                    matches!(owed.body, Body::GoOn { .. }) && !self.owed_go_on(record)
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
                self.tell_parent(&parent, Lost::NotTold, name);
            }
        }

        let language = self.interface_language();
        self.heal_letters
            .owed
            .values()
            .filter_map(|owed| {
                let target = match owed.body {
                    // The line to go on waits for the agent in its pane.
                    Body::GoOn { .. } => self.observed_agent(&owed.to),
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

    /// The live registration in `id`'s place: `id` itself, or the agent that
    /// took its pane over (`coordination::live_successor`), so a line owed
    /// to an agent goes to the one after it, as the letters waiting for it
    /// do (PRD letters-missions-simplify B2).
    fn registration(&self, id: &str) -> Option<&AgentRecord> {
        crate::coordination::live_successor(self.delivery_ledger.as_ref().ok()?, id)
            .and_then(Result::ok)
    }

    /// The agent registered as `id`, observed in its pane.
    fn observed_agent(&self, id: &str) -> Option<Observation> {
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
        if let Some(observed) = self.observed_agent(id) {
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

    /// Whether a heal tells the agent `record` registers to go on (D-19):
    /// someone answers for it (`Runtime::answerer`: its parent, else the
    /// Conductor of an open mission it works or checks in, which an agent
    /// that took such a pane over without a parent still has), or it conducts
    /// an open mission.
    fn owed_go_on(&self, record: &AgentRecord) -> bool {
        self.answerer(record).is_some()
            || self.missions.book.missions.iter().any(|mission| {
                mission.status.open()
                    && (mission.conductor.agent_id.as_deref() == Some(record.id.as_str())
                        || mission.conductor.actor.same_identity(&record.actor))
            })
    }
}

/// The letter of a parent's lost children: its batch and how many the
/// batch has said in all, so each name added makes a new letter and a new
/// batch never meets an earlier one's.
fn children_letter(owed: &Owed) -> String {
    match &owed.body {
        Body::Children {
            batch,
            told,
            lost,
            more,
        } => {
            format!("children:{}:{batch}:{}", owed.to, told + lost.len() + more)
        }
        _ => owed.letter.clone(),
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
            .map(|held| held.resume.pane.clone())
            .collect()
    }
}
