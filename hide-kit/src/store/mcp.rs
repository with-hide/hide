//! The MCP's [`Applier`] and changes (PRD skill-mcp-manager B8-B14, B18,
//! B28, D-10, D-11, D-27, D-28, D-32, D-39).
//!
//! An MCP's column is its agent, since each agent keeps its own settings
//! file; its place is that file in the agent's default account and in each
//! profile account (`[account.paths] mcp`), and one entry is the server's
//! entry in it, read, written and taken out in the agent's format by
//! `hide_agent_hooks::mcp` with every other byte of the file kept (B11).
//!
//! The item's body is the server without its key values: its name, how it
//! is reached and the names of its variables or headers. The values live in
//! the item's own store folder (`crate::mcp_keys`), which a delete moves to
//! the undo list with the item, a restore brings back and a permanent delete
//! removes (D-11, D-39). A value reaches the store only in a change the
//! operator made on the screen ([`StoreChange::AddMcp`],
//! [`StoreChange::SetMcp`]), a paste ([`StoreChange::ReadMcp`]) or a copy
//! from another machine ([`StoreChange::Receive`]), each as [`Sealed`], which
//! prints as `••••` and which [`StoreChange::redacted`] empties; it leaves
//! the store only into an agent's file or a copy to another machine (B13).
//!
//! The ownership digest ([`McpBody::digest`]) holds no key value, so a key
//! changed in Hide is not seen as a change of the agents' copies: the change
//! marks every place Hide wrote the item to as stale ([`StoreRecord::stale`])
//! and the reconcile writes them again, a failed one staying stale until a
//! pass gets through (B12, B26).
//!
//! Whether an agent's entry is the item, or two agents' entries are one
//! server, or an entry is still the one Hide wrote, takes the keys too: the
//! same server with another key is the operator's other server (B21), and a
//! key typed into a file by hand makes the entry the operator's (B22). The
//! values travel only inside [`KeyValues`], which compares and fingerprints
//! them and gives none out; the record keeps a fingerprint keyed with the
//! store's secret (`crate::mcp_keys::KeysSecret`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use hide_agent_adapter::AccountPath;
use hide_agent_hooks::account::AccountAt;
use hide_agent_hooks::mcp::{self, Keys, McpFile, Pin, Reach, Secret, Server, Transport};
use serde::{Deserialize, Serialize};

use super::{
    Applier, Carried, Column, Ctx, Entry, ItemBody, ItemKind, Listed, Place, Source, StoreItem,
    StoreRecord, StoreRefusal, StoreRefusalCause, Version, failed, name_free, not_found,
    now_unix_ms,
};
use crate::agents::{ADAPTERS, AgentAdapter};

/// The longest paste Hide reads.
pub const PASTE_LIMIT: usize = 256 * 1024;
/// The most servers one paste gives.
pub const PASTED_LIMIT: usize = 32;
/// How long what a paste read stays for the add screen; its staged keys go
/// with it.
pub const PREVIEW_MINUTES: u64 = 60;

/// An MCP server as the store keeps it: its name, how it is reached and the
/// names of its keys, never a value.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct McpBody {
    pub server: Server,
}

impl McpBody {
    /// What every agent's entry is compared with (`Server::digest`).
    pub(crate) fn digest(&self) -> String {
        self.server.digest()
    }

    /// The short version beside the name (B36): '원격' for a remote server,
    /// the version a local one's package or image pins, 'npm 최신' for an
    /// npm runner's package that pins none, and none for anything else.
    pub(crate) fn version(&self) -> Version {
        match &self.server.reach {
            Reach::Remote { .. } => Version::Remote,
            Reach::Local { command, args, .. } => match mcp::pin(command, args) {
                Pin::Version(version) => Version::Pinned(version),
                Pin::NpmLatest => Version::NpmLatest,
                Pin::Unpinned => Version::Unpinned,
            },
        }
    }
}

/// A value the operator typed or pasted, on its way from the screen or
/// another machine to a store: a key's value, or a paste that may hold one.
/// It prints as `••••`, and a change kept after its pass holds it emptied
/// ([`StoreChange::redacted`]); an empty one is a value not given, so a
/// redacted change applied again changes no key.
#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Sealed(String);

impl Sealed {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub(crate) fn emptied(&self) -> Self {
        Self(String::new())
    }

