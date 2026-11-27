//! The two records of a pairing (PRD D-29, D-30, D-62).
//!
//! [`Grants`] is the host's side, `peers/grants.json`: who may run work
//! here. [`Known`] is the guest's side, `peers/known.json`: the hosts this
//! machine may run work on, and where to find them.
//!
//! Every change is saved first: the whole new record is built in memory and
//! written, and only when the write succeeded does the store hold it. A
//! failed write changes nothing, on disk or in memory, so nothing ever acts
//! on a grant that is not saved.

use std::io;
use std::path::{Path, PathBuf};

use hide_peer_protocol::hello::GrantState;
use hide_peer_protocol::pairing::ConfirmState;
use hide_peer_protocol::{Candidate, PeerAddr, PeerKey};
use hide_platform::fs::{Access, atomic, private};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::limits;

const VERSION: u32 = 1;

/// The most bytes either record takes. A change that would pass it is
/// refused as full.
const MAX_STORE_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub enum StoreError {
    /// The record is at its cap: the operator removes a machine first.
    Full,
    /// No such grant or host.
    Unknown,
    Io(io::Error),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full => f.write_str("the peer record is full"),
            Self::Unknown => f.write_str("no such peer record"),
            Self::Io(error) => write!(f, "the peer record could not be saved: {error}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<io::Error> for StoreError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

fn load<T: DeserializeOwned + Default>(path: &Path) -> io::Result<T> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
            // A record that cannot be read is never replaced by an empty
            // one: that would drop every grant silently.
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} is not a peer record: {error}", path.display()),
            )
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) => Err(error),
    }
}

fn save<T: Serialize>(path: &Path, record: &T) -> Result<(), StoreError> {
    let bytes = serde_json::to_vec_pretty(record).map_err(io::Error::other)?;
    if bytes.len() > MAX_STORE_BYTES {
        return Err(StoreError::Full);
    }
    if let Some(parent) = path.parent() {
        private::create_dir_all(parent)?;
    }
    atomic::write_file(path, &bytes, Access::Private)?;
    Ok(())
}

fn new_id() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| io::Error::other(format!("no random bytes: {error}")))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantStatus {
    /// Saved, waiting for the guest's confirm. It opens nothing.
    Pending,
    Active,
    Paused,
}

/// How the pairing was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Via {
    Link,
    Ssh,
    Wsl,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub id: String,
    /// The guest's connection id.
    pub key: PeerKey,
    /// The guest's product device id.
    pub device: String,
    /// The name the guest gave itself.
    pub name: String,
    /// The name this machine's operator chose for it, which it is shown
    /// under instead of `name` ([`Grant::shown_name`]); none until they do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chosen_name: Option<String>,
    pub state: GrantStatus,
    /// The generation its streams run under. It moves on every change.
    pub generation: u64,
    pub created_unix_ms: u64,
    pub via: Via,
}

impl Grant {
    /// The name the machine is shown under: the operator's choice, else its own.
    pub fn shown_name(&self) -> &str {
        self.chosen_name.as_deref().unwrap_or(&self.name)
    }
}

impl Host {
    /// The name the machine is shown under: the operator's choice, else its own.
    pub fn shown_name(&self) -> &str {
        self.chosen_name.as_deref().unwrap_or(&self.name)
    }
}

/// A removed grant, remembered so its guest hears "removed".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tombstone {
    pub key: PeerKey,
    pub id: String,
    pub revoked_unix_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct GrantsRecord {
    #[serde(default)]
    version: u32,
    /// The last generation handed out. It only rises, so a generation is
    /// never reused, across removals and restarts.
    #[serde(default)]
    epoch: u64,
    #[serde(default)]
    grants: Vec<Grant>,
    #[serde(default)]
    tombstones: Vec<Tombstone>,
}

/// What a change means for one guest: the state and generation its streams
/// now run under. Whoever holds the connection applies it after the save.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    pub key: PeerKey,
    pub state: GrantState,
    pub generation: u64,
}

/// `peers/grants.json`.
pub struct Grants {
    path: PathBuf,
    record: GrantsRecord,
}

/// Whether `grant` still counts at `now_unix_ms`. A pending grant waits
/// [`limits::PENDING_FOR`] for its confirm and counts for nothing after:
/// this is read wherever a grant is, so an expired one holds no place and
/// keeps no connection, whether or not a write has dropped it yet.
fn waits(grant: &Grant, now_unix_ms: u64) -> bool {
    grant.state != GrantStatus::Pending
        || now_unix_ms.saturating_sub(grant.created_unix_ms)
            < limits::PENDING_FOR.as_millis() as u64
}

/// Whether a removed grant is still remembered at `now_unix_ms`.
fn remembered(stone: &Tombstone, now_unix_ms: u64) -> bool {
    now_unix_ms.saturating_sub(stone.revoked_unix_ms) < limits::TOMBSTONE_FOR.as_millis() as u64
}

impl Grants {
    pub fn open(path: PathBuf) -> io::Result<Self> {
        let record = load(&path)?;
        Ok(Self { path, record })
    }

    /// The grants that count at `now_unix_ms`: the settled ones and the
    /// pending ones still waiting for their confirm.
    pub fn list(&self, now_unix_ms: u64) -> impl Iterator<Item = &Grant> {
        self.record
            .grants
            .iter()
            .filter(move |grant| waits(grant, now_unix_ms))
    }

