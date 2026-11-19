//! The MCP servers Hide writes into each agent's own MCP file (PRD
//! skill-mcp-manager B11, B14, B22, B27; docs/agent-hooks.md, MCP servers).
//!
//! An agent's definition names the file (`[account.paths] mcp`, profile
//! accounts included) and its shape (`mcp_format`,
//! [`hide_agent_adapter::McpFormat`]); this module holds one converter per
//! shape, never one per agent, and each shape says which transports it
//! takes ([`accepts`]). A converter writes and takes out Hide's own entries
//! only: an entry is Hide's when Hide's record of that file names it
//! ([`record`]), and an entry of the same name that the record does not name
//! is the operator's, which Hide reports and never overwrites. Every other
//! byte of the file is kept: a JSON file goes through the same lossless read
//! and compare-before-replace as the hook files, a TOML file through
//! `toml_edit`, which keeps the rest of the document as it was written.
//!
//! A server as Hide keeps it ([`Server`]) holds the names of its variables
//! and headers and never their values: those are the operator's keys, which
//! live in Hide's owner-only key file and reach a converter as [`Keys`] only
//! at the moment it writes them into the agent's file, so a server can be
//! logged, reported and sent to the screen without carrying one.

use std::collections::BTreeMap;
use std::fmt;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hide_agent_adapter::{AccountPath, AgentAdapter, McpFormat};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use toml_edit::{Item, TableLike};

use crate::account::AccountAt;
use crate::install::{HookDocument, InstallFailure, read_document, write_document};

mod format;
pub mod paste;
mod toml_file;

use format::{KeyRefusal, Shape, Syntax, shape};
use toml_file::TomlFile;

/// The most bytes a server's name holds.
pub const MAX_NAME_BYTES: usize = 64;

/// One MCP server as Hide keeps it: its name in the agents' files and how it
/// is reached.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Server {
    pub name: String,
    pub reach: Reach,
}

/// How an agent reaches a server: by starting a program on this machine, or
/// at an address.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Reach {
    Local {
        command: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
        /// The variables the program is started with, by name; their values
        /// are [`Keys`].
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        env: Vec<String>,
    },
    Remote {
        transport: Remote,
        url: String,
        /// The headers every request carries, by name; their values are
        /// [`Keys`].
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        headers: Vec<String>,
    },
}

/// The protocol a remote server speaks.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Remote {
    /// MCP's Streamable HTTP.
    Http,
    /// The older HTTP with Server-Sent Events.
    Sse,
}

/// The transport a server uses, which an agent's shape takes or not.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Stdio,
    Http,
    Sse,
}

impl Reach {
    pub fn transport(&self) -> Transport {
        match self {
            Self::Local { .. } => Transport::Stdio,
            Self::Remote {
                transport: Remote::Http,
                ..
            } => Transport::Http,
            Self::Remote {
                transport: Remote::Sse,
                ..
            } => Transport::Sse,
        }
    }

    /// The names whose values are the operator's keys: a local server's
    /// variables, a remote one's headers.
    pub fn key_names(&self) -> &[String] {
        match self {
            Self::Local { env, .. } => env,
            Self::Remote { headers, .. } => headers,
        }
    }
}

/// A value the operator gave a variable or a header: an API key, or a
/// value Hide treats as one. It prints as `••••` and has no serialization,
/// so a log line, a report or an event that formats it carries no key;
/// [`Secret::expose`] is the one way to its text, for the key file and the
/// agent's own file.
#[derive(Clone, Eq, PartialEq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// The value itself, for the writes that must carry it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("••••")
    }
}

/// A server's keys by the variable or header they belong to.
pub type Keys = BTreeMap<String, Secret>;

