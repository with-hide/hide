//! The MCP servers Hide writes into each agent's own MCP file (PRD
//! skill-mcp-manager B11, B14, B27; docs/agent-hooks.md, MCP servers).
//!
//! An agent's definition names the file (`[account.paths] mcp`, profile
//! accounts included) and its shape (`mcp_format`,
//! [`hide_agent_adapter::McpFormat`]); this module holds one converter per
//! shape, never one per agent, and each shape says which transports it
//! takes ([`accepts`]). A write puts one entry in and a removal takes one
//! out, and every other byte of the file is kept: a JSON file goes through
//! the same lossless read and compare-before-replace as the hook files, a
//! TOML file through `toml_edit`, which keeps the rest of the document as
//! it was written. Which entries are Hide's is the store's record
//! (`hide_kit::store`), which compares an entry's [`Server::digest`] with
//! the one it wrote; this module decides no ownership.
//!
//! A server as Hide keeps it ([`Server`]) holds the names of its variables
//! and headers and never their values: those are the operator's keys, which
//! live in Hide's owner-only key file and reach a converter as [`Keys`] only
//! at the moment it writes them into the agent's file, so a server can be
//! logged, reported and sent to the screen without carrying one.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

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
/// The most arguments a local server's command takes, and the most bytes
/// of its command or of one argument.
pub const MAX_ARGS: usize = 64;
pub const MAX_ARG_BYTES: usize = 4096;
/// The longest address of a remote server.
pub const MAX_URL_BYTES: usize = 2048;
/// The most variables or headers a server names, and the longest name.
pub const MAX_KEY_NAMES: usize = 64;
pub const MAX_KEY_NAME_BYTES: usize = 256;

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