    /// How long `key`'s pending grant still waits for its confirm, when it
    /// has one that does.
    pub fn pending_left(&self, key: &PeerKey, now_unix_ms: u64) -> Option<std::time::Duration> {
        self.list(now_unix_ms)
            .find(|grant| grant.key == *key && grant.state == GrantStatus::Pending)
            .map(|grant| {
                let waited = now_unix_ms.saturating_sub(grant.created_unix_ms);
                limits::PENDING_FOR.saturating_sub(std::time::Duration::from_millis(waited))
            })
    }

    /// The guest's settled grant: active or paused.
    pub fn current(&self, key: &PeerKey) -> Option<&Grant> {
        self.record
            .grants
            .iter()
            .find(|grant| grant.key == *key && grant.state != GrantStatus::Pending)
    }

    /// What `key`'s streams run under at `now_unix_ms`. A pending grant
    /// opens nothing, and so does a removed one.
    pub fn state_of(&self, key: &PeerKey, now_unix_ms: u64) -> Change {
        let (state, generation) = match self.current(key) {
            Some(grant) if grant.state == GrantStatus::Active => {
                (GrantState::Active, grant.generation)
            }
            Some(_) => (GrantState::Paused, 0),
            None if self.pending_left(key, now_unix_ms).is_some() => (GrantState::Pending, 0),
            None if self
                .record
                .tombstones
                .iter()
                .any(|stone| stone.key == *key && remembered(stone, now_unix_ms)) =>
            {
                (GrantState::Revoked, 0)
            }
            None => (GrantState::None, 0),
        };
        Change {
            key: *key,
            state,
            generation,
        }
    }

    /// Saves `next` and only then holds it.
    fn commit(&mut self, mut next: GrantsRecord) -> Result<(), StoreError> {
        next.version = VERSION;
        save(&self.path, &next)?;
        self.record = next;
        Ok(())
    }

    /// Saves a pending grant for a guest the operator allowed. It replaces
    /// an earlier pending one for the same key and leaves a settled grant
    /// alone until the confirm (D-30).
    pub fn grant_pending(
        &mut self,
        key: PeerKey,
        device: &str,
        name: &str,
        via: Via,
        now_unix_ms: u64,
    ) -> Result<Grant, StoreError> {
        // What stopped waiting holds no place among the machines counted.
        let mut next = self.swept(now_unix_ms);
        // The name the operator chose stays through a pairing again, also
        // one chosen while only a pending grant stood.
        let chosen_name = next
            .grants
            .iter()
            .find(|grant| grant.key == key && grant.chosen_name.is_some())
            .and_then(|grant| grant.chosen_name.clone());
        next.grants
            .retain(|grant| !(grant.key == key && grant.state == GrantStatus::Pending));
        // The count is of machines: a settled grant and the pending one
        // that will replace it are one machine.
        let machines = next.grants.iter().filter(|grant| grant.key != key).count();
        if machines >= limits::MAX_GRANTS {
            return Err(StoreError::Full);
        }
        let grant = Grant {
            id: new_id()?,
            key,
            device: device.to_owned(),
            name: name.to_owned(),
            chosen_name,
            state: GrantStatus::Pending,
            generation: 0,
            created_unix_ms: now_unix_ms,
            via,
        };
        next.grants.push(grant.clone());
        self.commit(next)?;
        Ok(grant)
    }

    /// The guest's confirm: a pending grant becomes active and replaces the
    /// settled one. It answers the same however often it is sent, and never
    /// revives a pause or a removal.
    pub fn confirm(
        &mut self,
        key: &PeerKey,
        id: &str,
        now_unix_ms: u64,
    ) -> Result<(ConfirmState, Option<Change>), StoreError> {
        self.expire(now_unix_ms)?;
        let Some(found) = self
            .record
            .grants
            .iter()
            .find(|grant| grant.key == *key && grant.id == id)
        else {
            return Ok((ConfirmState::Unknown, None));
        };
        match found.state {
            GrantStatus::Active => return Ok((ConfirmState::Active, None)),
            GrantStatus::Paused => return Ok((ConfirmState::Paused, None)),
            GrantStatus::Pending => {}
        }
        let mut next = self.record.clone();
        next.epoch += 1;
        let generation = next.epoch;
        let mut replaced = Vec::new();
        next.grants.retain(|grant| {
            let old = grant.key == *key && grant.id != id;
            if old {
                replaced.push(Tombstone {
                    key: *key,
                    id: grant.id.clone(),
                    revoked_unix_ms: now_unix_ms,
                });
            }
            !old
        });
        next.tombstones.extend(replaced);
        trim_tombstones(&mut next.tombstones);
        for grant in &mut next.grants {
            if grant.key == *key {
                grant.state = GrantStatus::Active;
                grant.generation = generation;
            }
        }
        self.commit(next)?;
        Ok((ConfirmState::Active, Some(self.state_of(key, now_unix_ms))))
    }

    /// Moves `key`'s settled grant to `to`. `None` when it is there
    /// already: the same request sent twice saves nothing and moves no
    /// generation, so the streams running under the grant go on.
    fn settle(&mut self, key: &PeerKey, to: GrantStatus) -> Result<Option<Change>, StoreError> {
        let mut next = self.record.clone();
        next.epoch += 1;
        let generation = next.epoch;
        let grant = next
            .grants
            .iter_mut()
            .find(|grant| grant.key == *key && grant.state != GrantStatus::Pending)
            .ok_or(StoreError::Unknown)?;
        if grant.state == to {
            return Ok(None);
        }
        grant.state = to;
        grant.generation = generation;
        self.commit(next)?;
        Ok(Some(Change {
            key: *key,
            state: match to {
                GrantStatus::Active => GrantState::Active,
                _ => GrantState::Paused,
            },
            generation: if to == GrantStatus::Active {
                generation
            } else {
                0
            },
        }))
    }

