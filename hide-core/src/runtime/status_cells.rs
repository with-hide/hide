//! Hide's own notices as status cells (PRD letters-missions-simplify D-03,
//! D-16, D-17, D-24; `docs/delivery.md`, Hide's notices are status cells).
//!
//! What Hide itself has to tell an agent is not a letter: it is the state of
//! one subject (a mission node, a child agent, a letter the agent sent), held
//! once for the party responsible for it (the Conductor, the parent, the
//! sender). A new fact about the same subject updates the cell, and the cell
//! is gone once its cause is: a cell is computed from facts Hide already
//! holds, the mission's recorded wakes (`Mission::wakes`, kept one standing
//! and one news per node by `missions::push_wake`) and the watches, or posted
//! by a pass that judges a fact kept in memory only (`post_status`), which
//! takes it back once the fact is gone (`retract_status`). A cell that is
//! news (a merge, a revival, a letter that ended) is shown once and then
//! gone; a standing cell (a warning, a failed CI, a blocked child) stays
//! until its cause resolves but is shown again only when it changes.
//!
//! The receiver gets the cells that changed since it last looked, at its
//! turn start (the prompt pull, beside the plan hint) and in `hide inbox`,
//! and both mark what they gave as seen (`take_status`). What each
//! registration saw is the one thing kept, in the delivery ledger
//! (`Ledger::status_seen`), written behind the memory copy this module reads.
//! A resting receiver is rung once when a subject it has never seen appears
//! (`status_rings_due`, `delivery::doorbell`), never for an update of one.
//!
//! Every revision grows with the fact it stands for: a mission's wake count
//! for a node's cell, the time the fact was made otherwise, so "changed" is
//! a revision past the one seen.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::json;

use super::Runtime;
use crate::delivery::Actor;
use crate::delivery::ledger::Ledger;
use crate::missions::{MissionStatus, PendingWake};

/// The most text one turn's changed cells take in the prompt's context,
/// beside the letters (`delivery::HOOK_LIMIT`): the rest wait, unseen, for
/// the next turn or `hide inbox`.
const SHOWN_BYTES: usize = 3 * 1_024;
/// The most text one cell shows; a longer one is cut.
const CELL_BYTES: usize = 700;
/// What the changed cells open with in a prompt's context.
const HEADING: &str = "Hide status, changed since you last looked:\n";
/// The most cells `hide inbox` gives at once.
const INBOX_CELLS: usize = 64;
/// The most cells posted by memory-only passes; the oldest goes first
/// (engineering #15). Bounded in practice by the panes and open letters.
const POSTED_LIMIT: usize = 1_024;

/// One subject's status for the party responsible for it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Cell {
    /// What the cell is about: `node:<mission>:<node>`, `mission:<mission>`,
    /// `child:<pane>` or `letter:<id>`.
    pub subject: String,
    /// What the receiver reads.
    pub line: String,
    /// Shown once, then gone; a standing cell stays until its cause goes.
    pub news: bool,
    /// Grows with the fact the cell stands for.
    #[serde(skip)]
    pub revision: u64,
}

/// A cell and the party it is for: the registration id, or the pane of an
/// agent Hide holds no registration for, and the identity a bell for it
/// would ring.
#[derive(Clone, Debug)]
pub(crate) struct Addressed {
    pub(crate) recipient: String,
    pub(crate) actor: Option<Actor>,
    pub(crate) cell: Cell,
}

/// A receiver the doorbell may ring once for the subjects it has never seen
/// (`Runtime::status_rings_due`).
#[derive(Clone, Debug)]
pub(crate) struct StatusRing {
    pub(crate) recipient: String,
    pub(crate) actor: Actor,
    pub(crate) subjects: Vec<String>,
    /// The newest new cell, which the bell line names.
    pub(crate) line: String,
    pub(crate) others: usize,
}

#[derive(Default)]
pub(in crate::runtime) struct StatusCells {
    /// The cells memory-only passes posted, by recipient and subject.
    posted: BTreeMap<(String, String), Posted>,
    /// What each recipient saw, by subject: read from the ledger once, then
    /// kept here and written behind (`Effect::StatusSeen`).
    seen: Option<BTreeMap<String, BTreeMap<String, u64>>>,
    /// The subjects each recipient was rung for since this core started.
    rung: BTreeMap<String, BTreeSet<String>>,
    /// The last revision a post took, so two posts in one millisecond differ.
    last_revision: u64,
}