    fn given(&self) -> Option<&str> {
        (!self.0.is_empty()).then_some(self.0.as_str())
    }

    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }
}

impl fmt::Debug for Sealed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("••••")
    }
}

/// Key values by the variable or header they belong to, as a change
/// carries them.
pub type SentKeys = BTreeMap<String, Sealed>;

/// `keys` with every value emptied.
pub(crate) fn emptied(keys: &SentKeys) -> SentKeys {
    keys.iter()
        .map(|(name, value)| (name.clone(), value.emptied()))
        .collect()
}

fn sent(keys: &Keys) -> SentKeys {
    keys.iter()
        .map(|(name, value)| (name.clone(), Sealed::new(value.expose())))
        .collect()
}

/// One server of a paste the add screen offers (B8, B9).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PastedServer {
    pub server: Server,
    /// The variables and headers the paste gave a value; the values stay in
    /// the store.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub given: Vec<String>,
    /// An item of that name is already in the store.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub taken: bool,
    /// The agents it would reach now (B9).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<String>,
}

/// What a paste read as (B8): no server for a paste Hide cannot read, whose
/// fields the operator fills.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct McpPreview {
    pub id: String,
    pub servers: Vec<PastedServer>,
    pub at_unix_ms: u64,
}

/// The server of a paste whose values an add takes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PastedPick {
    pub preview: String,
    /// Its place in [`McpPreview::servers`].
    pub index: usize,
}

pub(crate) struct McpApplier;

/// The key a place of `agent`'s at the real path `path` is recorded under:
/// the agent names the file's shape, so a place the record rebuilds after
/// its account went (`store::recorded_place`, which names no column) still
/// reads it.
fn key(agent: &str, path: &Path) -> String {
    format!("mcp:{agent}:{}", path.display())
}

/// `place`'s file in its agent's shape.
fn file_of(place: &Place) -> Result<McpFile, String> {
    let agent = match place.columns.first() {
        Some(column) => column.as_str(),
        None => place
            .key
            .strip_prefix("mcp:")
            .and_then(|rest| rest.split_once(':'))
            .map_or("", |(agent, _)| agent),
    };
    crate::agents::adapter(agent)
        .and_then(|adapter| McpFile::at(adapter.declaration(), place.path.clone()))
        .ok_or_else(|| {
            format!(
                "Hide no longer knows which agent's MCP file {} is",
                place.path.display()
            )
        })
}

/// Lists `adapter`'s file at the real path `path`, or adds `profiles` to the
/// place already listed for it (a profile's file that is a link to the
/// default one); that file is blocked only while every spelling of it is.
fn join(
    places: &mut Vec<Place>,
    ctx: &Ctx<'_>,
    adapter: &AgentAdapter,
    path: PathBuf,
    profiles: Vec<String>,
    blocked: Option<String>,
) {
    let key = key(adapter.id, &path);
    if let Some(listed) = places.iter_mut().find(|listed| listed.key == key) {
        listed.profiles.extend(profiles);
        if blocked.is_none() {
            listed.blocked = None;
        }
        return;
    }
    places.push(Place {
        key,
        path,
        columns: vec![adapter.id.to_owned()],
        profiles,
        blocked,
        fresh: ctx.fresh,
    });
}

fn body_of(item: &StoreItem) -> Result<&McpBody, String> {
    match &item.body {
        ItemBody::Mcp(body) => Ok(body),
        ItemBody::Skill(_) => Err(format!("{} is not an MCP server", item.id)),
    }
}

/// Why `adapter`'s file takes no `server`, or nothing (B14).
fn refusal(adapter: &AgentAdapter, server: &Server) -> Option<String> {
    let Some(format) = adapter.declaration().mcp_format() else {
        return Some(format!("{} takes no MCP server from Hide", adapter.label));
    };
    let transport = server.reach.transport();
    if !mcp::accepts(format, transport) {
        let kind = match transport {
            Transport::Stdio => "a local",
            Transport::Http => "an HTTP",
            Transport::Sse => "an SSE",
        };
        return Some(format!("{} does not take {kind} server", adapter.label));
    }
    if !mcp::takes_name(format, &server.name) {
        return Some(format!(
            "{} reads a server named {} as something else",
            adapter.label, server.name
        ));
    }
    None
}