/// A server name every agent's file can hold as it is: a JSON member name
/// and a TOML bare key, Codex's `^[a-zA-Z0-9_-]+$`, at most
/// [`MAX_NAME_BYTES`].
pub fn valid_name(name: &str) -> bool {
    (1..=MAX_NAME_BYTES).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// `text` as a server name: lower case, every character a name may not hold
/// turned into `-`, runs of them into one, cut to [`MAX_NAME_BYTES`]; empty
/// when nothing of it remains.
pub fn normal_name(text: &str) -> String {
    let mut name = String::new();
    for character in text.trim().chars() {
        let character = character.to_ascii_lowercase();
        if character.is_ascii_alphanumeric() || character == '_' {
            name.push(character);
        } else if !name.ends_with('-') && !name.is_empty() {
            name.push('-');
        }
    }
    let mut name = name.trim_end_matches('-').to_owned();
    name.truncate(MAX_NAME_BYTES);
    name.trim_end_matches('-').to_owned()
}

/// Whether an agent whose file has the shape `format` takes a server of
/// `transport` (B14); a cell of one that does not reads unsupported.
pub fn accepts(format: McpFormat, transport: Transport) -> bool {
    shape(format).transports.contains(&transport)
}

/// The most entries Hide's record keeps: every server in every account
/// file of one home, far more than an operator keeps.
pub const RECORD_LIMIT: usize = 4096;
/// How long a change waits for another change of Hide's MCP record, which
/// holds it across one file's read and write.
const RECORD_WAIT: Duration = Duration::from_secs(5);

/// One agent's MCP file in one account.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpFile {
    agent: String,
    format: McpFormat,
    path: PathBuf,
    /// The account's home, under which Hide keeps its record.
    home: PathBuf,
}

impl McpFile {
    /// `adapter`'s MCP file in the account `at`, `None` for an agent Hide
    /// writes no MCP server for.
    pub fn of(adapter: &AgentAdapter, at: AccountAt<'_>) -> Option<Self> {
        let format = adapter.mcp_format()?;
        Some(Self {
            agent: adapter.id.to_owned(),
            format,
            path: at.place(adapter, AccountPath::Mcp),
            home: at.home().to_path_buf(),
        })
    }

    pub fn agent(&self) -> &str {
        &self.agent
    }