#[derive(Clone, Debug)]
struct Posted {
    actor: Actor,
    cell: Cell,
}

/// The subject of a cell about mission `mission`'s node `node`, or about the
/// mission itself when `node` is empty.
pub(crate) fn node_subject(mission: &str, node: &str) -> String {
    if node.is_empty() {
        format!("mission:{mission}")
    } else {
        format!("node:{mission}:{node}")
    }
}

/// The subject of a cell about the child in pane `pane`.
pub(crate) fn child_subject(pane: &str) -> String {
    format!("child:{pane}")
}

/// The subject of a cell about letter `id`, for its sender.
pub(crate) fn letter_subject(id: &str) -> String {
    format!("letter:{id}")
}

/// The key a cell for `actor` is kept under: its live registration, or its
/// pane while Hide holds none for it.
pub(crate) fn recipient_key(ledger: Option<&Ledger>, actor: &Actor) -> String {
    ledger
        .and_then(|ledger| {
            ledger
                .agents
                .iter()
                .rev()
                .find(|record| !record.ended && record.actor.same_identity(actor))
        })
        .map_or_else(
            || format!("pane:{}:{}", actor.device_id, actor.pane_id),
            |record| record.id.clone(),
        )
}

fn cut(text: &str) -> String {
    if text.len() <= CELL_BYTES {
        return text.to_owned();
    }
    let mut end = CELL_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", text[..end].trim_end())
}

impl Runtime {
    /// Posts `line` as `recipient`'s cell about `subject`: a pass that judges
    /// a fact kept in memory only (a blocked child, a letter that could not
    /// go). The same line again is no change; another one updates the cell.
    pub(crate) fn post_status(
        &mut self,
        recipient: &Actor,
        subject: String,
        line: String,
        news: bool,
    ) {
        let key = recipient_key(self.delivery_ledger.as_deref().ok(), recipient);
        let line = cut(&line);
        let cells = &mut self.status_cells;
        if let Some(posted) = cells.posted.get_mut(&(key.clone(), subject.clone())) {
            posted.actor = recipient.clone();
            if posted.cell.line == line && posted.cell.news == news {
                return;
            }
        }
        let revision = super::unix_milliseconds().max(cells.last_revision + 1);
        cells.last_revision = revision;
        crate::diagnostic!(json!({
            "component": "status", "kind": "status.posted",
            "recipient": key, "subject": subject, "news": news,
        }));
        cells.posted.insert(
            (key, subject.clone()),
            Posted {
                actor: recipient.clone(),
                cell: Cell {
                    subject,
                    line,
                    news,
                    revision,
                },
            },
        );
        while cells.posted.len() > POSTED_LIMIT {
            let Some(oldest) = cells
                .posted
                .iter()
                .min_by_key(|(_, posted)| posted.cell.revision)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            cells.posted.remove(&oldest);
        }
    }

    /// Takes back every posted cell about `subject`: its cause is gone.
    pub(in crate::runtime) fn retract_status(&mut self, subject: &str) {
        self.status_cells
            .posted
            .retain(|(_, about), _| about != subject);
    }

    /// Every cell standing now, one per recipient and subject, the newest
    /// fact of each.
    pub(crate) fn status_cells(&self, ledger: Option<&Ledger>) -> Vec<Addressed> {
        let mut out: BTreeMap<(String, String), Addressed> = BTreeMap::new();
        let mut put = |addressed: Addressed| {
            let key = (addressed.recipient.clone(), addressed.cell.subject.clone());
            match out.get(&key) {
                Some(kept) if kept.cell.revision >= addressed.cell.revision => {}
                _ => {
                    out.insert(key, addressed);
                }
            }
        };
        for addressed in self.mission_cells(ledger) {
            put(addressed);
        }
        if let Some(ledger) = ledger {
            for addressed in watch_cells(ledger) {
                put(addressed);
            }
        }
        for ((recipient, _), posted) in &self.status_cells.posted {
            put(Addressed {
                recipient: recipient.clone(),
                actor: Some(posted.actor.clone()),
                cell: posted.cell.clone(),
            });
        }
        out.into_values().collect()
    }

