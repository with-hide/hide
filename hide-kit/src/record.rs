//! `~/.hide/kit/installed.json`: which parts the kit has put on this machine.
//!
//! It is the only thing that tells "the operator took it away" from "never
//! installed" (D-26). A part enters it the first time the kit finds it in
//! place and leaves it only when the machine is removed from Hide.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use hide_agent_hooks::codex_trust::HookEntry;
use serde::{Deserialize, Serialize};

use crate::ComponentId;

const FORMAT: u32 = 1;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Record {
    format: u32,
    installed: BTreeSet<String>,
    /// The last command destination this kit actually observed installed.
    /// This proves ownership even after an old package has been deleted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cli_destination: Option<PathBuf>,
    /// One-time retirements of older layouts this machine has finished, so a
    /// later pass does not ask again (PRD hide-home-layout D-14). A build
    /// that does not know the field ignores it.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    retired: BTreeSet<String>,
    /// The operator's explicit choice per agent (issue #517), by adapter id:
    /// `true` switched on, `false` switched off. An agent with no entry has
    /// made no choice and is on only when its adapter says it is by default.
    /// A build that does not know the field ignores it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    agents: BTreeMap<String, bool>,
    /// The machine's first-run agent choice has not been answered: the hold
    /// wrote the record and no explicit agent choice has come since. Held
    /// here, with the choices it governs, so one file says both whether the
    /// operator was asked and what they answered. A record without the field
    /// is one that predates the choice, so it was never held.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    awaiting_choice: bool,
    /// The entries Herdr's integration install added to an agent's hook file
    /// when the kit ran it, by adapter id (PRD codex-herdr-hook-trust): what
    /// Herdr wrote, as the kit saw it, so Codex's trust for it is for exactly
    /// those bytes. Read only while the record also holds that agent's
    /// `herdr:<agent>` piece; an integration the operator installed first has
    /// neither. A build that does not know the field ignores it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    herdr_hooks: BTreeMap<String, Vec<HookEntry>>,
    /// There was no record file when this one was loaded: the machine has
    /// never had the kit applied. Never written.
    #[serde(skip)]
    fresh: bool,
}

impl Record {
    pub(crate) fn contains(&self, id: ComponentId) -> bool {
        self.installed.contains(id.code())
    }

    pub(crate) fn insert(&mut self, id: ComponentId) {
        self.installed.insert(id.code().to_owned());
    }

    /// Whether the kit has put the piece with this code on the machine
    /// (`hook:<agent>`, `skill:<folder>`); the same set that holds the parts.
    pub(crate) fn contains_piece(&self, code: &str) -> bool {
        self.installed.contains(code)
    }

    pub(crate) fn insert_piece(&mut self, code: &str) -> bool {
        self.installed.insert(code.to_owned())
    }

    /// Forgets a piece, and with it the hook entries learned for it.
    pub(crate) fn forget_piece(&mut self, code: &str) -> bool {
        if let Some(agent) = code.strip_prefix("herdr:") {
            self.herdr_hooks.remove(agent);
        }
        self.installed.remove(code)
    }

    /// The hook entries the kit saw Herdr's integration for `agent` write,
    /// only while the record holds that integration as the kit's own.
    pub(crate) fn herdr_hook_entries(&self, agent: &str) -> &[HookEntry] {
        if !self.contains_piece(&format!("herdr:{agent}")) {
            return &[];
        }
        self.herdr_hooks.get(agent).map_or(&[], Vec::as_slice)
    }

    /// Keeps what Herdr's integration for `agent` wrote; true when that
    /// changed the record.
    pub(crate) fn set_herdr_hook_entries(&mut self, agent: &str, entries: Vec<HookEntry>) -> bool {
        if entries.is_empty() {
            return self.herdr_hooks.remove(agent).is_some();
        }
        self.herdr_hooks.insert(agent.to_owned(), entries.clone()) != Some(entries)
    }

    /// Whether the machine has never had the kit applied.
    pub(crate) fn is_fresh(&self) -> bool {
        self.fresh
    }

    /// Whether the machine still waits for the operator's first agent choice.
    pub(crate) fn awaiting_choice(&self) -> bool {
        self.awaiting_choice
    }

    /// Marks the first-run choice as asked and unanswered; true when it changed.
    pub(crate) fn await_choice(&mut self) -> bool {
        !std::mem::replace(&mut self.awaiting_choice, true)
    }

