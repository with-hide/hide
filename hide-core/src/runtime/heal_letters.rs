//! What a heal says after a cut (PRD letters-missions-simplify D-06, D-18,
//! D-19, B16 to B19): the one line a resumed agent gets to go on, and what
//! its parent or Conductor hears of the heal.
//!
//! An engine switch, a reboot, a lost holder and a device's reboot or link
//! return all end in the one heal: the engine resumes every agent whose
//! pane was cut and answers, once its hook names its session, that it
//! healed it (`Engine::hook_session`). The core then reads whether the agent
//! was at work when its engine went, from what it holds of the pane's last
//! turn (`HookFacts::cut_pending`), which covers a device's panes as much as
//! this machine's. Only an agent that was at work and is a mission's
//! participant or another agent's child is owed the line; an agent at rest
//! is only reopened, and the operator's own agents are never typed into. The
//! line is the same whatever cut the work, in the operator's language.
//!
//! Each parent hears once per heal how many of its children came back
//! (`Body::Resumed`), once none of their machines is still healing; a child
//! its engine could not resume is said by name (`Body::NotWoken`), and a
//! child that came back but could not be told to go on within [`OWED_FOR`]
//! too (`Body::NotTold`). A mission worker the engine could not resume is
//! its Conductor's `worker_ended` raise already, so it is not said twice.
//!
//! Every line is owed until the ledger admits it: a send that fails, a
//! ledger that is full and a thread that does not start leave it owed, and
//! the next tick sends it again under the same intent, which the ledger
//! takes once. The intent names the heal it belongs to (the moment it was
//! owed), so a second cut of the same conversation is a new line while a
//! retry of one converges. What is owed is memory only, bounded per machine
//! by [`OWED_PER_MACHINE`]: a pane is owed one line to go on at most and
//! its parent one line about it, and a parent one count, so a machine's
//! holder cap bounds them; a line that would pass it is refused in the log,
//! and the cut it stands on stays unspent.

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

/// How long a line may wait for its agent to show, or for the ledger to
/// take it, before the heal gives it up.
const OWED_FOR: u64 = 10 * 60 * 1000;

/// How long a parent's count waits after its last child came back, so
/// the children a heal brings back one after another are said once.
const SETTLE_FOR: u64 = 5_000;

/// The most children a count names; the rest are only counted.
const NAMES_SAID: usize = 8;

#[derive(Default)]
pub(in crate::runtime) struct HealLetters {
    /// The lines owed, by their key (the intent after `heal:`), until the
    /// ledger admits them.
    owed: BTreeMap<String, Owed>,
    /// The children each parent has seen come back in the heal running
    /// now, by the parent's registration id.
    resumed: BTreeMap<String, Resumed>,
    /// The keys a thread is sending now; it is the only one.
    sending: BTreeSet<String>,
    /// The panes each machine's engine names as being healed in its last
    /// read (`ProjectionState::healing`), by the machine's own pane ids,
    /// with when the core first saw each so named.
    healing: BTreeMap<String, BTreeMap<String, Healing>>,
}

/// A pane its engine names healing. A resumed agent's first report can be
/// lost (a hook that cannot reach a daemon still starting gives up), and an
/// agent at rest may report nothing more, so the engine would name it
/// healing for good: the core reads it so for [`OWED_FOR`] at most.
struct Healing {
    since: u64,
    given_up: bool,
}

struct Owed {
    /// The machine of the agent the line is about: what the cap counts.
    machine: String,
    to: To,
    body: Body,
    since: u64,
}

enum To {
    /// The healed agent in this pane, by the core's name, once it shows.
    Pane(String),
    /// The agent registered under this id, observed or asleep.
    Agent(String),
}

enum Body {
    /// Go on with the work that was cut.
    GoOn {
        /// Who to tell when the line cannot go.
        parent: Option<String>,
        name: String,
    },
    /// This many of your children came back, these by name.
    Resumed { count: usize, names: Vec<String> },
    /// This child could not be brought back.
    NotWoken(String),
    /// This child came back but could not be told to go on.
    NotTold(String),
}