    /// The cells of each mission's recorded wakes: the Conductor's, or a
    /// worker's for a wake to it. A cancelled mission tells no one (D-51).
    fn mission_cells(&self, ledger: Option<&Ledger>) -> Vec<Addressed> {
        let mut out = Vec::new();
        for mission in &self.missions.book.missions {
            if mission.status == MissionStatus::Cancelled {
                continue;
            }
            for wake in &mission.wakes {
                let recipient = wake_recipient(mission, wake, ledger);
                let actor = match &wake.to {
                    Some(agent) => registered_actor(ledger, agent),
                    None => mission
                        .conductor
                        .agent_id
                        .as_deref()
                        .and_then(|agent| registered_actor(ledger, agent))
                        .or_else(|| Some(mission.conductor.actor.clone())),
                };
                out.push(Addressed {
                    recipient,
                    actor,
                    cell: Cell {
                        subject: node_subject(&mission.id, &wake.node),
                        line: cut(&super::missions::wake_body(mission, wake)),
                        news: !wake.standing(),
                        revision: wake.seq,
                    },
                });
            }
        }
        out
    }

    /// What `recipient` saw, read from the ledger the first time.
    fn status_seen(&mut self) -> &mut BTreeMap<String, BTreeMap<String, u64>> {
        if self.status_cells.seen.is_none() {
            let kept = self
                .delivery_ledger
                .as_ref()
                .ok()
                .map(|ledger| ledger.status_seen.clone())
                .unwrap_or_default();
            self.status_cells.seen = Some(kept);
        }
        self.status_cells.seen.get_or_insert_with(BTreeMap::new)
    }

    /// The cells of the agent in `caller`'s pane on `device` that changed
    /// since it last looked, marked seen now: for a turn's prompt as many as
    /// `room` bytes of text hold, at most `SHOWN_BYTES`; for `hide inbox`
    /// (`room` none) at most `INBOX_CELLS`. A news cell seen is gone. None
    /// for a caller in no pane Hide observes.
    pub(crate) fn take_status(
        &mut self,
        device: &str,
        caller: &str,
        room: Option<usize>,
    ) -> Option<Vec<Cell>> {
        let crate::workspace_control::Caller::Pane(pane) =
            crate::workspace_control::Caller::parse(caller)
        else {
            return None;
        };
        let pane = crate::address::named_pane_id(self.node.as_str(), device, pane);
        let actor = self.delivery_observations.get(&pane)?.actor.clone();
        let ledger = self.delivery_ledger.clone().ok();
        let recipient = recipient_key(ledger.as_deref(), &actor);
        Some(self.take_status_of(&recipient, ledger.as_deref(), room))
    }

    pub(crate) fn take_status_of(
        &mut self,
        recipient: &str,
        ledger: Option<&Ledger>,
        room: Option<usize>,
    ) -> Vec<Cell> {
        let mut mine: Vec<Cell> = self
            .status_cells(ledger)
            .into_iter()
            .filter(|addressed| addressed.recipient == recipient)
            .map(|addressed| addressed.cell)
            .collect();
        // Newest first: what moved last is read first.
        mine.sort_by(|a, b| b.revision.cmp(&a.revision));
        let standing: BTreeSet<String> = mine.iter().map(|cell| cell.subject.clone()).collect();
        let seen = self.status_seen().entry(recipient.to_owned()).or_default();
        let mut given = Vec::new();
        // The text's own heading and each cell's marks count too.
        let mut bytes = HEADING.len();
        for cell in mine {
            if seen
                .get(&cell.subject)
                .is_some_and(|at| *at >= cell.revision)
            {
                continue;
            }
            match room {
                None if given.len() >= INBOX_CELLS => break,
                None => {}
                Some(room) => {
                    bytes += cell.line.len() + 4 * cell.line.lines().count().max(1);
                    if bytes > room.min(SHOWN_BYTES) {
                        break;
                    }
                }
            }
            given.push(cell);
        }
        for cell in &given {
            seen.insert(cell.subject.clone(), cell.revision);
        }
        // A subject no cell stands for any more keeps nothing seen.
        seen.retain(|subject, _| standing.contains(subject));
        let kept = seen.clone();
        if kept.is_empty() {
            self.status_seen().remove(recipient);
        }
        if !given.is_empty() {
            crate::diagnostic!(json!({
                "component": "status", "kind": "status.taken", "recipient": recipient,
                "cells": given.len(), "inbox": room.is_none(),
            }));
        }
        self.drop_seen_news(recipient, &given);
        // Only a registration's seen cells outlast the core.
        if !recipient.starts_with("pane:")
            && let Some(client) = self.delivery_client.clone()
            && let Err(code) = client.post(crate::delivery::worker::Effect::StatusSeen {
                recipient: recipient.to_owned(),
                seen: kept,
            })
        {
            crate::diagnostic!(json!({
                "component": "status", "kind": "status.seen_unsaved",
                "recipient": recipient, "code": code,
            }));
        }
        given
    }