    /// Keeps `name`, what the guest under `key` calls itself now, as the
    /// name of its grants. A name opens nothing, so the generation stays
    /// and no stream is told. Whether anything changed.
    pub fn take_name(&mut self, key: &PeerKey, name: &str) -> Result<bool, StoreError> {
        let mut next = self.record.clone();
        let mut changed = false;
        for grant in next.grants.iter_mut().filter(|grant| grant.key == *key) {
            if grant.name != name {
                grant.name = name.to_owned();
                changed = true;
            }
        }
        if changed {
            self.commit(next)?;
        }
        Ok(changed)
    }

    pub fn pause(&mut self, key: &PeerKey) -> Result<Option<Change>, StoreError> {
        self.settle(key, GrantStatus::Paused)
    }

    pub fn resume(&mut self, key: &PeerKey) -> Result<Option<Change>, StoreError> {
        self.settle(key, GrantStatus::Active)
    }

    /// Gives `key`'s grants the name this machine's operator chose, or with
    /// `None` takes it back to the name the guest gave itself. A name opens
    /// nothing, so the generation stays and no stream is told. Whether a
    /// grant for `key` was there.
    pub fn rename(&mut self, key: &PeerKey, chosen: Option<&str>) -> Result<bool, StoreError> {
        let mut next = self.record.clone();
        let mut found = false;
        let mut changed = false;
        for grant in next.grants.iter_mut().filter(|grant| grant.key == *key) {
            found = true;
            if grant.chosen_name.as_deref() != chosen {
                grant.chosen_name = chosen.map(str::to_owned);
                changed = true;
            }
        }
        if changed {
            self.commit(next)?;
        }
        Ok(found)
    }

    /// Removes `key`'s grants. With `id`, only when that grant is the one
    /// held now, so a late removal of an old grant does not take the new
    /// one. `None` when there was nothing to remove: the answer to the
    /// guest is the same either way.
    pub fn revoke(
        &mut self,
        key: &PeerKey,
        id: Option<&str>,
        now_unix_ms: u64,
    ) -> Result<Option<Change>, StoreError> {
        let matches = |grant: &Grant| grant.key == *key && id.is_none_or(|id| grant.id == id);
        if !self.record.grants.iter().any(matches) {
            return Ok(None);
        }
        let mut next = self.record.clone();
        next.epoch += 1;
        for grant in next.grants.iter().filter(|grant| matches(grant)) {
            next.tombstones.push(Tombstone {
                key: *key,
                id: grant.id.clone(),
                revoked_unix_ms: now_unix_ms,
            });
        }
        next.grants.retain(|grant| !matches(grant));
        trim_tombstones(&mut next.tombstones);
        self.commit(next)?;
        Ok(Some(self.state_of(key, now_unix_ms)))
    }

    /// The record without the pending grants and tombstones past their
    /// time.
    fn swept(&self, now_unix_ms: u64) -> GrantsRecord {
        let mut next = self.record.clone();
        next.grants.retain(|grant| waits(grant, now_unix_ms));
        next.tombstones
            .retain(|stone| remembered(stone, now_unix_ms));
        next
    }

    /// Drops pending grants and tombstones past their time from the file.
    /// Neither moves a generation: a pending grant never opened anything.
    /// Every read already leaves them out; this only stops keeping them.
    pub fn expire(&mut self, now_unix_ms: u64) -> Result<(), StoreError> {
        let next = self.swept(now_unix_ms);
        if next.grants.len() == self.record.grants.len()
            && next.tombstones.len() == self.record.tombstones.len()
        {
            return Ok(());
        }
        self.commit(next)
    }
}

/// At the cap the oldest tombstones go first.
fn trim_tombstones(tombstones: &mut Vec<Tombstone>) {
    if tombstones.len() > limits::MAX_TOMBSTONES {
        let extra = tombstones.len() - limits::MAX_TOMBSTONES;
        tombstones.drain(..extra);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnownState {
    /// Saved after the operator said the words match; the confirm is sent
    /// again at every connection until the host answers.
    Confirming,
    Active,
    /// This machine paused it and does not connect.
    PausedByMe,
    /// This machine removed it and keeps telling the host until it answers.
    RevokePending,
    /// The host says it holds no grant for this machine: it removed it, or
    /// the pairing never settled there. Nothing is dialed and nothing opens
    /// until the operator pairs again, which replaces this record, or
    /// removes it (B4, B51).
    Unpaired,
}

impl KnownState {
    /// Whether this machine itself keeps its side shut, whatever the host
    /// grants.
    pub fn held_back(self) -> bool {
        matches!(
            self,
            Self::PausedByMe | Self::RevokePending | Self::Unpaired
        )
    }

    /// Whether the host is dialed again after a drop. A removal still has
    /// to reach it; a pause and a pairing the host dropped do not.
    pub fn dialed(self) -> bool {
        !matches!(self, Self::PausedByMe | Self::Unpaired)
    }
}

/// A host this machine is a guest of.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Host {
    /// The host's connection id.
    pub key: PeerKey,
    /// The host's product device id.
    pub device: String,
    /// The name the host gave itself.
    pub name: String,
    /// The name this machine's operator chose for it, which it is shown
    /// under instead of `name` ([`Host::shown_name`]); none until they do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chosen_name: Option<String>,
    /// The grant the host gave.
    pub grant: String,
    pub state: KnownState,
    /// Where the host was last reached, in the order to try.
    #[serde(default)]
    pub candidates: Vec<Candidate>,
    /// The login this machine switched the host through, when it did: a
    /// Hide started from the programs put there over a login has no app of
    /// its own to update it, so this machine brings it to its own build
    /// until the account has one (PRD device-connect-iroh B23).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started: Option<Login>,
    /// How this machine reaches the host over SSH when no direct path opens
    ///: an alias of this account's SSH configuration, recorded
    /// only once a carrier over it proved the host's key, or `Off` once an
    /// alias reached a Hide that proved another key. Absent, the alias of
    /// [`Host::started`] is read ([`Host::ssh_alias`]). A field, not a
    /// candidate, so a build before this one still reads the record (and
    /// drops it on its next write).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<SshRoute>,
}

/// The SSH route of a host this machine is a guest of.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SshRoute {
    /// Dialed through this alias when no direct path opens.
    Alias(String),
    /// Not dialed over SSH: the alias reached a Hide that proved another
    /// key. The next `hide device add <alias>` records one again.
    Off,
}