impl Server {
    /// Why Hide keeps no server as `self` says it, the fields the operator
    /// filled or a paste gave (B8): a name every agent's file holds, a
    /// command or an `http(s)` address, and sizes within Hide's limits.
    pub fn check(&self) -> Result<(), String> {
        if !valid_name(&self.name) {
            return Err(format!(
                "a server's name is 1 to {MAX_NAME_BYTES} letters, digits, - or _"
            ));
        }
        match &self.reach {
            Reach::Local { command, args, .. } => {
                if command.trim().is_empty() {
                    return Err("a local server needs the command that starts it".to_owned());
                }
                if args.len() > MAX_ARGS {
                    return Err(format!("a command takes at most {MAX_ARGS} arguments"));
                }
                if std::iter::once(command)
                    .chain(args)
                    .any(|word| word.len() > MAX_ARG_BYTES || word.contains('\0'))
                {
                    return Err(format!(
                        "a command or argument is at most {MAX_ARG_BYTES} bytes, with no NUL"
                    ));
                }
            }
            Reach::Remote { url, .. } => {
                let scheme = url.split_once("://").map(|(scheme, _)| scheme);
                if !matches!(scheme, Some("http" | "https"))
                    || url.len() > MAX_URL_BYTES
                    || url.chars().any(char::is_whitespace)
                {
                    return Err(format!(
                        "a remote server's address is an http or https address of at most {MAX_URL_BYTES} bytes"
                    ));
                }
            }
        }
        let names = self.reach.key_names();
        if names.len() > MAX_KEY_NAMES {
            return Err(format!(
                "a server names at most {MAX_KEY_NAMES} variables or headers"
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for name in names {
            if name.is_empty()
                || name.len() > MAX_KEY_NAME_BYTES
                || name.chars().any(|character| {
                    character == '='
                        || character == ':'
                        || character.is_whitespace()
                        || character.is_control()
                })
            {
                return Err(format!(
                    "a variable's or header's name is 1 to {MAX_KEY_NAME_BYTES} bytes with no space, = or :"
                ));
            }
            if !seen.insert(name.as_str()) {
                return Err(format!("{name} is named twice"));
            }
        }
        Ok(())
    }

    /// Why `keys` do not give every variable and header `self` names a
    /// value an agent's file can carry: each needs one that is not empty
    /// and does not start with `!`, which Pi and omp run as a command. The
    /// reason names the key, never its value.
    pub fn check_keys(&self, keys: &Keys) -> Result<(), String> {
        for name in self.reach.key_names() {
            match keys.get(name).map(Secret::expose) {
                None | Some("") => return Err(format!("{name} has no value")),
                Some(value) if value.starts_with('!') => {
                    return Err(format!(
                        "{name} starts with !, which Pi and omp run as a command"
                    ));
                }
                Some(_) => {}
            }
        }
        Ok(())
    }
}

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

/// Whether `format` can hold a server named `name` in the table Hide
/// writes: OpenCode reads a server named `servers` there as its second
/// shape's table.
pub fn takes_name(format: McpFormat, name: &str) -> bool {
    valid_name(name) && !shape(format).reserved.contains(&name)
}

/// The version a local server's command runs, for the short version beside
/// its name (B36).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Pin {
    /// The package or image names this version.
    Version(String),
    /// An npm runner's package that names no version, or `latest`, so each
    /// start runs the newest ('npm 최신').
    NpmLatest,
    /// A program on the machine, or another runner's package that names no
    /// version.
    Unpinned,
}

/// What `command` with `args` runs, as [`Pin`] says it.
pub fn pin(command: &str, args: &[String]) -> Pin {
    let words: Vec<String> = std::iter::once(command.to_owned())
        .chain(args.iter().cloned())
        .collect();
    let (program, package) = paste::package_of(&words);
    let Some(package) = package else {
        return Pin::Unpinned;
    };
    let npm = matches!(
        program.as_str(),
        "npx" | "bunx" | "pnpx" | "pnpm" | "yarn" | "bun"
    );
    let version = match program.as_str() {
        // `name@1.2.3`, `@scope/name@1.2.3`.
        _ if npm => package
            .rsplit_once('@')
            .filter(|(name, _)| !name.is_empty())
            .map(|(_, version)| version),
        // `pkg==1.2.3`, or uv's `pkg@1.2.3`.
        "uvx" | "uv" | "pipx" => package
            .split_once("==")
            .or_else(|| package.split_once('@'))
            .map(|(_, version)| version),
        // `ghcr.io/org/image:tag`; a registry's port is before a `/`.
        "docker" | "podman" => package
            .rsplit_once(':')
            .filter(|(_, tag)| !tag.contains('/'))
            .map(|(_, tag)| tag),
        _ => None,
    };
    match version.filter(|version| !version.is_empty()) {
        Some("latest") | None if npm => Pin::NpmLatest,
        Some("latest") | None => Pin::Unpinned,
        Some(version) => Pin::Version(version.to_owned()),
    }
}

impl Server {
    /// What the store compares an agent's entry with (`hide_kit::store`):
    /// the server's name, how it is reached and the names of its keys, in a
    /// fixed order, never a key's value, so the digest can be kept, logged
    /// and sent where a key cannot. A key changed in Hide is written by
    /// the key's own change, not found by a digest.
    pub fn digest(&self) -> String {
        use sha2::Digest as _;
        let mut reach = self.reach.clone();
        match &mut reach {
            Reach::Local { env, .. } => env.sort(),
            Reach::Remote { headers, .. } => headers.sort(),
        }
        let text = serde_json::to_string(&(&self.name, &reach)).expect("a server serializes");
        sha2::Sha256::digest(text.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

/// One agent's MCP file in one account.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpFile {
    agent: String,
    format: McpFormat,
    path: PathBuf,
}

impl McpFile {
    /// `adapter`'s MCP file in the account `at`, `None` for an agent Hide
    /// writes no MCP server for.
    pub fn of(adapter: &AgentAdapter, at: AccountAt<'_>) -> Option<Self> {
        adapter.mcp_format()?;
        // Claude Code reads its servers, as its folder trust, from its older
        // `.config.json` while that file has content.
        let older = (adapter.id == crate::AgentRuntime::ClaudeCode.dialect().adapter().id)
            .then(|| crate::AgentRuntime::ClaudeCode.older_config(at))
            .flatten();
        Self::at(
            adapter,
            older.unwrap_or_else(|| at.place(adapter, AccountPath::Mcp)),
        )
    }

    /// `adapter`'s MCP file at `path`, a place its definition named once:
    /// what a record of a place kept after its account went.
    pub fn at(adapter: &AgentAdapter, path: PathBuf) -> Option<Self> {
        Some(Self {
            agent: adapter.id.to_owned(),
            format: adapter.mcp_format()?,
            path,
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

/// What a write did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Written {
    /// The file holds the server's entry now.
    Written,
    /// The entry already said this, so no byte changed (B27).
    Unchanged,
    /// The agent does not take this server's transport, or its name (B14).
    Unsupported,
}

/// One entry of an agent's MCP file.
#[derive(Debug)]
pub struct Found {
    pub name: String,
    /// The server and its keys, `None` for an entry Hide cannot read in
    /// the file's shape.
    pub server: Option<(Server, Keys)>,
    /// It reads as a server and holds no field the shape's reader leaves out
    /// (a timeout, an `enabled` switch, a tool list): only then does writing
    /// the server back lose nothing of it.
    pub whole: bool,
}

/// Every entry of `file`, in the tables the agent reads (B19); none when
/// the file is not there.
pub fn entries(file: &McpFile) -> Result<Vec<Found>, InstallFailure> {
    let shape = shape(file.format);
    let document = Document::read(shape, &file.path)?;
    Ok(document
        .entries(shape, &file.path)?
        .into_iter()
        .map(|(_, name, server, whole)| Found {
            name,
            server,
            whole,
        })
        .collect())
}

/// The entry `name` of `file`: the first the agent reads, in the table
/// Hide writes before any other.
pub fn entry(file: &McpFile, name: &str) -> Result<Option<Found>, InstallFailure> {
    Ok(entries(file)?.into_iter().find(|found| found.name == name))
}

/// Writes `server` with `keys` as the entry `name` of `file`, in place of
/// any entry of that name, and leaves every other entry and byte of the
/// file as it is (B11). Whose the entry is, is the caller's to decide: the
/// store writes only over its own (B22). A key the server names and `keys`
/// lacks, or one that starts with `!`, is refused before anything is
/// written.
pub fn write(file: &McpFile, server: &Server, keys: &Keys) -> Result<Written, InstallFailure> {
    let shape = shape(file.format);
    if !accepts(file.format, server.reach.transport()) || !takes_name(file.format, &server.name) {
        return Ok(Written::Unsupported);
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
    let _agent = agent_lock(file)?;
    let mut document = Document::read(shape, &file.path)?;
    let found = document.find(shape, &file.path, &server.name)?;
    if let [(0, Some((read, read_keys)))] = found.as_slice()
        && read == server
        && *read_keys == wanted
    {
        return Ok(Written::Unchanged);
    }
    // An entry in a table Hide does not write (one the agent's own second
    // shape holds) moves to Hide's; one in Hide's is replaced where it is.
    for (table, _) in found.iter().filter(|(table, _)| *table != 0) {
        document.remove(shape, &file.path, *table, &server.name)?;
    }
    document.insert(shape, &file.path, &server.name, entry)?;
    document.write(&file.path)?;
    Ok(Written::Written)
}

/// Takes the entry `name` out of every table of `file` that holds it.
/// Answers whether one went.
pub fn remove(file: &McpFile, name: &str) -> Result<bool, InstallFailure> {
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

/// An entry taken out of an agent's file whole, spelled as the file spelled
/// it, for [`put_back`]. Its text holds the entry's keys, so it prints none
/// and is kept only in a private file.
#[derive(Clone, Serialize, Deserialize)]
pub struct SetAside {
    /// Each table that held it, by its place among the shape's tables, with
    /// the entry's text in the file's syntax.
    entries: Vec<(usize, String)>,
}

impl fmt::Debug for SetAside {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "SetAside({} entries)", self.entries.len())
    }
}

/// The key a TOML entry's text is kept under.
const SET_ASIDE_KEY: &str = "entry";

/// Takes the entry `name` out of every table of `file` that holds it, as the
/// file spelled it, every field kept. `keep` gets it, with what the first
/// table's entry says, before the file is rewritten, so an entry it cannot
/// keep stays where it is. False when there is no entry `name`.
pub fn set_aside(
    file: &McpFile,
    name: &str,
    keep: impl FnOnce(&SetAside, Option<&(Server, Keys)>) -> Result<(), String>,
) -> Result<bool, InstallFailure> {
    let shape = shape(file.format);
    if !file.path.exists() {
        return Ok(false);
    }
    let _agent = agent_lock(file)?;
    let mut document = Document::read(shape, &file.path)?;
    let found = document.find(shape, &file.path, name)?;
    let Some((_, readout)) = found.first() else {
        return Ok(false);
    };
    let mut entries = Vec::new();
    for (table, _) in &found {
        entries.push((*table, document.text(shape, &file.path, *table, name)?));
    }
    let set_aside = SetAside { entries };
    keep(&set_aside, readout.as_ref()).map_err(|detail| InstallFailure::Refused {
        path: file.path.display().to_string(),
        detail: format!("{name} stays there: {detail}"),
    })?;
    for (table, _) in &found {
        document.remove(shape, &file.path, *table, name)?;
    }
    document.write(&file.path)?;
    Ok(true)
}

/// Puts what [`set_aside`] took back into `file` as it was. True when it is
/// there now, put back or found back already; false, with nothing written,
/// when the file holds another entry `name`.
pub fn put_back(file: &McpFile, name: &str, set_aside: &SetAside) -> Result<bool, InstallFailure> {
    let shape = shape(file.format);
    let _agent = agent_lock(file)?;
    let mut document = Document::read(shape, &file.path)?;
    let found = document.find(shape, &file.path, name)?;
    if !found.is_empty() {
        let now = found
            .iter()
            .map(|(table, _)| Ok((*table, document.text(shape, &file.path, *table, name)?)))
            .collect::<Result<Vec<_>, InstallFailure>>()?;
        return Ok(now == set_aside.entries);
    }
    for (table, text) in &set_aside.entries {
        document.put(shape, &file.path, *table, name, text)?;
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
    if file.agent != hide_agent_adapter::AgentId::ClaudeCode.adapter().id {
        return Ok(None);
    }
    if let Some(parent) = file.path.parent() {
        hide_platform::fs::private::create_dir_all(parent).map_err(|error| {
            InstallFailure::NotWritable {
                path: file.path.display().to_string(),
                detail: error.to_string(),
            }
        })?;
    }
    crate::folder_trust::claude::lock(&file.path).map(Some)
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
    ) -> Result<Vec<(usize, String, Readout, bool)>, InstallFailure> {
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
                        let server = format::json_server(shape, name, entry);
                        let whole = server.is_some() && format::json_whole(shape, entry);
                        entries.push((table, name.clone(), server, whole));
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
                        let server = format::toml_server(shape, name, entry);
                        let whole = server.is_some() && format::toml_whole(shape, entry);
                        entries.push((table, name.to_owned(), server, whole));
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
            .filter(|(_, each, _, _)| each == name)
            .map(|(table, _, server, _)| (table, server))
            .collect())
    }

    /// The keys of the shape's table `table`.
    fn table_keys(shape: &Shape, table: usize) -> &'static [&'static str] {
        Self::tables(shape)
            .find(|(index, _)| *index == table)
            .map(|(_, keys)| keys)
            .expect("a table the entries came from")
    }

    /// The text of the entry `name` in `table`, in the file's syntax.
    fn text(
        &self,
        shape: &Shape,
        path: &Path,
        table: usize,
        name: &str,
    ) -> Result<String, InstallFailure> {
        let keys = Self::table_keys(shape, table);
        let missing = || shape_failure(path, format!("{name} went while Hide read it"));
        match self {
            Self::Json(document) => {
                let entry = json_table(document, keys, path)?
                    .and_then(|servers| servers.get(name))
                    .ok_or_else(missing)?;
                Ok(entry.to_string())
            }
            Self::Toml(document) => {
                let mut entry = toml_table(document.document.as_item(), keys, path)?
                    .and_then(|servers| servers.get(name))
                    .ok_or_else(missing)?
                    .clone();
                unplaced(&mut entry);
                let mut kept = toml_edit::DocumentMut::new();
                kept.insert(SET_ASIDE_KEY, entry);
                Ok(kept.to_string())
            }
        }
    }

    /// Puts the entry `text` [`Document::text`] read back as `name` in
    /// `table`.
    fn put(
        &mut self,
        shape: &Shape,
        path: &Path,
        table: usize,
        name: &str,
        text: &str,
    ) -> Result<(), InstallFailure> {
        let keys = Self::table_keys(shape, table);
        let unreadable = |detail: String| {
            shape_failure(
                path,
                format!("the {name} Hide set aside cannot be read: {detail}"),
            )
        };
        match self {
            Self::Json(document) => {
                let entry: Value =
                    serde_json::from_str(text).map_err(|error| unreadable(error.to_string()))?;
                json_table_mut(document, keys, path, true)?
                    .expect("a table made when missing")
                    .insert(name.to_owned(), entry);
            }
            Self::Toml(document) => {
                let mut kept: toml_edit::DocumentMut =
                    text.parse().map_err(|error: toml_edit::TomlError| {
                        unreadable(error.message().to_owned())
                    })?;
                let mut entry = kept
                    .remove(SET_ASIDE_KEY)
                    .ok_or_else(|| unreadable("it is empty".to_owned()))?;
                unplaced(&mut entry);
                match toml_item_mut(document.document.as_item_mut(), keys, path, true)?
                    .expect("a table made when missing")
                {
                    Item::Value(toml_edit::Value::InlineTable(servers)) => {
                        let entry = match entry {
                            Item::Table(entry) => {
                                toml_edit::Value::InlineTable(entry.into_inline_table())
                            }
                            Item::Value(entry) => entry,
                            _ => return Err(unreadable("it is not a table".to_owned())),
                        };
                        servers.insert(name, entry);
                    }
                    servers => {
                        servers
                            .as_table_like_mut()
                            .expect("a table checked on the way")
                            .insert(name, entry);
                    }
                }
            }
        }
        Ok(())
    }

    fn remove(
        &mut self,
        shape: &Shape,
        path: &Path,
        table: usize,
        name: &str,
    ) -> Result<(), InstallFailure> {
        let keys = Self::table_keys(shape, table);
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
        self.drop_emptied(path, keys)
    }

    /// Takes out the table at `keys` when a removal left it empty, and each
    /// table above it that it was the last of, so a file Hide added them to
    /// is as it was before. An empty table the operator kept there goes too,
    /// which every agent reads as no server.
    fn drop_emptied(&mut self, path: &Path, keys: &[&str]) -> Result<(), InstallFailure> {
        for depth in (1..=keys.len()).rev() {
            let (parents, last) = keys[..depth].split_at(depth - 1);
            let emptied = match self {
                Self::Json(document) => {
                    let Some(parent) = json_table_mut(document, parents, path, false)? else {
                        return Ok(());
                    };
                    let emptied = parent
                        .get(last[0])
                        .and_then(Value::as_object)
                        .is_some_and(Map::is_empty);
                    if emptied {
                        parent.shift_remove(last[0]);
                    }
                    emptied
                }
                Self::Toml(document) => {
                    let Some(parent) =
                        toml_item_mut(document.document.as_item_mut(), parents, path, false)?
                            .and_then(Item::as_table_like_mut)
                    else {
                        return Ok(());
                    };
                    let emptied = parent
                        .get(last[0])
                        .and_then(Item::as_table_like)
                        .is_some_and(TableLike::is_empty);
                    if emptied {
                        parent.remove(last[0]);
                    }
                    emptied
                }
            };
            if !emptied {
                return Ok(());
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

/// Drops where `item`'s tables sat in the file they came from, so they are
/// written where they are put rather than at that place of another file.
fn unplaced(item: &mut Item) {
    if let Item::Table(table) = item {
        table.set_position(None);
        for (_, inner) in table.iter_mut() {
            unplaced(inner);
        }
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

#[cfg(test)]
#[path = "mcp/tests.rs"]
mod tests;