    /// Lets the news cells `recipient` was just given go: a posted one from
    /// memory, a mission's wake from its record.
    fn drop_seen_news(&mut self, recipient: &str, given: &[Cell]) {
        let news: Vec<&Cell> = given.iter().filter(|cell| cell.news).collect();
        if news.is_empty() {
            return;
        }
        self.status_cells.posted.retain(|(to, subject), posted| {
            !(to == recipient
                && news
                    .iter()
                    .any(|cell| cell.subject == *subject && cell.revision == posted.cell.revision))
        });
        let ledger = self.delivery_ledger.clone().ok();
        let mut changed = false;
        for mission in &mut self.missions.book.missions {
            let keep: Vec<bool> = mission
                .wakes
                .iter()
                .map(|wake| {
                    wake.standing()
                        || wake_recipient(mission, wake, ledger.as_deref()) != recipient
                        || !news.iter().any(|cell| {
                            cell.subject == node_subject(&mission.id, &wake.node)
                                && cell.revision >= wake.seq
                        })
                })
                .collect();
            if keep.iter().all(|kept| *kept) {
                continue;
            }
            let mut at = 0;
            mission.wakes.retain(|_| {
                at += 1;
                keep[at - 1]
            });
            changed = true;
        }
        if changed {
            self.save_missions();
        }
    }

    /// The receivers to ring once now: each with a cell whose subject it has
    /// neither seen nor been rung for, and an identity a bell can reach.
    pub(crate) fn status_rings_due(&mut self) -> Vec<StatusRing> {
        let ledger = self.delivery_ledger.clone().ok();
        let cells = self.status_cells(ledger.as_deref());
        let mut fresh: BTreeMap<String, (Actor, Vec<Cell>)> = BTreeMap::new();
        let seen = self.status_seen().clone();
        let rung = &self.status_cells.rung;
        for addressed in cells {
            let Some(actor) = addressed.actor else {
                continue;
            };
            let known = seen
                .get(&addressed.recipient)
                .is_some_and(|seen| seen.contains_key(&addressed.cell.subject))
                || rung
                    .get(&addressed.recipient)
                    .is_some_and(|rung| rung.contains(&addressed.cell.subject));
            if known {
                continue;
            }
            fresh
                .entry(addressed.recipient)
                .or_insert_with(|| (actor, Vec::new()))
                .1
                .push(addressed.cell);
        }
        fresh
            .into_iter()
            .map(|(recipient, (actor, mut cells))| {
                cells.sort_by(|a, b| b.revision.cmp(&a.revision));
                StatusRing {
                    recipient,
                    actor,
                    line: cells[0].line.clone(),
                    others: cells.len() - 1,
                    subjects: cells.into_iter().map(|cell| cell.subject).collect(),
                }
            })
            .collect()
    }

    /// The workers a mission woke Hide for with a cell they have not seen
    /// (`mission_health::hide_wake`, a node's fix while its worker sleeps):
    /// each worker's identity and the newest such cell's revision. A cell
    /// rings a resting receiver; one for a sleeping worker wakes it instead
    /// (`agent_sleep::letters`), once per revision.
    pub(crate) fn worker_cells_unseen(&mut self) -> Vec<(String, Actor, u64)> {
        let ledger = self.delivery_ledger.clone().ok();
        let seen = self.status_seen().clone();
        let mut newest: BTreeMap<String, (Actor, u64)> = BTreeMap::new();
        for mission in &self.missions.book.missions {
            for wake in &mission.wakes {
                let Some(agent) = wake.to.as_deref() else {
                    continue;
                };
                let subject = node_subject(&mission.id, &wake.node);
                let looked = seen
                    .get(agent)
                    .and_then(|seen| seen.get(&subject))
                    .copied()
                    .unwrap_or(0);
                if wake.seq <= looked {
                    continue;
                }
                let Some(actor) = registered_actor(ledger.as_deref(), agent) else {
                    continue;
                };
                let entry = newest.entry(agent.to_owned()).or_insert((actor, 0));
                entry.1 = entry.1.max(wake.seq);
            }
        }
        newest
            .into_iter()
            .map(|(agent, (actor, revision))| (agent, actor, revision))
            .collect()
    }