    pub fn format(&self) -> McpFormat {
        self.format
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// What writing a server found.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Put {
    /// Hide wrote its entry.
    Written,
    /// Hide's entry already says this, so nothing was written (B27).
    Unchanged,
    /// The file has an entry of this name that is not Hide's: the
    /// operator's, which stays as it is (B22, D-16).
    Theirs,
    /// The agent does not take this server's transport, or its name (B14).
    Unsupported,
}

/// One entry of an agent's MCP file, Hide's or not.
#[derive(Debug)]
pub struct Found {
    pub name: String,
    /// The server and its keys, `None` for an entry Hide cannot read in
    /// the file's shape.
    pub server: Option<(Server, Keys)>,
    /// Hide's record names it as Hide's.
    pub hide: bool,
}

/// Writes `server` with `keys` as Hide's entry in `file`, leaving every
/// other entry and byte of the file as it is. A key the server names and
/// `keys` lacks, or one that starts with `!`, is refused before anything is
/// written.
pub fn put(file: &McpFile, server: &Server, keys: &Keys) -> Result<Put, InstallFailure> {
    let shape = shape(file.format);
    if !accepts(file.format, server.reach.transport())
        || !valid_name(&server.name)
        || shape.reserved.contains(&server.name.as_str())
    {
        return Ok(Put::Unsupported);
    }
    let wanted: Keys = server
        .reach
        .key_names()
        .iter()
        .filter_map(|name| keys.get(name).map(|key| (name.clone(), key.clone())))
        .collect();
    let refused = |refusal: KeyRefusal| InstallFailure::Refused {
        path: file.path.display().to_string(),
        detail: match refusal {
            KeyRefusal::Missing(name) => format!("{name} of {} has no value", server.name),
            KeyRefusal::Runs(name) => format!(
                "{name} of {} starts with !, which Pi and omp run as a command",
                server.name
            ),
        },
    };
    // Rendered first, so a refused key writes nothing.
    let entry = match shape.syntax {
        Syntax::Json => Entry::Json(format::json_entry(shape, server, &wanted).map_err(refused)?),
        Syntax::Toml => Entry::Toml(format::toml_entry(shape, server, &wanted).map_err(refused)?),
    };
    let _held = hold(&file.home)?;
    let mut record = read_record(&file.home)?;
    let recorded = record.iter().any(|kept| kept.is(file, &server.name));
    let _agent = agent_lock(file)?;
    let mut document = Document::read(shape, &file.path)?;
    let found = document.find(shape, &file.path, &server.name)?;
    if !recorded && !found.is_empty() {
        return Ok(Put::Theirs);
    }
    let current = found.iter().find(|(table, _)| *table == 0);
    if found.len() == 1
        && current.is_some_and(|(_, read)| {
            read.as_ref()
                .is_some_and(|(read, keys)| read == server && *keys == wanted)
        })
    {
        return Ok(Put::Unchanged);
    }
    // An entry in a table Hide does not write (one the operator had there
    // before Hide took it in) moves to Hide's; one in Hide's is replaced
    // where it is.
    for (table, _) in found.iter().filter(|(table, _)| *table != 0) {
        document.remove(shape, &file.path, *table, &server.name)?;
    }
    document.insert(shape, &file.path, &server.name, entry)?;
    if recorded {
        document.write(&file.path)?;
        return Ok(Put::Written);
    }
    // The record names the entry before the file holds it, so a crash
    // between the two leaves a name Hide takes as its own with no entry,
    // never an entry of Hide's no record names. A failed write takes the
    // name back out: left there, it would make an entry the operator adds
    // later look like Hide's.
    remember(&mut record, file, &server.name)?;
    write_record(&file.home, &record)?;
    if let Err(failure) = document.write(&file.path) {
        record.retain(|kept| !kept.is(file, &server.name));
        let _ = write_record(&file.home, &record);
        return Err(failure);
    }
    Ok(Put::Written)
}

/// Takes Hide's entry `name` out of `file`; an entry Hide's record does not
/// name stays. Answers whether an entry went.
pub fn take(file: &McpFile, name: &str) -> Result<bool, InstallFailure> {
    if !record_path(&file.home).exists() {
        return Ok(false);
    }
    let _held = hold(&file.home)?;
    let mut record = read_record(&file.home)?;
    if !record.iter().any(|kept| kept.is(file, name)) {
        return Ok(false);
    }
    let removed = take_out(file, name)?;
    record.retain(|kept| !kept.is(file, name));
    write_record(&file.home, &record)?;
    Ok(removed)
}

/// Takes every entry Hide keeps in `file` out: the agent turned off, or its
/// profile or machine gone (B25, B30). Answers the names that went.
pub fn take_all(file: &McpFile) -> Result<Vec<String>, InstallFailure> {
    if !record_path(&file.home).exists() {
        return Ok(Vec::new());
    }
    let _held = hold(&file.home)?;
    let mut record = read_record(&file.home)?;
    let mut gone = Vec::new();
    for kept in record.iter().filter(|kept| kept.in_file(file)) {
        if take_out(file, &kept.name)? {
            gone.push(kept.name.clone());
        }
    }
    record.retain(|kept| !kept.in_file(file));
    write_record(&file.home, &record)?;
    Ok(gone)
}

/// The entries of `file`, each with whether it is Hide's: what the
/// operator put there by hand is the rest (B19, B21, B22).
pub fn read(file: &McpFile) -> Result<Vec<Found>, InstallFailure> {
    let shape = shape(file.format);
    let record = read_record(&file.home)?;
    let document = Document::read(shape, &file.path)?;
    Ok(document
        .entries(shape, &file.path)?
        .into_iter()
        .map(|(_, name, server)| Found {
            hide: record.iter().any(|kept| kept.is(file, &name)),
            name,
            server,
        })
        .collect())
}

/// Makes the operator's entry `name` in `file` Hide's, once its server is
/// in Hide's store (B20, D-17): Hide's record names it, so turning the
/// server off or deleting it takes it out, and the next write puts Hide's
/// entry in its place. Answers whether the file has such an entry.
pub fn adopt(file: &McpFile, name: &str) -> Result<bool, InstallFailure> {
    let shape = shape(file.format);
    let _held = hold(&file.home)?;
    let mut record = read_record(&file.home)?;
    if record.iter().any(|kept| kept.is(file, name)) {
        return Ok(true);
    }
    let document = Document::read(shape, &file.path)?;
    if document.find(shape, &file.path, name)?.is_empty() {
        return Ok(false);
    }
    remember(&mut record, file, name)?;
    write_record(&file.home, &record)?;
    Ok(true)
}

/// Every file Hide keeps an entry in under `home`, with the names: what a
/// machine's removal takes out (B30).
pub fn recorded(home: &Path) -> Result<Vec<(McpFile, Vec<String>)>, InstallFailure> {
    let mut files: Vec<(McpFile, Vec<String>)> = Vec::new();
    for kept in read_record(home)? {
        let file = McpFile {
            agent: kept.agent.clone(),
            format: kept.format,
            path: kept.file.clone(),
            home: home.to_path_buf(),
        };
        match files.iter_mut().find(|(each, _)| *each == file) {
            Some((_, names)) => names.push(kept.name),
            None => files.push((file, vec![kept.name])),
        }
    }
    Ok(files)
}

/// Takes `name` out of every table of `file` it is in, under the agent's
/// lock; the caller holds Hide's.
fn take_out(file: &McpFile, name: &str) -> Result<bool, InstallFailure> {
    let shape = shape(file.format);
    if !file.path.exists() {
        return Ok(false);
    }
    let _agent = agent_lock(file)?;
    let mut document = Document::read(shape, &file.path)?;
    let found = document.find(shape, &file.path, name)?;
    if found.is_empty() {
        return Ok(false);
    }
    for (table, _) in &found {
        document.remove(shape, &file.path, *table, name)?;
    }
    document.write(&file.path)?;
    Ok(true)
}

/// The lock the agent itself takes before it rewrites the file, held across
/// Hide's read and write: Claude Code's on its global config, which it
/// rewrites while it runs (`crate::folder_trust`). The other agents write
/// their MCP file only from their own `mcp add`, which the compare before
/// Hide's replacement catches.
fn agent_lock(
    file: &McpFile,
) -> Result<Option<crate::folder_trust::lock_folder::LockFolder>, InstallFailure> {
    if file.agent == hide_agent_adapter::AgentId::ClaudeCode.adapter().id {
        if let Some(parent) = file.path.parent() {
            hide_platform::fs::private::create_dir_all(parent).map_err(|error| {
                InstallFailure::NotWritable {
                    path: file.path.display().to_string(),
                    detail: error.to_string(),
                }
            })?;
        }
        return crate::folder_trust::claude::lock(&file.path).map(Some);
    }
    Ok(None)
}

/// What an entry says: its server and keys, `None` when Hide cannot read
/// it in the file's shape.
type Readout = Option<(Server, Keys)>;

/// An entry ready for a file of either syntax.
enum Entry {
    Json(Value),
    Toml(toml_edit::Table),
}

/// An agent's MCP file as read, in its syntax.
enum Document {
    Json(HookDocument),
    Toml(TomlFile),
}

fn shape_failure(path: &Path, detail: String) -> InstallFailure {
    InstallFailure::UnexpectedShape {
        path: path.display().to_string(),
        detail,
    }
}

impl Document {
    fn read(shape: &Shape, path: &Path) -> Result<Self, InstallFailure> {
        Ok(match shape.syntax {
            Syntax::Json => Self::Json(
                read_document(path)?
                    .unwrap_or_else(|| HookDocument::new(Value::Object(Map::new()))),
            ),
            Syntax::Toml => Self::Toml(TomlFile::read(path)?),
        })
    }

