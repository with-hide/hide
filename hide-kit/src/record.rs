//! `~/.hide/kit/installed.json`: which parts the kit has put on this machine.
//!
//! It is the only thing that tells "the operator took it away" from "never
//! installed" (D-26). A part enters it the first time the kit finds it in
//! place and leaves it only when the machine is removed from Hide.
//!
//! An account can hold more than one Hide installation (a desktop app and a
//! device helper another machine's Hide put there), each with its own kit:
//! what one installation put on the machine, its `hide` command and its hook
//! route are kept apart by installation ([`InstallRecord`]), so neither reads
//! the other's as its own (docs/agent-hooks.md, More than one Hide on an
//! account). The operator's agent choices and the retirements stay the
//! account's.
//!
//! What an installation put in an agent's account is kept by target
//! (docs/agent-hooks.md, Profile accounts): the pieces in each agent's
//! default account stay in `installed`, where every earlier build reads and
//! writes them, and the built-in default profile refers to them for good, so
//! no profile's going ever takes them out. Each other account a profile runs
//! in is a [`TargetRecord`] in `targets`, keyed by the agent and the account's
//! own folder as the file system names it, with the pieces put there and the
//! profiles that refer to it: two profiles of one folder share one target,
//! and its pieces come out only when the last of them goes. A record from
//! before profiles has no `targets` and reads as the default target alone.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use hide_agent_adapter::{AccountRef, ProfileId};
use hide_agent_hooks::codex_trust::HookEntry;
use serde::{Deserialize, Serialize};

use crate::ComponentId;

const FORMAT: u32 = 1;

/// What one installation's kit put on the machine.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
struct InstallRecord {
    #[serde(default)]
    installed: BTreeSet<String>,
    /// The last command destination this kit actually observed installed.
    /// This proves ownership even after an old package has been deleted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cli_destination: Option<PathBuf>,
    /// The daemon that owns this installation's hook route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    state_route: Option<PathBuf>,
    /// Each profile account this installation put pieces in, by its key
    /// (`crate::targets::key`). A build that does not know the field
    /// ignores it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    targets: BTreeMap<String, TargetRecord>,
}

/// One account a profile runs in that is not its agent's default account:
/// what the kit put there, and which profiles refer to it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct TargetRecord {
    /// The agent's adapter id.
    pub(crate) agent: String,
    /// The account as the file system names it (a folder with every link
    /// resolved, or omp's name), which reaches the folder the pieces went
    /// into once no profile names it, whatever a link it was spelled
    /// through leads to by then.
    pub(crate) account: AccountRef,
    /// The pieces put there, by the codes `installed` uses.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub(crate) pieces: BTreeSet<String>,
    /// The skills folder Hide's stubs went into, as the file system names
    /// it: another installation's target holding the same folder keeps the
    /// stubs when this one goes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) skill_root: Option<PathBuf>,
    /// The profiles that run in it. A target with none is one whose pieces
    /// could not all be taken out yet.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub(crate) refs: BTreeSet<ProfileId>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Record {
    format: u32,
    /// The desktop app's part, where every build before installations were
    /// kept apart read and wrote the account's: an installation with no
    /// entry in `installs` yet starts from it.
    #[serde(flatten)]
    app: InstallRecord,
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
    /// Every other installation's part of the record, by its name
    /// (`hide_agent_hooks::route::install_of_root`); written, empty or not,
    /// by every build that keeps installations apart, so a record without it
    /// is an earlier build's, whose part was the account's. A build that does
    /// not know the field ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    installs: Option<BTreeMap<String, InstallRecord>>,
    /// The installation this record was loaded for. Never written.
    #[serde(skip)]
    install: String,
    /// There was no record file when this one was loaded: the machine has
    /// never had the kit applied. Never written.
    #[serde(skip)]
    fresh: bool,
}

impl Record {
    /// The part of the record that is the loaded installation's.
    fn mine(&self) -> Option<&InstallRecord> {
        if is_app(&self.install) {
            return Some(&self.app);
        }
        self.installs.as_ref()?.get(&self.install)
    }

    fn mine_mut(&mut self) -> &mut InstallRecord {
        if is_app(&self.install) {
            return &mut self.app;
        }
        self.installs
            .get_or_insert_default()
            .entry(self.install.clone())
            .or_default()
    }

    pub(crate) fn state_route(&self) -> Option<&Path> {
        self.mine()?.state_route.as_deref()
    }

    pub(crate) fn remember_state_route(&mut self, route: PathBuf) -> bool {
        if self.state_route() == Some(route.as_path()) {
            return false;
        }
        self.mine_mut().state_route = Some(route);
        true
    }