    /// The doorbell rang `recipient` for `subjects`: none of them rings it
    /// again. Subjects no cell stands for any more are forgotten.
    pub(crate) fn status_rung(&mut self, recipient: &str, subjects: &[String]) {
        let ledger = self.delivery_ledger.clone().ok();
        let standing: BTreeSet<(String, String)> = self
            .status_cells(ledger.as_deref())
            .into_iter()
            .map(|addressed| (addressed.recipient, addressed.cell.subject))
            .collect();
        let rung = &mut self.status_cells.rung;
        rung.entry(recipient.to_owned())
            .or_default()
            .extend(subjects.iter().cloned());
        for (to, subjects) in rung.iter_mut() {
            subjects.retain(|subject| standing.contains(&(to.clone(), subject.clone())));
        }
        rung.retain(|_, subjects| !subjects.is_empty());
    }
}

#[cfg(test)]
impl Runtime {
    /// The cells `mission`'s Conductor reads now, taken as its turn's
    /// prompt would take them, with room for all.
    pub(crate) fn take_conductor_cells(&mut self, mission: &str) -> Vec<Cell> {
        let ledger = self.delivery_ledger.clone().ok();
        let conductor = &self.missions.book.get(mission).expect("mission").conductor;
        let recipient = conductor
            .agent_id
            .clone()
            .unwrap_or_else(|| recipient_key(ledger.as_deref(), &conductor.actor));
        self.take_status_of(&recipient, ledger.as_deref(), Some(usize::MAX))
    }

    /// Every cell standing now for `recipient`, seen or not.
    pub(crate) fn cells_of(&self, recipient: &str) -> Vec<Cell> {
        let ledger = self.delivery_ledger.clone().ok();
        self.status_cells(ledger.as_deref())
            .into_iter()
            .filter(|addressed| addressed.recipient == recipient)
            .map(|addressed| addressed.cell)
            .collect()
    }
}

/// Whom a mission's recorded wake is for: the worker it names, else the
/// Conductor's registration, else the Conductor's pane.
fn wake_recipient(
    mission: &crate::missions::Mission,
    wake: &PendingWake,
    ledger: Option<&Ledger>,
) -> String {
    wake.to
        .clone()
        .or_else(|| mission.conductor.agent_id.clone())
        .unwrap_or_else(|| recipient_key(ledger, &mission.conductor.actor))
}

/// The identity of live registration `agent`.
fn registered_actor(ledger: Option<&Ledger>, agent: &str) -> Option<Actor> {
    ledger?
        .agents
        .iter()
        .find(|record| record.id == agent && !record.ended)
        .map(|record| record.actor.clone())
}

/// A watch whose target went quiet is its observer's cell about that target
/// (`delivery::watch`): standing until the target moves again, updated by
/// the second warning.
fn watch_cells(ledger: &Ledger) -> Vec<Addressed> {
    ledger
        .watches
        .iter()
        .filter(|watch| watch.warning_count > 0 && !watch.parent.code_owned())
        .filter_map(|watch| {
            let first = watch.first_warning_at_unix_ms?;
            let revision = first.saturating_add(
                u64::from(watch.warning_count.saturating_sub(1))
                    * crate::delivery::SECOND_WARNING_MS,
            );
            Some(Addressed {
                recipient: recipient_key(Some(ledger), &watch.parent),
                actor: Some(watch.parent.clone()),
                cell: Cell {
                    subject: child_subject(&watch.target.pane_id),
                    line: crate::delivery::watch::warning_line(watch, revision),
                    news: false,
                    revision,
                },
            })
        })
        .collect()
}

/// The changed cells as the prompt's context shows them, after the letters.
pub(crate) fn render(cells: &[Cell]) -> String {
    let mut text = String::from(HEADING);
    for cell in cells {
        let mut lines = cell.line.lines();
        if let Some(first) = lines.next() {
            text.push_str("- ");
            text.push_str(first);
            text.push('\n');
        }
        for line in lines {
            text.push_str("  ");
            text.push_str(line);
            text.push('\n');
        }
    }
    text
}