/// An entry's digest and keys; one Hide cannot read has a digest no server
/// has, so it is never taken for Hide's.
fn entry_of(found: &mcp::Found) -> Entry {
    match &found.server {
        Some((server, keys)) => Entry {
            digest: server.digest(),
            link: None,
            whole: found.whole,
            keys: Some(KeyValues::of(server, keys)),
        },
        None => Entry {
            digest: "unreadable".to_owned(),
            link: None,
            whole: false,
            keys: None,
        },
    }
}

/// The values of the keys a server names, as one entry or item holds them:
/// they compare, and fingerprint with the store's secret, and give no value
/// out, so they print as names alone.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct KeyValues(Vec<(String, Option<Secret>)>);

impl KeyValues {
    fn of(server: &Server, keys: &Keys) -> Self {
        let mut names = server.reach.key_names().to_vec();
        names.sort();
        names.dedup();
        Self(
            names
                .into_iter()
                .map(|name| {
                    let value = keys.get(&name).cloned();
                    (name, value)
                })
                .collect(),
        )
    }

    /// An HMAC-SHA256 of the values under `secret`, for the record.
    pub(crate) fn fingerprint(&self, secret: &crate::mcp_keys::KeysSecret) -> String {
        use sha2::{Digest as _, Sha256};
        let mut message = Vec::new();
        for (name, value) in &self.0 {
            message.extend((name.len() as u64).to_le_bytes());
            message.extend(name.as_bytes());
            match value {
                Some(value) => {
                    message.push(1);
                    message.extend((value.expose().len() as u64).to_le_bytes());
                    message.extend(value.expose().as_bytes());
                }
                None => message.push(0),
            }
        }
        let mut block = [0u8; 64];
        block[..32].copy_from_slice(secret.bytes());
        let pad = |with: u8| block.iter().map(|byte| byte ^ with).collect::<Vec<u8>>();
        let inner = Sha256::new()
            .chain_update(pad(0x36))
            .chain_update(&message)
            .finalize();
        Sha256::new()
            .chain_update(pad(0x5c))
            .chain_update(inner)
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

impl fmt::Debug for KeyValues {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_list()
            .entries(self.0.iter().map(|(name, _)| name))
            .finish()
    }
}

fn keys_failed(error: crate::mcp_keys::KeyFileError) -> StoreRefusal {
    failed(error.to_string())
}

impl Applier for McpApplier {
    fn kind(&self) -> ItemKind {
        ItemKind::Mcp
    }

    fn columns(&self) -> Vec<Column> {
        ADAPTERS
            .iter()
            .filter(|adapter| adapter.declaration().mcp_format().is_some())
            .map(|adapter| Column {
                id: adapter.id.to_owned(),
                agents: vec![adapter.id.to_owned()],
                also: Vec::new(),
            })
            .collect()
    }

    fn places(&self, ctx: &Ctx<'_>) -> Vec<Place> {
        let home = ctx.home();
        let mut places = Vec::new();
        for adapter in ADAPTERS {
            let Some(file) = McpFile::of(adapter.declaration(), AccountAt::Home(home)) else {
                continue;
            };
            // The server goes in once the agent has set itself up here, as a
            // skill goes in once the agent made its folder.
            let state = AccountAt::Home(home).place(adapter.declaration(), AccountPath::State);
            let blocked = (!state.is_dir()).then(|| {
                "its agent has not set itself up here yet; the server goes in once it has"
                    .to_owned()
            });
            join(
                &mut places,
                ctx,
                adapter,
                super::skill::real_folder(file.path()),
                Vec::new(),
                blocked,
            );
        }
        for account in ctx.accounts.list() {
            let Some(file) = account.mcp_file(home) else {
                continue;
            };
            let (path, blocked) = match account.judge(file.path()) {
                Ok(real) => (real, None),
                Err(reason) => (file.path().to_path_buf(), Some(reason)),
            };
            join(
                &mut places,
                ctx,
                account.adapter(),
                path,
                account.profiles().to_vec(),
                blocked,
            );
        }
        places
    }

    fn supports(&self, item: &StoreItem, column: &Column) -> Result<(), String> {
        let body = body_of(item)?;
        let adapter =
            crate::agents::adapter(&column.id).ok_or_else(|| format!("no agent {}", column.id))?;
        refusal(adapter, &body.server).map_or(Ok(()), Err)
    }

    fn entry(&self, place: &Place, name: &str) -> Result<Option<Entry>, String> {
        let file = file_of(place)?;
        Ok(mcp::entry(&file, name)
            .map_err(|failure| failure.message())?
            .map(|found| entry_of(&found)))
    }