struct Resumed {
    machines: BTreeSet<String>,
    children: BTreeMap<String, String>,
    since: u64,
    last: u64,
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
    /// The agent in `caller`'s pane on `device` named its session `session`
    /// after its hook reported: `healed` when a heal resumed it there. Its
    /// parent counts it back; one that was at work when its engine went is
    /// owed the line when it is a child or a participant. One the engine did
    /// not heal was never cut, whatever the pane's last turn said, since its
    /// holder kept it running.
    pub(crate) fn hook_after_heal(
        &mut self,
        device: &str,
        caller: &str,
        session: &str,
        healed: bool,
        now_unix_ms: u64,
    ) {
        let pane = crate::address::named_pane_id(self.node.as_str(), device, caller);
        let facts = self.hook_facts.clone();
        let facts = || {
            facts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        };
        if !healed {
            facts().forget_cut(&pane);
            return;
        }
        let cut = facts().cut_pending(&pane);
        let record = self.registration_in(&pane).cloned();
        crate::diagnostic!(json!({
            "component": "heal", "kind": "heal.resumed",
            "pane_id": pane, "device": device, "cut": cut,
            "agent_id": record.as_ref().map(|record| record.id.as_str()),
        }));
        if let Some(record) = &record
            && let Some(parent) = &record.parent
        {
            let resumed = self
                .heal_letters
                .resumed
                .entry(parent.clone())
                .or_insert_with(|| Resumed {
                    machines: BTreeSet::new(),
                    children: BTreeMap::new(),
                    since: now_unix_ms,
                    last: now_unix_ms,
                });
            resumed.last = now_unix_ms;
            resumed.machines.insert(device.to_owned());
            resumed
                .children
                .insert(record.id.clone(), agent_name(record));
        }
        if !cut {
            return;
        }
        let Some(record) = record.filter(|record| self.participant_or_child(record)) else {
            facts().take_cut(&pane);
            crate::diagnostic!(json!({
                "component": "heal", "kind": "heal_letter.operator_root", "pane_id": pane,
            }));
            return;
        };
        let owed = Owed {
            machine: device.to_owned(),
            to: To::Pane(pane.clone()),
            body: Body::GoOn {
                parent: record.parent.clone(),
                name: agent_name(&record),
            },
            since: now_unix_ms,
        };
        if self.owe_heal_line(format!("go-on:{pane}:{session}:{now_unix_ms}"), owed) {
            facts().take_cut(&pane);
        }
    }