    pub(crate) fn contains(&self, id: ComponentId) -> bool {
        self.contains_piece(id.code())
    }

    pub(crate) fn insert(&mut self, id: ComponentId) {
        self.mine_mut().installed.insert(id.code().to_owned());
    }

    /// Whether the kit has put the piece with this code on the machine
    /// (`hook:<agent>`, `skill:<folder>`); the same set that holds the parts.
    pub(crate) fn contains_piece(&self, code: &str) -> bool {
        self.mine()
            .is_some_and(|mine| mine.installed.contains(code))
    }

    pub(crate) fn insert_piece(&mut self, code: &str) -> bool {
        self.mine_mut().installed.insert(code.to_owned())
    }

    /// Forgets a piece, and with it the hook entries learned for it.
    pub(crate) fn forget_piece(&mut self, code: &str) -> bool {
        if let Some(agent) = code.strip_prefix("herdr:") {
            self.herdr_hooks.remove(agent);
        }
        self.mine_mut().installed.remove(code)
    }

    /// The hook entries the kit saw Herdr's integration for `agent` write,
    /// only while the record holds that integration as the kit's own.
    pub(crate) fn herdr_hook_entries(&self, agent: &str) -> &[HookEntry] {
        if !self.contains_piece(&format!("herdr:{agent}")) {
            return &[];
        }
        self.herdr_hooks.get(agent).map_or(&[], Vec::as_slice)
    }