    /// Marks it answered; true when it changed.
    pub(crate) fn answer_choice(&mut self) -> bool {
        std::mem::replace(&mut self.awaiting_choice, false)
    }

    pub(crate) fn agent_choice(&self, id: &str) -> Option<bool> {
        self.agents.get(id).copied()
    }

    /// Records the operator's choice; true when it changed the record.
    pub(crate) fn set_agent_choice(&mut self, id: &str, on: bool) -> bool {
        self.agents.insert(id.to_owned(), on) != Some(on)
    }

    /// Drops the choice recorded for an agent Hide no longer knows; true when
    /// there was one.
    pub(crate) fn forget_agent_choice(&mut self, id: &str) -> bool {
        self.agents.remove(id).is_some()
    }

    pub(crate) fn owns_cli(&self, destination: &Path) -> bool {
        self.cli_destination.as_deref().is_some_and(|known| {
            known == destination
                || match (
                    hide_platform::path::to_wire(known),
                    hide_platform::path::to_wire(destination),
                ) {
                    (Ok(known), Ok(destination)) => known == destination,
                    _ => false,
                }
        })
    }

    pub(crate) fn remember_cli(&mut self, destination: PathBuf) -> bool {
        if self.cli_destination.as_ref() == Some(&destination) {
            return false;
        }
        self.cli_destination = Some(destination);
        true
    }

    pub(crate) fn has_retired(&self, what: &str) -> bool {
        self.retired.contains(what)
    }

    pub(crate) fn mark_retired(&mut self, what: &str) {
        self.retired.insert(what.to_owned());
    }
}

/// The folders under HOME that hold the kit's state and retirement receipt.
pub(crate) const STATE_PARTS: [&str; 2] = [crate::layout::HIDE_HOME, "kit"];

pub fn kit_state_dir(home: &Path) -> PathBuf {
    home.join(STATE_PARTS[0]).join(STATE_PARTS[1])
}

/// The kit's state folder, made private when it is missing, or why the kit
/// keeps nothing there (`crate::private_dirs`).
pub(crate) fn private_state_dir(home: &Path, create: bool) -> Result<PathBuf, crate::KitRefusal> {
    crate::private_dirs(home, &STATE_PARTS, create)
}

fn path(home: &Path) -> PathBuf {
    kit_state_dir(home).join("installed.json")
}

/// The record, an empty one when the machine has none, or why it could not
/// be read. An unreadable record is not treated as empty: that would read
/// every part the operator removed as never installed and put it back.
/// The machine's record, or why a pass must stop before changing anything:
/// without it the kit cannot tell which parts are Hide's, and one written by
/// a newer build is that build's to change.
pub(crate) fn load(home: &Path) -> Result<Record, crate::KitRefusal> {
    private_state_dir(home, false)?;
    let path = path(home);
    let unreadable = |detail: String| crate::KitRefusal::record(&path, detail);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Record {
                format: FORMAT,
                fresh: true,
                ..Record::default()
            });
        }
        Err(error) => {
            return Err(unreadable(format!(
                "{} could not be read: {error}",
                path.display()
            )));
        }
    };
    let record: Record = serde_json::from_slice(&bytes).map_err(|error| {
        unreadable(format!(
            "{} is not a record Hide can read: {error}",
            path.display()
        ))
    })?;
    if record.format != FORMAT {
        return Err(unreadable(format!(
            "{} has format {}, which this build does not read",
            path.display(),
            record.format
        )));
    }
    Ok(record)
}

pub(crate) fn save(home: &Path, record: &Record) -> Result<(), String> {
    let record = Record {
        format: FORMAT,
        installed: record.installed.clone(),
        cli_destination: record.cli_destination.clone(),
        retired: record.retired.clone(),
        agents: record.agents.clone(),
        awaiting_choice: record.awaiting_choice,
        herdr_hooks: record.herdr_hooks.clone(),
        fresh: false,
    };
    let mut bytes = serde_json::to_vec_pretty(&record).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    private_state_dir(home, true)?;
    crate::write_atomically(&path(home), &bytes, hide_platform::fs::Access::Private)
}

pub(crate) fn forget(home: &Path) -> Result<(), String> {
    private_state_dir(home, false)?;
    let path = path(home);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("{} could not be removed: {error}", path.display())),
    }
}