    fn entries(&self, place: &Place) -> Result<Vec<Listed>, String> {
        let file = file_of(place)?;
        Ok(mcp::entries(&file)
            .map_err(|failure| failure.message())?
            .iter()
            .map(|found| {
                let read = match found.server {
                    Some(_) => Ok(entry_of(found)),
                    None => Err(format!(
                        "Hide cannot read it as a server in {}",
                        place.path.display()
                    )),
                };
                (found.name.clone(), read)
            })
            .collect())
    }

    fn keys(&self, item: &StoreItem, files: &Path) -> Result<Option<KeyValues>, String> {
        let body = body_of(item)?;
        let keys = crate::mcp_keys::read(files).map_err(|error| error.to_string())?;
        Ok(Some(KeyValues::of(&body.server, &keys)))
    }

    fn entry_path(&self, place: &Place, _name: &str) -> PathBuf {
        place.path.clone()
    }

    fn write(&self, ctx: &Ctx<'_>, place: &Place, item: &StoreItem) -> Result<String, String> {
        let body = body_of(item)?;
        let file = file_of(place)?;
        let keys =
            crate::mcp_keys::read(&ctx.item_files(&item.id)).map_err(|error| error.to_string())?;
        match mcp::write(&file, &body.server, &keys).map_err(|failure| failure.message())? {
            mcp::Written::Written | mcp::Written::Unchanged => Ok(body.server.digest()),
            mcp::Written::Unsupported => Err(format!(
                "{} does not take the server {}",
                file.agent(),
                body.server.name
            )),
        }
    }

    fn remove(&self, place: &Place, name: &str) -> Result<(), String> {
        let file = file_of(place)?;
        mcp::remove(&file, name)
            .map(drop)
            .map_err(|failure| failure.message())
    }

    fn adopt(
        &self,
        _ctx: &Ctx<'_>,
        place: &Place,
        name: &str,
        into: &Path,
    ) -> Result<(Source, ItemBody), StoreRefusal> {
        let file = file_of(place).map_err(failed)?;
        let found = mcp::entry(&file, name)
            .map_err(|failure| failed(failure.message()))?
            .ok_or_else(|| not_found(format!("{} holds no server {name}", place.path.display())))?;
        let (server, keys) = found.server.ok_or_else(|| {
            StoreRefusal::new(
                StoreRefusalCause::Unsupported,
                format!(
                    "Hide cannot read the server {name} in {}",
                    place.path.display()
                ),
            )
        })?;
        server.check().map_err(|reason| {
            StoreRefusal::new(
                StoreRefusalCause::Unsupported,
                format!("the server {name} in {}: {reason}", place.path.display()),
            )
        })?;
        // The folder is the item's even when the server has no key, since
        // the store moves it.
        hide_platform::fs::private::create_dir_all(into)
            .map_err(|error| failed(format!("{} could not be made: {error}", into.display())))?;
        crate::mcp_keys::write(into, &keys).map_err(keys_failed)?;
        // The entry's address, as the store names what it took over and set
        // aside: the file, then the server's name in it.
        Ok((
            Source::Copy {
                from: place.path.join(name).display().to_string(),
            },
            ItemBody::Mcp(McpBody { server }),
        ))
    }

    /// Takes the agent's entry out of its file whole, every field it holds
    /// kept with its keys in the undo list's folder, so putting it back
    /// gives the file what it had (B21, B30).
    fn set_aside(&self, place: &Place, name: &str, into: &Path) -> Result<ItemBody, StoreRefusal> {
        let file = file_of(place).map_err(failed)?;
        let mut server = None;
        let taken = mcp::set_aside(&file, name, |entry, readout| {
            let (read, keys) =
                readout.ok_or_else(|| "Hide cannot read it as a server".to_owned())?;
            crate::mcp_keys::keep_set_aside(into, keys, entry)
                .map_err(|error| error.to_string())?;
            server = Some(read.clone());
            Ok(())
        })
        .map_err(|failure| failed(failure.message()))?;
        match server {
            Some(server) if taken => Ok(ItemBody::Mcp(McpBody { server })),
            _ => Err(not_found(format!(
                "{} holds no server {name}",
                place.path.display()
            ))),
        }
    }