    /// Keeps what Herdr's integration for `agent` wrote, as earlier builds
    /// did; true when that changed the record. Only a fixture writes it now,
    /// one of the Unix kit cases (`tests::herdr_cases`).
    #[cfg(all(test, unix))]
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
        let known = self.mine().and_then(|mine| mine.cli_destination.as_deref());
        known.is_some_and(|known| {
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

    /// The profile accounts this installation put pieces in, by key.
    pub(crate) fn targets(&self) -> &BTreeMap<String, TargetRecord> {
        static NONE: BTreeMap<String, TargetRecord> = BTreeMap::new();
        self.mine().map_or(&NONE, |mine| &mine.targets)
    }

    /// The profile account `key`, made with `make` the first time.
    pub(crate) fn target_mut(
        &mut self,
        key: &str,
        make: impl FnOnce() -> TargetRecord,
    ) -> &mut TargetRecord {
        self.mine_mut()
            .targets
            .entry(key.to_owned())
            .or_insert_with(make)
    }

    /// Whether another installation on the account keeps Hide's skill stubs
    /// in a folder (`TargetRecord::skill_root`) that holds the real place
    /// `path`, so this one's going leaves it.
    pub(crate) fn skill_kept_elsewhere(&self, path: &Path) -> bool {
        let others = std::iter::once((hide_agent_hooks::route::APP_INSTALL, &self.app))
            .chain(
                self.installs
                    .iter()
                    .flatten()
                    .map(|(install, part)| (install.as_str(), part)),
            )
            .filter(|(install, _)| {
                !(is_app(install) && is_app(&self.install)) && *install != self.install
            });
        others
            .flat_map(|(_, part)| part.targets.values())
            .any(|target| {
                target
                    .skill_root
                    .as_deref()
                    .is_some_and(|root| path.starts_with(root))
                    && target
                        .pieces
                        .iter()
                        .any(|piece| piece.starts_with("skill:"))
            })
    }

    /// Forgets the profile account `key`; true when it was recorded.
    pub(crate) fn forget_target(&mut self, key: &str) -> bool {
        self.mine_mut().targets.remove(key).is_some()
    }

    pub(crate) fn remember_cli(&mut self, destination: PathBuf) -> bool {
        let mine = self.mine_mut();
        if mine.cli_destination.as_ref() == Some(&destination) {
            return false;
        }
        mine.cli_destination = Some(destination);
        true
    }
}

/// Whether `install` is the desktop app's, whose part is the record's own
/// fields. A record loaded with no installation (a test's) is the app's.
fn is_app(install: &str) -> bool {
    install.is_empty() || install == hide_agent_hooks::route::APP_INSTALL
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
/// It is read for the installation `install`, whose part of it a pass reads
/// and changes; one that has none yet starts from what a build before
/// installations were kept apart recorded for the account.
pub(crate) fn load(home: &Path, install: &str) -> Result<Record, crate::KitRefusal> {
    private_state_dir(home, false)?;
    let path = path(home);
    let unreadable = |detail: String| crate::KitRefusal::record(&path, detail);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Record {
                format: FORMAT,
                install: install.to_owned(),
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
    let mut record: Record = serde_json::from_slice(&bytes).map_err(|error| {
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
    // An earlier build's record is the account's: another installation
    // starts from it, once, as the app keeps reading it as its own.
    if !is_app(install) && record.installs.is_none() {
        record.installs = Some(BTreeMap::from([(install.to_owned(), record.app.clone())]));
    }
    record.install = install.to_owned();
    Ok(record)
}

pub(crate) fn save(home: &Path, record: &Record) -> Result<(), String> {
    let record = Record {
        format: FORMAT,
        app: record.app.clone(),
        agents: record.agents.clone(),
        awaiting_choice: record.awaiting_choice,
        herdr_hooks: record.herdr_hooks.clone(),
        installs: Some(record.installs.clone().unwrap_or_default()),
        install: String::new(),
        fresh: false,
    };
    let mut bytes = serde_json::to_vec_pretty(&record).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    private_state_dir(home, true)?;
    crate::write_atomically(&path(home), &bytes, hide_platform::fs::Access::Private)
}

/// Forgets the installation `install`'s part of the record, and the record
/// with it when no other installation on the account keeps a part. A record
/// that cannot be read stays as it is: it may hold another installation's
/// part and the account's choices, which removing it would lose.
pub(crate) fn forget(home: &Path, install: &str) -> Result<(), String> {
    private_state_dir(home, false)?;
    let path = path(home);
    let mut record = load(home, install).map_err(|refusal| refusal.detail)?;
    if record.fresh {
        return Ok(());
    }
    let others = record.installs.get_or_insert_default();
    if is_app(install) {
        record.app = InstallRecord::default();
    } else {
        others.remove(install);
    }
    if !record.installs.as_ref().is_none_or(BTreeMap::is_empty)
        || record.app != InstallRecord::default()
    {
        return save(home, &record);
    }
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("{} could not be removed: {error}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The app and a device helper on one account each keep what they put
    /// there, starting from what a build before them recorded, and one going
    /// leaves the other's.
    #[test]
    fn each_installation_keeps_its_own_part_of_the_record() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let device = "root:/helper";
        private_state_dir(home, true).unwrap();
        std::fs::write(
            path(home),
            r#"{"format":1,"installed":["cli"],"state_route":"/earlier"}"#,
        )
        .unwrap();

        // Both start from what the earlier build recorded for the account.
        let mut helper = load(home, device).unwrap();
        assert!(helper.contains(ComponentId::Cli));
        assert_eq!(helper.state_route(), Some(Path::new("/earlier")));
        assert!(helper.remember_state_route(PathBuf::from("/device")));
        save(home, &helper).unwrap();

        let mut app = load(home, APP).unwrap();
        assert!(app.contains(ComponentId::Cli));
        assert_eq!(app.state_route(), Some(Path::new("/earlier")));
        app.insert(ComponentId::CodexHook);
        assert!(app.remember_state_route(PathBuf::from("/app")));
        save(home, &app).unwrap();
        assert!(!load(home, device).unwrap().contains(ComponentId::CodexHook));

        // An installation that comes later starts with nothing of theirs.
        let later = load(home, "root:/later").unwrap();
        assert!(!later.contains(ComponentId::Cli));
        assert_eq!(later.state_route(), None);

        assert_eq!(
            load(home, APP).unwrap().state_route(),
            Some(Path::new("/app"))
        );
        assert_eq!(
            load(home, device).unwrap().state_route(),
            Some(Path::new("/device"))
        );
        forget(home, device).unwrap();
        assert_eq!(
            load(home, APP).unwrap().state_route(),
            Some(Path::new("/app"))
        );
        forget(home, APP).unwrap();
        assert!(!path(home).exists());
    }

    /// A record this build cannot read, one a newer build wrote or one that
    /// is damaged, may hold another installation's part and the account's
    /// choices: forgetting one installation refuses and leaves its bytes.
    #[test]
    fn an_unreadable_record_is_kept_when_an_installation_is_forgotten() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        private_state_dir(home, true).unwrap();
        for bytes in [
            r#"{"format":99,"installed":["cli"],"installs":{"root:/helper":{"installed":["cli"]}}}"#,
            r#"{"format":1,"installed":["cli"],"installs":{"root:/helper":"#,
        ] {
            std::fs::write(path(home), bytes).unwrap();
            for install in [APP, "root:/helper"] {
                assert!(forget(home, install).is_err(), "{install}: {bytes}");
                assert_eq!(std::fs::read_to_string(path(home)).unwrap(), bytes);
            }
        }
    }

    const APP: &str = hide_agent_hooks::route::APP_INSTALL;
}