    /// The registrations the engine on `device` could not resume: each
    /// child's parent is told by name, once, unless it is a mission's
    /// worker, whose Conductor hears it as `worker_ended`.
    pub(in crate::runtime) fn heal_not_woken(&mut self, ended: &[String], now_unix_ms: u64) {
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
                    to: To::Agent(parent.clone()),
                    body: Body::NotWoken(agent_name(record)),
                    since: now_unix_ms,
                },
            );
        }
    }

    /// Owes `owed` under `key` and answers whether it is owed: a key owed
    /// already stays as it is; a machine past its cap refuses it.
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

    /// Keeps the panes `device`'s engine named healing in its last read,
    /// each from when the core first saw it so.
    pub(crate) fn note_healing(&mut self, device: &str, panes: &[String], now_unix_ms: u64) {
        let healing = &mut self.heal_letters.healing;
        if panes.is_empty() {
            healing.remove(device);
            return;
        }
        let mut was = healing.remove(device).unwrap_or_default();
        let now: BTreeMap<String, Healing> = panes
            .iter()
            .map(|pane| {
                let kept = was.remove(pane).unwrap_or(Healing {
                    since: now_unix_ms,
                    given_up: false,
                });
                (pane.clone(), kept)
            })
            .collect();
        healing.insert(device.to_owned(), now);
        self.give_up_healing(now_unix_ms);
    }

    /// Reads a pane named healing past [`OWED_FOR`] as healed no more.
    fn give_up_healing(&mut self, now_unix_ms: u64) {
        for (device, panes) in &mut self.heal_letters.healing {
            for (pane, healing) in panes.iter_mut() {
                if !healing.given_up && now_unix_ms.saturating_sub(healing.since) > OWED_FOR {
                    healing.given_up = true;
                    crate::diagnostic!(json!({
                        "component": "heal", "kind": "heal.unconfirmed",
                        "device": device, "pane_id": pane,
                    }));
                }
            }
        }
    }

    /// Whether `device`'s engine names a pane healing that the core still
    /// reads so.
    fn still_healing(&self, device: &str, pane: Option<&str>) -> bool {
        self.heal_letters.healing.get(device).is_some_and(|panes| {
            panes
                .iter()
                .any(|(named, healing)| !healing.given_up && pane.is_none_or(|pane| pane == named))
        })
    }

    /// Whether the agent `record` names is being healed: its engine has yet
    /// to bring it back, or it is back and still owed its line. Nothing it
    /// does or leaves undone in that time is its own (B19).
    pub(crate) fn healing(&self, record: &AgentRecord) -> bool {
        let pane = record.pane_on(self.node.as_str());
        self.still_healing(&record.machine, Some(&record.pane))
            || self.heal_letters.owed.values().any(|owed| {
                matches!(owed.body, Body::GoOn { .. })
                    && matches!(&owed.to, To::Pane(to) if *to == pane)
            })
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
        let notes = self.heal_notes_due(now_unix_ms);
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

    /// The notes to send now: each parent's count once none of its
    /// children's machines still heals and none came back for
    /// [`SETTLE_FOR`], and each owed line whose agent is reached. A line to
    /// go on that waited [`OWED_FOR`] becomes its parent's line that it
    /// could not go, and any other line is given up in the log. Nothing is
    /// settled here: a line stays owed until the ledger admits it
    /// ([`Self::heal_lines_admitted`]).
    pub(in crate::runtime) fn heal_notes_due(&mut self, now_unix_ms: u64) -> Vec<Note> {
        let settled: Vec<String> = self
            .heal_letters
            .resumed
            .iter()
            .filter(|(_, resumed)| {
                (now_unix_ms.saturating_sub(resumed.last) >= SETTLE_FOR
                    && resumed
                        .machines
                        .iter()
                        .all(|machine| !self.still_healing(machine, None)))
                    || now_unix_ms.saturating_sub(resumed.since) > OWED_FOR
            })
            .map(|(parent, _)| parent.clone())
            .collect();
        for parent in settled {
            let Some(resumed) = self.heal_letters.resumed.remove(&parent) else {
                continue;
            };
            let machine = self.registration(&parent).map_or_else(
                || self.node.as_str().to_owned(),
                |record| record.machine.clone(),
            );
            let owed = Owed {
                machine,
                to: To::Agent(parent.clone()),
                body: Body::Resumed {
                    count: resumed.children.len(),
                    names: resumed.children.into_values().take(NAMES_SAID).collect(),
                },
                since: now_unix_ms,
            };
            self.owe_heal_line(format!("resumed:{parent}:{}", resumed.since), owed);
        }

        let expired: Vec<String> = self
            .heal_letters
            .owed
            .iter()
            .filter(|(_, owed)| now_unix_ms.saturating_sub(owed.since) > OWED_FOR)
            .map(|(key, _)| key.clone())
            .collect();
        for key in expired {
            let Some(owed) = self.heal_letters.owed.remove(&key) else {
                continue;
            };
            crate::diagnostic!(json!({
                "component": "heal", "kind": "heal_letter.expired", "key": key,
            }));
            if let Body::GoOn {
                parent: Some(parent),
                name,
            } = owed.body
            {
                let key = format!("not-told:{}", key.trim_start_matches("go-on:"));
                let owed = Owed {
                    machine: owed.machine,
                    to: To::Agent(parent),
                    body: Body::NotTold(name),
                    since: now_unix_ms,
                };
                self.owe_heal_line(key, owed);
            }
        }

        let language = self.interface_language();
        self.heal_letters
            .owed
            .iter()
            .filter_map(|(key, owed)| {
                let target = match &owed.to {
                    To::Pane(pane) => self.delivery_observations.get(pane).cloned(),
                    To::Agent(id) => self.registered_target(id),
                }?;
                Some(Note {
                    letter: key.clone(),
                    target,
                    body: owed.body.text(language),
                })
            })
            .collect()
    }

    /// The live registration whose agent runs in `pane`, by the core's name.
    fn registration_in(&self, pane: &str) -> Option<&AgentRecord> {
        let node = self.node.as_str();
        self.delivery_ledger
            .as_ref()
            .ok()?
            .agents
            .iter()
            .find(|record| !record.ended && record.pane_on(node) == pane)
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

    /// Where a line to the agent registered as `id` goes: the agent observed
    /// in its pane, else that pane's agent asleep, whose mailbox keeps it
    /// for its next wake.
    fn registered_target(&self, id: &str) -> Option<Observation> {
        let record = self.registration(id)?;
        let pane = record.pane_on(self.node.as_str());
        if let Some(observed) = self.delivery_observations.get(&pane)
            && observed.actor.same_identity(&record.actor)
        {
            return Some(observed.clone());
        }
        let ledger = self.delivery_ledger.as_ref().ok()?;
        self.sleeping_target(ledger, Some(&record.actor), &pane)
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
}