    fn write(&self, path: &Path) -> Result<(), InstallFailure> {
        match self {
            Self::Json(document) => write_document(path, document),
            Self::Toml(document) => document.write(path),
        }
    }

    /// The tables of servers the shape reads, the one Hide writes first.
    fn tables(shape: &Shape) -> impl Iterator<Item = (usize, &'static [&'static str])> {
        std::iter::once(shape.container)
            .chain(shape.also.iter().copied())
            .enumerate()
    }

    /// Every entry, with the table it is in.
    fn entries(
        &self,
        shape: &Shape,
        path: &Path,
    ) -> Result<Vec<(usize, String, Readout)>, InstallFailure> {
        let mut entries = Vec::new();
        for (table, keys) in Self::tables(shape) {
            match self {
                Self::Json(document) => {
                    let Some(servers) = json_table(document, keys, path)? else {
                        continue;
                    };
                    for (name, entry) in servers {
                        if table == 0 && shape.reserved.contains(&name.as_str()) {
                            continue;
                        }
                        entries.push((
                            table,
                            name.clone(),
                            format::json_server(shape, name, entry),
                        ));
                    }
                }
                Self::Toml(document) => {
                    let Some(servers) = toml_table(document.document.as_item(), keys, path)? else {
                        continue;
                    };
                    for (name, entry) in servers.iter() {
                        if table == 0 && shape.reserved.contains(&name) {
                            continue;
                        }
                        entries.push((
                            table,
                            name.to_owned(),
                            format::toml_server(shape, name, entry),
                        ));
                    }
                }
            }
        }
        Ok(entries)
    }

    /// The tables holding an entry `name`, with what it says.
    fn find(
        &self,
        shape: &Shape,
        path: &Path,
        name: &str,
    ) -> Result<Vec<(usize, Readout)>, InstallFailure> {
        Ok(self
            .entries(shape, path)?
            .into_iter()
            .filter(|(_, each, _)| each == name)
            .map(|(table, _, server)| (table, server))
            .collect())
    }