    fn put_back(&self, place: &Place, name: &str, from: &Path) -> Result<bool, String> {
        let file = file_of(place)?;
        let entry = crate::mcp_keys::set_aside_of(from)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("{} holds no server Hide set aside", from.display()))?;
        if !mcp::put_back(&file, name, &entry).map_err(|failure| failure.message())? {
            return Ok(false);
        }
        // Back in the agent's file; the copy and its keys go.
        std::fs::remove_dir_all(from)
            .map_err(|error| format!("{} could not be removed: {error}", from.display()))?;
        Ok(true)
    }

    /// Writes the server taken over back into the agent's file it was
    /// taken from, with its keys, where nothing of its name stands (B30).
    fn leave_copy(&self, place: &Place, item: &StoreItem, files: &Path) -> Result<(), String> {
        let body = body_of(item)?;
        let file = file_of(place)?;
        let keys = crate::mcp_keys::read(files).map_err(|error| error.to_string())?;
        match mcp::write(&file, &body.server, &keys).map_err(|failure| failure.message())? {
            mcp::Written::Written | mcp::Written::Unchanged => Ok(()),
            mcp::Written::Unsupported => Err(format!(
                "{} does not take the server {}",
                file.agent(),
                body.server.name
            )),
        }
    }

    fn purge(&self, _ctx: &Ctx<'_>, _item: &StoreItem) -> Result<(), String> {
        // The keys are in the item's folder, which the store removes with it.
        Ok(())
    }

    /// Carries the server's spec and its key values (D-11, B28); the keys
    /// travel only inside the device link.
    fn export(&self, ctx: &Ctx<'_>, item: &StoreItem) -> Result<Carried, StoreRefusal> {
        let body = body_of(item).map_err(failed)?;
        let keys = crate::mcp_keys::read(&ctx.item_files(&item.id)).map_err(keys_failed)?;
        Ok(Carried::Mcp {
            body: body.clone(),
            keys: sent(&keys),
        })
    }

    /// Keeps the carried spec as the body and its key values in the item's
    /// folder `into`.
    fn import(
        &self,
        _ctx: &Ctx<'_>,
        name: &str,
        content: &Carried,
        into: &Path,
    ) -> Result<ItemBody, StoreRefusal> {
        let Carried::Mcp { body, keys } = content else {
            return Err(StoreRefusal::new(
                StoreRefusalCause::Unsupported,
                format!("{name} was carried as a skill"),
            ));
        };
        if body.server.name != name {
            return Err(failed(format!(
                "{name} was carried with the server {}",
                body.server.name
            )));
        }
        let keys = checked(&body.server, keys, &Keys::new())?;
        hide_platform::fs::private::create_dir_all(into)
            .map_err(|error| failed(format!("{} could not be made: {error}", into.display())))?;
        crate::mcp_keys::write(into, &keys).map_err(keys_failed)?;
        Ok(ItemBody::Mcp(body.clone()))
    }
}

// --- Changes ----------------------------------------------------------------------

/// `server` checked, with the keys it is kept with: each name it gives,
/// typed on the screen, else from `before` (a staged paste, or the item's
/// current keys); only the names the server gives are kept, and every one
/// needs a value an agent's file can carry.
fn checked(server: &Server, typed: &SentKeys, before: &Keys) -> Result<Keys, StoreRefusal> {
    server.check().map_err(failed)?;
    let mut keys = Keys::new();
    for name in server.reach.key_names() {
        let value = typed
            .get(name)
            .and_then(Sealed::given)
            .map(|value| Secret::new(value.to_owned()))
            .or_else(|| before.get(name).cloned());
        if let Some(value) = value {
            keys.insert(name.clone(), value);
        }
    }
    server.check_keys(&keys).map_err(failed)?;
    Ok(keys)
}

fn staged(ctx: &Ctx<'_>, preview: &str, index: usize) -> PathBuf {
    ctx.staging().join(preview).join(index.to_string())
}