/// Whether `alias` is a name `ssh` reads as a destination and never as an
/// option: what an alias is checked against wherever one is read or saved.
pub fn valid_alias(alias: &str) -> bool {
    !alias.is_empty()
        && alias.len() <= 255
        && !alias.starts_with('-')
        && !alias
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
}

impl Host {
    /// The alias this machine dials the host by over SSH, when it has one:
    /// the recorded route, else the login it was switched through, unless
    /// the route was turned off. An alias that is not one `ssh` reads as a
    /// destination is none.
    pub fn ssh_alias(&self) -> Option<&str> {
        let alias = match &self.ssh {
            Some(SshRoute::Alias(alias)) => alias.as_str(),
            Some(SshRoute::Off) => return None,
            None => match &self.started {
                Some(Login::Ssh(alias)) => alias.as_str(),
                Some(Login::Wsl(_)) | None => return None,
            },
        };
        valid_alias(alias).then_some(alias)
    }

    /// The address the keeper dials: the saved candidates, then the SSH
    /// alias last, built in memory and never saved. A distribution of this
    /// PC is reached through `wsl.exe` alone and takes no alias.
    pub fn dial_addr(&self) -> PeerAddr {
        let mut candidates = self.candidates.clone();
        let distribution = candidates
            .iter()
            .any(|candidate| matches!(candidate, Candidate::Wsl(_)));
        if !distribution && let Some(alias) = self.ssh_alias() {
            candidates.push(Candidate::Ssh(alias.to_owned()));
        }
        PeerAddr::new(self.key, candidates)
    }
}

/// An account on another machine as this one signs in to it without Hide.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Login {
    /// An alias of this account's SSH configuration.
    Ssh(String),
    /// A WSL distribution of this PC.
    Wsl(String),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct KnownRecord {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    hosts: Vec<Host>,
}

/// The version `peers/known.json` is written at. 2: a host's name follows
/// what it calls itself on every connection, and the name a switched host
/// had here is its `chosen_name`.
const KNOWN_VERSION: u32 = 2;

/// A record written before [`KNOWN_VERSION`] 2 kept a switched host's name
/// by never replacing `name`; that name moves to `chosen_name`, where it
/// stays shown while `name` follows the host. Read in memory on every open
/// of such a record, and saved with the first change to it, so it happens
/// once.
fn switched_names_chosen(record: &mut KnownRecord) {
    for host in &mut record.hosts {
        if host.started.is_some() && host.chosen_name.is_none() {
            host.chosen_name = Some(host.name.clone());
        }
    }
}

/// `peers/known.json`.
pub struct Known {
    path: PathBuf,
    record: KnownRecord,
}

impl Known {
    pub fn open(path: PathBuf) -> io::Result<Self> {
        let mut record: KnownRecord = load(&path)?;
        if record.version < KNOWN_VERSION {
            switched_names_chosen(&mut record);
        }
        Ok(Self { path, record })
    }

    pub fn list(&self) -> &[Host] {
        &self.record.hosts
    }

    pub fn of(&self, key: &PeerKey) -> Option<&Host> {
        self.record.hosts.iter().find(|host| host.key == *key)
    }

    fn commit(&mut self, mut next: KnownRecord) -> Result<(), StoreError> {
        next.version = KNOWN_VERSION;
        save(&self.path, &next)?;
        self.record = next;
        Ok(())
    }

    /// Saves `host`, in place of what was known under its key.
    pub fn put(&mut self, mut host: Host) -> Result<(), StoreError> {
        let mut next = self.record.clone();
        // The name the operator chose stays through a pairing again.
        if host.chosen_name.is_none() {
            host.chosen_name = next
                .hosts
                .iter()
                .find(|known| known.key == host.key)
                .and_then(|known| known.chosen_name.clone());
        }
        next.hosts.retain(|known| known.key != host.key);
        if next.hosts.len() >= limits::MAX_KNOWN {
            return Err(StoreError::Full);
        }
        next.hosts.push(host);
        self.commit(next)
    }

    pub fn set_state(&mut self, key: &PeerKey, state: KnownState) -> Result<(), StoreError> {
        let mut next = self.record.clone();
        let host = next
            .hosts
            .iter_mut()
            .find(|host| host.key == *key)
            .ok_or(StoreError::Unknown)?;
        if host.state == state {
            return Ok(());
        }
        host.state = state;
        self.commit(next)
    }

    /// Gives the host under `key` the name this machine's operator chose,
    /// or with `None` takes it back to the name the host gave itself.
    /// Whether it was known.
    pub fn rename(&mut self, key: &PeerKey, chosen: Option<&str>) -> Result<bool, StoreError> {
        let mut next = self.record.clone();
        let Some(host) = next.hosts.iter_mut().find(|host| host.key == *key) else {
            return Ok(false);
        };
        if host.chosen_name.as_deref() != chosen {
            host.chosen_name = chosen.map(str::to_owned);
            self.commit(next)?;
        }
        Ok(true)
    }