    fn remove(
        &mut self,
        shape: &Shape,
        path: &Path,
        table: usize,
        name: &str,
    ) -> Result<(), InstallFailure> {
        let keys = Self::tables(shape)
            .find(|(index, _)| *index == table)
            .map(|(_, keys)| keys)
            .expect("a table the entries came from");
        match self {
            Self::Json(document) => {
                if let Some(servers) = json_table_mut(document, keys, path, false)? {
                    servers.shift_remove(name);
                }
            }
            Self::Toml(document) => {
                if let Some(servers) =
                    toml_item_mut(document.document.as_item_mut(), keys, path, false)?
                    && let Some(servers) = servers.as_table_like_mut()
                {
                    servers.remove(name);
                }
            }
        }
        Ok(())
    }

    fn insert(
        &mut self,
        shape: &Shape,
        path: &Path,
        name: &str,
        entry: Entry,
    ) -> Result<(), InstallFailure> {
        match (self, entry) {
            (Self::Json(document), Entry::Json(entry)) => {
                json_table_mut(document, shape.container, path, true)?
                    .expect("a table made when missing")
                    .insert(name.to_owned(), entry);
            }
            (Self::Toml(document), Entry::Toml(entry)) => {
                match toml_item_mut(document.document.as_item_mut(), shape.container, path, true)?
                    .expect("a table made when missing")
                {
                    // `mcp_servers = { ... }` written inline keeps its form.
                    Item::Value(toml_edit::Value::InlineTable(servers)) => {
                        servers.insert(
                            name,
                            toml_edit::Value::InlineTable(entry.into_inline_table()),
                        );
                    }
                    servers => {
                        servers
                            .as_table_like_mut()
                            .expect("a table checked on the way")
                            .insert(name, Item::Table(entry));
                    }
                }
            }
            _ => unreachable!("an entry is rendered in its file's syntax"),
        }
        Ok(())
    }
}

/// The JSON object at `keys` below the root, `None` when a step is missing.
fn json_table<'a>(
    document: &'a Value,
    keys: &[&str],
    path: &Path,
) -> Result<Option<&'a Map<String, Value>>, InstallFailure> {
    let mut at = document
        .as_object()
        .ok_or_else(|| shape_failure(path, "the file is not a JSON object".to_owned()))?;
    for key in keys {
        match at.get(*key) {
            None => return Ok(None),
            Some(Value::Object(next)) => at = next,
            Some(_) => {
                return Err(shape_failure(
                    path,
                    format!("{} is not an object", keys.join(".")),
                ));
            }
        }
    }
    Ok(Some(at))
}

fn json_table_mut<'a>(
    document: &'a mut Value,
    keys: &[&str],
    path: &Path,
    make: bool,
) -> Result<Option<&'a mut Map<String, Value>>, InstallFailure> {
    let mut at = document
        .as_object_mut()
        .ok_or_else(|| shape_failure(path, "the file is not a JSON object".to_owned()))?;
    for key in keys {
        if !at.contains_key(*key) {
            if !make {
                return Ok(None);
            }
            at.insert((*key).to_owned(), Value::Object(Map::new()));
        }
        at = match at.get_mut(*key) {
            Some(Value::Object(next)) => next,
            _ => {
                return Err(shape_failure(
                    path,
                    format!("{} is not an object", keys.join(".")),
                ));
            }
        };
    }
    Ok(Some(at))
}

/// The TOML table at `keys` below the root, `None` when a step is missing.
fn toml_table<'a>(
    root: &'a Item,
    keys: &[&str],
    path: &Path,
) -> Result<Option<&'a dyn TableLike>, InstallFailure> {
    let mut at = root.as_table_like().expect("a document's root is a table");
    for key in keys {
        match at.get(key) {
            None => return Ok(None),
            Some(item) => {
                at = item.as_table_like().ok_or_else(|| {
                    shape_failure(path, format!("{} is not a table", keys.join(".")))
                })?;
            }
        }
    }
    Ok(Some(at))
}