/// Reads a paste (B8): each server it gives lands in
/// [`StoreRecord::mcp_previews`] with the names of its keys, and the values
/// it gave stay staged in the store until an add takes them, the operator
/// closes the answer ([`cancel`]) or, unanswered, it passes its hour or
/// four newer pastes.
pub(super) fn read_paste(
    ctx: &Ctx<'_>,
    store: &mut StoreRecord,
    text: &Sealed,
) -> Result<String, StoreRefusal> {
    if text.len() > PASTE_LIMIT {
        return Err(StoreRefusal::new(
            StoreRefusalCause::TooLarge,
            format!("a paste is at most {} KiB", PASTE_LIMIT / 1024),
        ));
    }
    let id = super::source::fresh_id("mcp-");
    let mut servers = Vec::new();
    for pasted in mcp::paste::read(&text.0, &|program| ctx.detection.finds(program)) {
        let Some(server) = pasted.server else {
            continue;
        };

        if servers.len() == PASTED_LIMIT {
            break;
        }
        let given: Vec<String> = pasted
            .keys
            .iter()
            .filter(|(name, value)| {
                !value.expose().is_empty() && server.reach.key_names().contains(name)
            })
            .map(|(name, _)| name.clone())
            .collect();
        let keys: Keys = pasted
            .keys
            .into_iter()
            .filter(|(name, _)| given.contains(name))
            .collect();
        crate::mcp_keys::write(&staged(ctx, &id, servers.len()), &keys).map_err(keys_failed)?;
        servers.push(PastedServer {
            server,
            given,
            taken: false,
            agents: Vec::new(),
        });
    }
    store.mcp_previews.push(McpPreview {
        id: id.clone(),
        servers,
        at_unix_ms: now_unix_ms(),
    });
    Ok(id)
}

/// Adds the server the operator confirmed (B8-B10, B33): its keys typed on
/// the screen, else the values the paste `from` gave.
pub(super) fn add(
    ctx: &Ctx<'_>,
    store: &mut StoreRecord,
    server: &Server,
    typed: &SentKeys,
    from: Option<&PastedPick>,
) -> Result<(), StoreRefusal> {
    name_free(store, ItemKind::Mcp, &server.name)?;
    let before = match from {
        Some(pick) => {
            let preview = store
                .mcp_previews
                .iter()
                .find(|preview| preview.id == pick.preview)
                .ok_or_else(|| not_found("the paste's answer is gone; paste it again"))?;
            if pick.index >= preview.servers.len() {
                return Err(not_found("that server is not in the paste"));
            }
            crate::mcp_keys::read(&staged(ctx, &pick.preview, pick.index)).map_err(keys_failed)?
        }
        None => Keys::new(),
    };
    let keys = checked(server, typed, &before)?;
    let id = StoreItem::id_of(ItemKind::Mcp, &server.name);
    crate::mcp_keys::write(&ctx.item_files(&id), &keys).map_err(keys_failed)?;
    store.items.push(StoreItem {
        id: id.clone(),
        kind: ItemKind::Mcp,
        name: server.name.clone(),
        source: Source::Pasted,
        on: true,
        off_for: BTreeSet::new(),
        added_at_unix_ms: now_unix_ms(),
        latest: None,
        folder_missing: false,
        fingerprint: None,
        adopted: None,
        body: ItemBody::Mcp(McpBody {
            server: server.clone(),
        }),
    });
    // A key lives in one place once the add ends: the staged copy goes, and
    // the paste's answer with it once every server in it is added. A copy
    // that cannot go now goes with the answer's hour.
    if let Some(pick) = from {
        if let Err(reason) = remove_staged(&staged(ctx, &pick.preview, pick.index)) {
            super::log("store.staged_left", Some(&id), &reason);
        }
        let answered = store
            .mcp_previews
            .iter()
            .find(|preview| preview.id == pick.preview)
            .is_some_and(|preview| {
                preview.servers.iter().all(|pasted| {
                    store
                        .item(&StoreItem::id_of(ItemKind::Mcp, &pasted.server.name))
                        .is_some()
                })
            });
        // The answer goes either way; a folder left behind goes with the
        // staging folders no record names.
        if answered {
            if let Err(reason) = remove_staged(&ctx.staging().join(&pick.preview)) {
                super::log("store.staged_left", Some(&id), &reason);
            }
            store.mcp_previews.retain(|kept| kept.id != pick.preview);
        }
    }
    Ok(())
}

/// Drops the paste's answer `preview` and the values it staged.
pub(super) fn cancel(
    ctx: &Ctx<'_>,
    store: &mut StoreRecord,
    preview: &str,
) -> Result<(), StoreRefusal> {
    if !valid_preview(preview) {
        return Err(not_found(format!("no paste {preview}")));
    }
    remove_staged(&ctx.staging().join(preview)).map_err(failed)?;
    store.mcp_previews.retain(|kept| kept.id != preview);
    Ok(())
}