    /// Keeps `name`, what the host under `key` calls itself now, as its
    /// name. A name the operator chose, or the one its device had here when
    /// this machine switched it (B36), is its `chosen_name` and stays shown.
    /// Whether anything changed.
    pub fn take_name(&mut self, key: &PeerKey, name: &str) -> Result<bool, StoreError> {
        let mut next = self.record.clone();
        let Some(host) = next.hosts.iter_mut().find(|host| host.key == *key) else {
            return Ok(false);
        };
        if host.name == name {
            return Ok(false);
        }
        host.name = name.to_owned();
        self.commit(next)?;
        Ok(true)
    }

    /// Records `alias` as the SSH route of the host under `key`: only once a
    /// carrier over it proved that host's key. Whether
    /// anything changed.
    pub fn record_ssh(&mut self, key: &PeerKey, alias: &str) -> Result<bool, StoreError> {
        if !valid_alias(alias) {
            return Err(StoreError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not an alias ssh reads as a destination",
            )));
        }
        let mut next = self.record.clone();
        let host = next
            .hosts
            .iter_mut()
            .find(|host| host.key == *key)
            .ok_or(StoreError::Unknown)?;
        let route = Some(SshRoute::Alias(alias.to_owned()));
        if host.ssh == route {
            return Ok(false);
        }
        host.ssh = route;
        self.commit(next)?;
        Ok(true)
    }

    /// Turns the SSH route of the host under `key` off, when it is still
    /// `alias`: the alias reached a Hide that proved another key. A route
    /// that changed since the attempt that found it is left as it is, so an
    /// attempt never writes over a newer alias. [`Host::started`] stays, so
    /// a Hide this machine started there is still brought to its build.
    /// Whether it was turned off.
    pub fn refuse_ssh(&mut self, key: &PeerKey, alias: &str) -> Result<bool, StoreError> {
        let mut next = self.record.clone();
        let Some(host) = next.hosts.iter_mut().find(|host| host.key == *key) else {
            return Ok(false);
        };
        if host.ssh_alias() != Some(alias) {
            return Ok(false);
        }
        host.ssh = Some(SshRoute::Off);
        self.commit(next)?;
        Ok(true)
    }

    /// Whether anything was removed.
    pub fn remove(&mut self, key: &PeerKey) -> Result<bool, StoreError> {
        if self.of(key).is_none() {
            return Ok(false);
        }
        let mut next = self.record.clone();
        next.hosts.retain(|host| host.key != *key);
        self.commit(next)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: u64 = 60_000;

    fn key(byte: u8) -> PeerKey {
        PeerKey::from_bytes([byte; 32])
    }

    fn grants(dir: &tempfile::TempDir) -> Grants {
        Grants::open(dir.path().join("peers").join("grants.json")).unwrap()
    }

    #[test]
    fn a_grant_opens_nothing_until_the_guest_confirms() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = grants(&dir);
        let pending = store
            .grant_pending(key(1), "dev", "MacBook", Via::Link, 0)
            .unwrap();
        assert_eq!(store.state_of(&key(1), 0).state, GrantState::Pending);
        assert_eq!(store.state_of(&key(1), 0).generation, 0);

        // Another machine's confirm, or a wrong id, is not this grant's.
        assert_eq!(
            store.confirm(&key(2), &pending.id, 1).unwrap().0,
            ConfirmState::Unknown
        );
        assert_eq!(
            store.confirm(&key(1), "nope", 1).unwrap().0,
            ConfirmState::Unknown
        );

        let (state, change) = store.confirm(&key(1), &pending.id, 1).unwrap();
        let change = change.expect("the confirm changed the grant");
        assert_eq!(
            (state, change.state),
            (ConfirmState::Active, GrantState::Active)
        );
        assert!(change.generation > 0);
        // Sent again, it answers the same and moves nothing.
        assert_eq!(
            store.confirm(&key(1), &pending.id, 2).unwrap(),
            (ConfirmState::Active, None)
        );

        // What was saved is what a restart reads.
        assert_eq!(grants(&dir).state_of(&key(1), 0), change);
        assert!(private::is_private(&dir.path().join("peers").join("grants.json")).unwrap());
    }

    #[test]
    fn a_confirm_never_revives_a_pause_or_a_removal() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = grants(&dir);
        let grant = store
            .grant_pending(key(1), "dev", "MacBook", Via::Link, 0)
            .unwrap();
        store.confirm(&key(1), &grant.id, 1).unwrap();
        let active = store.state_of(&key(1), 0).generation;

        let paused = store.pause(&key(1)).unwrap().unwrap();
        assert_eq!((paused.state, paused.generation), (GrantState::Paused, 0));
        assert_eq!(
            store.confirm(&key(1), &grant.id, 2).unwrap(),
            (ConfirmState::Paused, None)
        );

        let resumed = store.resume(&key(1)).unwrap().unwrap();
        assert!(resumed.generation > active, "a generation is never reused");

        let removed = store.revoke(&key(1), None, 3).unwrap().unwrap();
        assert_eq!(removed.state, GrantState::Revoked);
        assert_eq!(
            store.confirm(&key(1), &grant.id, 4).unwrap(),
            (ConfirmState::Unknown, None)
        );
        // Removing what is not there is not an error and changes nothing.
        assert_eq!(store.revoke(&key(1), None, 5).unwrap(), None);
    }

    #[test]
    fn pairing_again_replaces_the_old_grant_only_at_the_confirm() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = grants(&dir);
        let first = store
            .grant_pending(key(1), "dev", "MacBook", Via::Link, 0)
            .unwrap();
        store.confirm(&key(1), &first.id, 1).unwrap();
        let before = store.state_of(&key(1), 0);

        let second = store
            .grant_pending(key(1), "dev", "MacBook", Via::Ssh, 2)
            .unwrap();
        assert_eq!(
            store.state_of(&key(1), 0),
            before,
            "the old grant holds while the new one waits"
        );
        // A late removal of the old grant by id does not take the new one.
        store.confirm(&key(1), &second.id, 3).unwrap();
        assert_eq!(store.revoke(&key(1), Some(&first.id), 4).unwrap(), None);
        let after = store.state_of(&key(1), 0);
        assert_eq!(after.state, GrantState::Active);
        assert!(after.generation > before.generation);
        assert_eq!(store.list(4).count(), 1, "one machine, one grant");
    }

    #[test]
    fn a_pending_grant_expires_and_the_thirty_third_machine_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = grants(&dir);
        let pending = store
            .grant_pending(key(0), "dev", "late", Via::Link, 0)
            .unwrap();
        assert_eq!(
            store.confirm(&key(0), &pending.id, 10 * MINUTE).unwrap().0,
            ConfirmState::Unknown,
            "ten minutes later the pending grant is gone"
        );
        assert_eq!(store.state_of(&key(0), 10 * MINUTE).state, GrantState::None);

        for byte in 1..=limits::MAX_GRANTS as u8 {
            store
                .grant_pending(key(byte), "dev", "guest", Via::Link, 10 * MINUTE)
                .unwrap();
        }
        assert!(matches!(
            store.grant_pending(key(200), "dev", "one too many", Via::Link, 10 * MINUTE),
            Err(StoreError::Full)
        ));
        // A machine already counted pairs again without using a new place.
        store
            .grant_pending(key(1), "dev", "guest", Via::Link, 10 * MINUTE)
            .unwrap();
    }

    #[test]
    fn a_pending_grant_past_its_time_counts_for_nothing_before_any_write_drops_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = grants(&dir);
        for byte in 1..=limits::MAX_GRANTS as u8 {
            store
                .grant_pending(key(byte), "dev", "guest", Via::Link, 0)
                .unwrap();
        }
        let late = 10 * MINUTE;
        // One millisecond earlier each still waits, and the host is full.
        assert_eq!(store.state_of(&key(1), late - 1).state, GrantState::Pending);
        assert_eq!(
            store.pending_left(&key(1), late - 1),
            Some(std::time::Duration::from_millis(1))
        );
        assert_eq!(store.list(late - 1).count(), limits::MAX_GRANTS);
        // At its time it is read as gone, with the file still holding it.
        assert_eq!(store.state_of(&key(1), late).state, GrantState::None);
        assert_eq!(store.pending_left(&key(1), late), None);
        assert_eq!(store.list(late).count(), 0);
        // And it holds no place: another machine is granted.
        store
            .grant_pending(key(200), "dev", "the next machine", Via::Link, late)
            .unwrap();
        assert_eq!(store.list(late).count(), 1);
    }

    #[test]
    fn the_same_pause_or_resume_sent_again_saves_nothing_and_moves_no_generation() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = grants(&dir);
        let grant = store
            .grant_pending(key(1), "dev", "MacBook", Via::Link, 0)
            .unwrap();
        store.confirm(&key(1), &grant.id, 1).unwrap();
        let active = store.state_of(&key(1), 2);

        // Resuming what is active: the streams under it go on.
        assert_eq!(store.resume(&key(1)).unwrap(), None);
        assert_eq!(store.state_of(&key(1), 2), active);

        let paused = store.pause(&key(1)).unwrap().expect("the first pause");
        assert_eq!(paused.state, GrantState::Paused);
        let path = dir.path().join("peers").join("grants.json");
        let saved = std::fs::read(&path).unwrap();
        assert_eq!(store.pause(&key(1)).unwrap(), None);
        assert_eq!(std::fs::read(&path).unwrap(), saved, "nothing was written");

        // A grant that is not there is still an error, not "unchanged".
        assert!(matches!(store.resume(&key(9)), Err(StoreError::Unknown)));
    }

    #[test]
    fn a_failed_save_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = grants(&dir);
        let grant = store
            .grant_pending(key(1), "dev", "MacBook", Via::Link, 0)
            .unwrap();
        store.confirm(&key(1), &grant.id, 1).unwrap();
        let before = store.state_of(&key(1), 0);

        // The record's place is taken by a folder, so the replace fails.
        let path = dir.path().join("peers").join("grants.json");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("in-the-way"), b"").unwrap();
        assert!(matches!(store.pause(&key(1)), Err(StoreError::Io(_))));
        assert_eq!(
            store.state_of(&key(1), 0),
            before,
            "memory still holds what was saved"
        );
    }

    #[test]
    fn a_damaged_record_is_an_error_not_an_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("grants.json");
        std::fs::write(&path, b"{ not json").unwrap();
        assert_eq!(
            Grants::open(path).err().unwrap().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn a_guest_remembers_its_hosts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peers").join("known.json");
        let mut known = Known::open(path.clone()).unwrap();
        let host = Host {
            key: key(7),
            device: "host-device".into(),
            name: "Windows PC".into(),
            chosen_name: None,
            grant: "g1".into(),
            state: KnownState::Confirming,
            candidates: vec![Candidate::Relay("https://relay.example".into())],
            started: None,
            ssh: None,
        };
        known.put(host.clone()).unwrap();
        known.set_state(&key(7), KnownState::Active).unwrap();
        assert!(matches!(
            known.set_state(&key(8), KnownState::Active),
            Err(StoreError::Unknown)
        ));

        // The login this machine started the host's Hide through is kept
        // with it, and a record written before there was one reads as none.
        let started = Host {
            state: KnownState::Active,
            started: Some(Login::Ssh("studio".into())),
            ssh: None,
            ..host.clone()
        };
        known.put(started.clone()).unwrap();
        let reopened = Known::open(path.clone()).unwrap();
        assert_eq!(reopened.of(&key(7)), Some(&started));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains(r#""started":{"ssh":"studio"}"#) || text.contains(r#""ssh": "studio""#)
        );
        known.put(host.clone()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("started"), "{text}");
        assert_eq!(Known::open(path).unwrap().of(&key(7)), Some(&host));
        assert!(known.remove(&key(7)).unwrap());
        assert!(!known.remove(&key(7)).unwrap());
    }

    /// A machine's own name moves its grant's and its host record's, and
    /// nothing else; a name chosen here, or the one a switched host's
    /// device had here, stays shown.
    #[test]
    fn a_name_a_machine_says_moves_its_own_name_and_never_the_one_chosen_here() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Grants::open(dir.path().join("grants.json")).unwrap();
        let pending = store
            .grant_pending(key(1), "dev", "example-MacBook-Pro.local", Via::Ssh, 0)
            .unwrap();
        store.confirm(&key(1), &pending.id, 1).unwrap();
        let before = store.state_of(&key(1), 1);
        assert!(store.take_name(&key(1), "작업용 MacBook Pro").unwrap());
        assert!(!store.take_name(&key(1), "작업용 MacBook Pro").unwrap());
        assert!(!store.take_name(&key(2), "nobody").unwrap());
        assert_eq!(store.state_of(&key(1), 1), before);
        let reopened = Grants::open(dir.path().join("grants.json")).unwrap();
        assert_eq!(
            reopened.current(&key(1)).unwrap().shown_name(),
            "작업용 MacBook Pro"
        );

        let path = dir.path().join("known.json");
        let mut known = Known::open(path.clone()).unwrap();
        let host = |key: PeerKey, chosen_name: Option<&str>| Host {
            key,
            device: "host-device".into(),
            name: "mini".into(),
            chosen_name: chosen_name.map(str::to_owned),
            grant: "g1".into(),
            state: KnownState::Active,
            candidates: Vec::new(),
            started: chosen_name.map(|_| Login::Ssh("mini".into())),
            ssh: None,
        };
        known.put(host(key(7), None)).unwrap();
        known.put(host(key(8), Some("작업실 Mac mini"))).unwrap();
        assert!(known.take_name(&key(7), "studio의 Mac mini").unwrap());
        assert!(known.take_name(&key(8), "studio의 Mac mini").unwrap());
        assert!(!known.take_name(&key(9), "nobody").unwrap());
        let reopened = Known::open(path).unwrap();
        let shown = |key: PeerKey| {
            let host = reopened.of(&key).unwrap();
            (host.name.clone(), host.shown_name().to_owned())
        };
        assert_eq!(
            shown(key(7)),
            ("studio의 Mac mini".into(), "studio의 Mac mini".into())
        );
        assert_eq!(
            shown(key(8)),
            ("studio의 Mac mini".into(), "작업실 Mac mini".into())
        );
    }

    /// An older build kept a switched host's name by never replacing it:
    /// read here, that name becomes the one chosen for it, once, so the
    /// host's own name can follow it from then on.
    #[test]
    fn a_host_an_older_build_switched_keeps_its_name_as_the_one_chosen_for_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known.json");
        let mut known = Known::open(path.clone()).unwrap();
        let host = |key: PeerKey, started: Option<Login>| Host {
            key,
            device: "host-device".into(),
            name: "WSL Ubuntu".into(),
            chosen_name: None,
            grant: "g1".into(),
            state: KnownState::Active,
            candidates: Vec::new(),
            started,
            ssh: None,
        };
        known
            .put(host(key(7), Some(Login::Wsl("Ubuntu".into()))))
            .unwrap();
        known.put(host(key(8), None)).unwrap();
        // As the older build wrote it.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(r#""version": 2"#), "{text}");
        std::fs::write(&path, text.replace(r#""version": 2"#, r#""version": 1"#)).unwrap();

        let mut known = Known::open(path.clone()).unwrap();
        assert!(known.take_name(&key(7), "DESKTOP-7Q2").unwrap());
        assert!(known.take_name(&key(8), "DESKTOP-7Q2").unwrap());
        let reopened = Known::open(path.clone()).unwrap();
        let switched = reopened.of(&key(7)).unwrap();
        assert_eq!(
            (switched.name.as_str(), switched.shown_name()),
            ("DESKTOP-7Q2", "WSL Ubuntu")
        );
        // A host it paired without a switch follows its own name.
        assert_eq!(reopened.of(&key(8)).unwrap().shown_name(), "DESKTOP-7Q2");
        // Once: the name taken back after the move is not chosen again.
        let mut known = reopened;
        assert!(known.rename(&key(7), None).unwrap());
        let again = Known::open(path).unwrap();
        assert_eq!(again.of(&key(7)).unwrap().shown_name(), "DESKTOP-7Q2");
    }

    #[test]
    fn a_chosen_name_is_kept_apart_from_the_machines_own_and_survives_a_pairing_again() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = grants(&dir);
        let pending = store
            .grant_pending(key(1), "dev", "MacBook", Via::Link, 0)
            .unwrap();
        store.confirm(&key(1), &pending.id, 1).unwrap();
        let before = store.state_of(&key(1), 1);
        assert!(store.rename(&key(1), Some("작업실 Mac")).unwrap());
        assert!(!store.rename(&key(2), Some("nobody")).unwrap());
        assert_eq!(store.state_of(&key(1), 1), before);
        let renamed = grants(&dir).current(&key(1)).unwrap().clone();
        assert_eq!(
            (renamed.name.as_str(), renamed.shown_name()),
            ("MacBook", "작업실 Mac")
        );
        // Paired again under the name it gives itself: the operator's stays.
        let again = store
            .grant_pending(key(1), "dev", "MacBook Pro", Via::Link, 2)
            .unwrap();
        store.confirm(&key(1), &again.id, 3).unwrap();
        assert_eq!(store.current(&key(1)).unwrap().shown_name(), "작업실 Mac");
        // Taken back, it is shown under its own name again.
        assert!(store.rename(&key(1), None).unwrap());
        assert_eq!(store.current(&key(1)).unwrap().shown_name(), "MacBook Pro");

        let path = dir.path().join("peers").join("known.json");
        let mut known = Known::open(path.clone()).unwrap();
        let host = Host {
            key: key(7),
            device: "host-device".into(),
            name: "Windows PC".into(),
            chosen_name: None,
            grant: "g1".into(),
            state: KnownState::Active,
            candidates: Vec::new(),
            started: None,
            ssh: None,
        };
        known.put(host.clone()).unwrap();
        assert!(known.rename(&key(7), Some("회사 PC")).unwrap());
        assert!(!known.rename(&key(8), Some("nobody")).unwrap());
        known
            .put(Host {
                name: "DESKTOP-7Q2".into(),
                ..host
            })
            .unwrap();
        let reopened = Known::open(path).unwrap();
        let kept = reopened.of(&key(7)).unwrap();
        assert_eq!(
            (kept.name.as_str(), kept.shown_name(), kept.state),
            ("DESKTOP-7Q2", "회사 PC", KnownState::Active)
        );
    }

    fn kept(started: Option<Login>, ssh: Option<SshRoute>) -> Host {
        Host {
            key: key(9),
            device: "studio".into(),
            name: "Studio".into(),
            chosen_name: None,
            grant: "g9".into(),
            state: KnownState::Active,
            candidates: vec![Candidate::Relay("https://relay.example./".into())],
            started,
            ssh,
        }
    }

    /// The alias a host is dialed by is kept in its record, a record written
    /// before it reads without one, and the address the keeper dials names
    /// it last, built in memory and never written.
    #[test]
    fn an_alias_field_is_kept_and_a_record_without_it_reads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known.json");
        let mut known = Known::open(path.clone()).unwrap();
        known.put(kept(None, None)).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(!written.contains("\"ssh\""), "{written}");
        assert!(known.record_ssh(&key(9), "studio").unwrap());
        assert!(!known.record_ssh(&key(9), "studio").unwrap(), "already so");
        assert!(known.record_ssh(&key(9), "-oProxyCommand=x").is_err());
        let read = Known::open(path.clone()).unwrap();
        let host = read.of(&key(9)).unwrap();
        assert_eq!(host.ssh_alias(), Some("studio"));
        let to = host.dial_addr();
        assert_eq!(to.candidates.last(), Some(&Candidate::Ssh("studio".into())));
        // A record an older build wrote, with no route, reads.
        let mut older: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        older["hosts"][0].as_object_mut().unwrap().remove("ssh");
        std::fs::write(&path, older.to_string()).unwrap();
        let read = Known::open(path).unwrap();
        assert_eq!(read.of(&key(9)).unwrap().ssh, None);
        // A distribution is reached through `wsl.exe` alone.
        let mut distribution = kept(Some(Login::Ssh("studio".into())), None);
        distribution.candidates = vec![Candidate::Wsl("Ubuntu".into())];
        assert!(
            !distribution
                .dial_addr()
                .candidates
                .iter()
                .any(|candidate| matches!(candidate, Candidate::Ssh(_)))
        );
    }

    /// A route turned off gives no alias to dial and keeps the login a Hide
    /// this machine started there is brought to its build through; it is
    /// turned off only while it is still the alias the attempt dialed (R3).
    #[test]
    fn an_alias_refused_by_its_key_stays_off_and_leaves_the_started_login() {
        let dir = tempfile::tempdir().unwrap();
        let mut known = Known::open(dir.path().join("known.json")).unwrap();
        known
            .put(kept(Some(Login::Ssh("studio".into())), None))
            .unwrap();
        assert_eq!(known.of(&key(9)).unwrap().ssh_alias(), Some("studio"));
        assert!(
            !known.refuse_ssh(&key(9), "another").unwrap(),
            "a newer alias stays"
        );
        assert!(known.refuse_ssh(&key(9), "studio").unwrap());
        let host = known.of(&key(9)).unwrap();
        assert_eq!(host.ssh, Some(SshRoute::Off));
        assert_eq!(host.ssh_alias(), None);
        assert_eq!(host.started, Some(Login::Ssh("studio".into())));
        assert!(
            !host
                .dial_addr()
                .candidates
                .iter()
                .any(|candidate| matches!(candidate, Candidate::Ssh(_)))
        );
        // Recorded again by a later add, it is dialed again.
        assert!(known.record_ssh(&key(9), "studio").unwrap());
        assert_eq!(known.of(&key(9)).unwrap().ssh_alias(), Some("studio"));
    }
}