/// The TOML item of the table at `keys` below the root, made as a table
/// with no header line of its own when `make` asks and it is missing.
fn toml_item_mut<'a>(
    root: &'a mut Item,
    keys: &[&str],
    path: &Path,
    make: bool,
) -> Result<Option<&'a mut Item>, InstallFailure> {
    let not_table = || shape_failure(path, format!("{} is not a table", keys.join(".")));
    let mut at = root;
    for key in keys {
        let table = at.as_table_like_mut().ok_or_else(not_table)?;
        if !table.contains_key(key) {
            if !make {
                return Ok(None);
            }
            let mut made = toml_edit::Table::new();
            made.set_implicit(true);
            table.insert(key, Item::Table(made));
        }
        at = table.get_mut(key).expect("a key just found or made");
    }
    if at.as_table_like().is_none() {
        return Err(not_table());
    }
    Ok(Some(at))
}

/// One entry Hide wrote, by the file it is in.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Recorded {
    agent: String,
    format: McpFormat,
    file: PathBuf,
    name: String,
}

impl Recorded {
    fn of(file: &McpFile, name: &str) -> Self {
        Self {
            agent: file.agent.clone(),
            format: file.format,
            file: file.path.clone(),
            name: name.to_owned(),
        }
    }

    fn in_file(&self, file: &McpFile) -> bool {
        self.file == file.path && self.agent == file.agent
    }

    fn is(&self, file: &McpFile, name: &str) -> bool {
        self.in_file(file) && self.name == name
    }
}

/// Where Hide keeps its own records of the agents' files under a home,
/// beside its folder trust record (`crate::folder_trust`).
fn agent_hooks_dir(home: &Path) -> PathBuf {
    home.join(".hide").join("agent-hooks")
}

fn record_path(home: &Path) -> PathBuf {
    agent_hooks_dir(home).join("mcp.json")
}

/// Hide's lock on its MCP record of `home`, held across a whole change, so
/// two changes never read the same record and drop each other's entry.
fn hold(home: &Path) -> Result<hide_platform::fs::lock::Lock, InstallFailure> {
    let path = agent_hooks_dir(home).join("mcp.lock");
    let failure = |detail: String| InstallFailure::NotWritable {
        path: path.display().to_string(),
        detail,
    };
    hide_platform::fs::private::create_dir_all(&agent_hooks_dir(home))
        .map_err(|error| failure(error.to_string()))?;
    let file = hide_platform::fs::private::open_or_create_file(&path)
        .map_err(|error| failure(error.to_string()))?;
    match hide_platform::fs::lock::lock_file(
        file,
        hide_platform::fs::lock::Mode::Exclusive,
        RECORD_WAIT,
        &|| false,
    )
    .map_err(|error| failure(error.to_string()))?
    {
        hide_platform::fs::lock::Waited::Locked(held) => Ok(held),
        hide_platform::fs::lock::Waited::TimedOut | hide_platform::fs::lock::Waited::Cancelled => {
            Err(failure(format!(
                "another change of Hide's MCP entries held the lock for more than {} s",
                RECORD_WAIT.as_secs()
            )))
        }
    }
}

fn read_record(home: &Path) -> Result<Vec<Recorded>, InstallFailure> {
    let path = record_path(home);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| InstallFailure::Unreadable {
            path: path.display().to_string(),
            detail: error.to_string(),
        }),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(InstallFailure::Unreadable {
            path: path.display().to_string(),
            detail: error.to_string(),
        }),
    }
}

fn write_record(home: &Path, record: &[Recorded]) -> Result<(), InstallFailure> {
    let path = record_path(home);
    let failure = |detail: String| InstallFailure::NotWritable {
        path: path.display().to_string(),
        detail,
    };
    let bytes = serde_json::to_vec(record).map_err(|error| failure(error.to_string()))?;
    hide_platform::fs::atomic::write_file(&path, &bytes, hide_platform::fs::Access::Private)
        .map(drop)
        .map_err(|error| failure(error.to_string()))
}

/// Adds `name` in `file` to the record. Entries of files that are gone go
/// first: a file the operator or an uninstall removed holds nothing of
/// Hide's, so the cap is only reached by entries of files that exist, and
/// at the cap the add is refused, since none of those can be dropped.
fn remember(record: &mut Vec<Recorded>, file: &McpFile, name: &str) -> Result<(), InstallFailure> {
    record.retain(|kept| kept.file.exists());
    if record.len() >= RECORD_LIMIT {
        return Err(InstallFailure::NotWritable {
            path: record_path(&file.home).display().to_string(),
            detail: format!(
                "Hide's MCP record already holds {RECORD_LIMIT} entries of files that exist"
            ),
        });
    }
    record.push(Recorded::of(file, name));
    Ok(())
}

#[cfg(test)]
#[path = "mcp/tests.rs"]
mod tests;