/// A preview id as [`read_paste`] makes them, so a cancel names nothing
/// outside the staging folder.
fn valid_preview(preview: &str) -> bool {
    preview.starts_with("mcp-")
        && preview
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn remove_staged(path: &Path) -> Result<(), String> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("{} could not be removed: {error}", path.display())),
    }
}

/// Changes an MCP item's server and keys (B12): a key not typed keeps its
/// value. A changed server reaches the agents as any changed item does; a
/// changed key marks every place Hide wrote the item to, which the
/// reconcile writes again.
pub(super) fn set(
    ctx: &Ctx<'_>,
    store: &mut StoreRecord,
    id: &str,
    server: &Server,
    typed: &SentKeys,
) -> Result<(), StoreRefusal> {
    let item = store
        .item(id)
        .ok_or_else(|| not_found(format!("no item {id}")))?;
    if item.kind != ItemKind::Mcp {
        return Err(StoreRefusal::new(
            StoreRefusalCause::Unsupported,
            format!("{id} is not an MCP server"),
        ));
    }
    if server.name != item.name {
        return Err(StoreRefusal::new(
            StoreRefusalCause::Unsupported,
            "a server keeps its name; add it again under the new one",
        ));
    }
    let folder = ctx.item_files(id);
    let before = crate::mcp_keys::read(&folder).map_err(keys_failed)?;
    let keys = checked(server, typed, &before)?;
    crate::mcp_keys::write(&folder, &keys).map_err(keys_failed)?;
    if keys != before {
        mark_stale(store, id);
    }
    let item = store.item_mut(id).expect("found above");
    item.body = ItemBody::Mcp(McpBody {
        server: server.clone(),
    });
    Ok(())
}

/// The keys a copy from another machine carried for the item `id`, which
/// is already here with the same server and switches (B28): taken over the
/// item's own when they differ, and the item's places marked to be written
/// again. Answers whether they differed.
pub(super) fn take_carried_keys(
    ctx: &Ctx<'_>,
    store: &mut StoreRecord,
    id: &str,
    staged: &Path,
) -> Result<bool, StoreRefusal> {
    let folder = ctx.item_files(id);
    let carried = crate::mcp_keys::read(staged).map_err(keys_failed)?;
    if crate::mcp_keys::read(&folder).map_err(keys_failed)? == carried {
        return Ok(false);
    }
    crate::mcp_keys::write(&folder, &carried).map_err(keys_failed)?;
    mark_stale(store, id);
    Ok(true)
}

/// Marks every place Hide wrote the item `id` to, to be written again.
fn mark_stale(store: &mut StoreRecord, id: &str) {
    let places: BTreeSet<String> = store
        .written
        .iter()
        .filter(|(_, place)| place.entries.values().any(|written| written.item == id))
        .map(|(key, _)| key.clone())
        .collect();
    if !places.is_empty() {
        store.stale.entry(id.to_owned()).or_default().extend(places);
    }
}

/// The previews as the report shows them: whether each server's name is
/// taken, and the agents it would reach now (B9).
pub(super) fn previews(ctx: &Ctx<'_>, store: &StoreRecord) -> Vec<McpPreview> {
    store
        .mcp_previews
        .iter()
        .map(|preview| McpPreview {
            servers: preview
                .servers
                .iter()
                .map(|pasted| PastedServer {
                    taken: name_free(store, ItemKind::Mcp, &pasted.server.name).is_err(),
                    agents: ADAPTERS
                        .iter()
                        .filter(|adapter| {
                            refusal(adapter, &pasted.server).is_none()
                                && ctx.enabled(adapter.id)
                                && ctx.installed(adapter.id)
                        })
                        .map(|adapter| adapter.id.to_owned())
                        .collect(),
                    ..pasted.clone()
                })
                .collect(),
            ..preview.clone()
        })
        .collect()
}

/// Drops the previews past their age or count, oldest first; their staged
/// keys go with the staging folder no record names (`store::trim_staging`).
pub(super) fn trim_previews(store: &mut StoreRecord) {
    let now = now_unix_ms();
    store
        .mcp_previews
        .retain(|preview| now.saturating_sub(preview.at_unix_ms) < PREVIEW_MINUTES * 60 * 1000);
    while store.mcp_previews.len() > super::PREVIEW_LIMIT {
        store.mcp_previews.remove(0);
    }
}
