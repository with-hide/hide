//! One converter per MCP file shape ([`McpFormat`]), never one per agent:
//! where the shape keeps its servers, how a server's fields are spelled, and
//! which transports it takes. Each shape is a row of data ([`Shape`]); the
//! JSON and TOML code below reads and writes every row alike.

use hide_agent_adapter::McpFormat;
use serde_json::{Map, Value};
use toml_edit::{Array, InlineTable, Item, Table};

use super::{Keys, Reach, Remote, Secret, Server, Transport};

/// The syntax of a shape's file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Syntax {
    Json,
    Toml,
}

/// How one shape keeps its servers.
pub(super) struct Shape {
    pub(super) syntax: Syntax,
    /// The keys from the document's root to the table of servers by name,
    /// where Hide writes its own.
    pub(super) container: &'static [&'static str],
    /// Other tables of servers the agent reads too, whose entries count for
    /// a name being taken and for taking a server in (OpenCode 2's
    /// `mcp.servers`); Hide writes none there.
    pub(super) also: &'static [&'static [&'static str]],
    /// Names a server in `container` cannot have, because the agent reads a
    /// table of that name as one of `also`.
    pub(super) reserved: &'static [&'static str],
    pub(super) transports: &'static [Transport],
    /// The key naming a server's transport and its value for each, when the
    /// shape names it.
    kind: Option<Kinds>,
    /// A local server's command and arguments as one list under `command`
    /// (OpenCode's), rather than `command` and `args`.
    command_list: bool,
    env: &'static str,
    url: &'static str,
    headers: &'static str,
}

/// How a shape names a transport: its key, and the value for each one,
/// `None` where the shape writes no key for it.
struct Kinds {
    key: &'static str,
    local: Option<&'static str>,
    http: Option<&'static str>,
    sse: &'static str,
}

const STDIO_HTTP: &[Transport] = &[Transport::Stdio, Transport::Http];
const ALL: &[Transport] = &[Transport::Stdio, Transport::Http, Transport::Sse];

/// Each shape as its agents' documentation and their own `mcp add` write it
/// (docs/agent-hooks.md, MCP servers, has the sources).
pub(super) fn shape(format: McpFormat) -> &'static Shape {
    match format {
        // Claude Code 2.1.328 (`claude mcp add -s user`) and omp 18.7.0
        // (docs/mcp-config.md).
        McpFormat::Claude => &Shape {
            syntax: Syntax::Json,
            container: &["mcpServers"],
            also: &[],
            reserved: &[],
            transports: ALL,
            kind: Some(Kinds {
                key: "type",
                local: Some("stdio"),
                http: Some("http"),
                sse: "sse",
            }),
            command_list: false,
            env: "env",
            url: "url",
            headers: "headers",
        },
        // Codex 0.169.0 (`codex mcp add`): a `transport = "sse"` is read as
        // Streamable HTTP, so SSE is not taken.
        McpFormat::Codex => &Shape {
            syntax: Syntax::Toml,
            container: &["mcp_servers"],
            also: &[],
            reserved: &[],
            transports: STDIO_HTTP,
            kind: None,
            command_list: false,
            env: "env",
            url: "url",
            headers: "http_headers",
        },
        // Grok 1.0.77 (`grok mcp add`): no `type` for a local server or
        // Streamable HTTP, `type = "sse"` for SSE.
        McpFormat::Grok => &Shape {
            syntax: Syntax::Toml,
            container: &["mcp_servers"],
            also: &[],
            reserved: &[],
            transports: ALL,
            kind: Some(Kinds {
                key: "type",
                local: None,
                http: None,
                sse: "sse",
            }),
            command_list: false,
            env: "env",
            url: "url",
            headers: "headers",
        },
        // OpenCode's first shape, `mcp.<name>`, which every version from
        // 1.0 reads, OpenCode 2 beside its own `mcp.servers.<name>`; neither
        // takes SSE (OpenCode 2 has no SSE client).
        McpFormat::Opencode => &Shape {
            syntax: Syntax::Json,
            container: &["mcp"],
            also: &[&["mcp", "servers"]],
            reserved: &["servers"],
            transports: STDIO_HTTP,
            kind: Some(Kinds {
                key: "type",
                local: Some("local"),
                http: Some("remote"),
                sse: "remote",
            }),
            command_list: true,
            env: "environment",
            url: "url",
            headers: "headers",
        },
        // Pi 1.0.4 (`pi mcp add`): no `type`, and SSE refused ("legacy SSE
        // transport is not supported").
        McpFormat::Pi => &Shape {
            syntax: Syntax::Json,
            container: &["mcpServers"],
            also: &[],
            reserved: &[],
            transports: STDIO_HTTP,
            kind: None,
            command_list: false,
            env: "env",
            url: "url",
            headers: "headers",
        },
        // agy (`agy mcp add`, 1.4 to 1.5.12): `serverUrl`, and no SSE.
        McpFormat::Agy => &Shape {
            syntax: Syntax::Json,
            container: &["mcpServers"],
            also: &[],
            reserved: &[],
            transports: STDIO_HTTP,
            kind: None,
            command_list: false,
            env: "env",
            url: "serverUrl",
            headers: "headers",
        },
    }
}

/// A server's fields between a file's syntax and a [`Server`].
enum Field {
    Text(String),
    List(Vec<String>),
    Map(Vec<(String, String)>),
}

/// Why a server's entry cannot be written with the keys handed in.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum KeyRefusal {
    /// The server names a variable or header that has no key.
    Missing(String),
    /// The key starts with `!`, which Pi and omp run as a shell command
    /// in place of the value, so no agent's file gets one.
    Runs(String),
}

fn fields(
    shape: &Shape,
    server: &Server,
    keys: &Keys,
) -> Result<Vec<(&'static str, Field)>, KeyRefusal> {
    let values = |names: &[String]| -> Result<Vec<(String, String)>, KeyRefusal> {
        names
            .iter()
            .map(|name| match keys.get(name) {
                None => Err(KeyRefusal::Missing(name.clone())),
                Some(value) if value.expose().starts_with('!') => {
                    Err(KeyRefusal::Runs(name.clone()))
                }
                Some(value) => Ok((name.clone(), value.expose().to_owned())),
            })
            .collect()
    };
    let mut fields = Vec::new();
    let kind = |pick: fn(&Kinds) -> Option<&'static str>| {
        let kinds = shape.kind.as_ref()?;
        Some((kinds.key, Field::Text(pick(kinds)?.to_owned())))
    };
    match &server.reach {
        Reach::Local { command, args, env } => {
            fields.extend(kind(|kinds| kinds.local));
            if shape.command_list {
                let mut words = vec![command.clone()];
                words.extend(args.iter().cloned());
                fields.push(("command", Field::List(words)));
            } else {
                fields.push(("command", Field::Text(command.clone())));
                if !args.is_empty() {
                    fields.push(("args", Field::List(args.clone())));
                }
            }
            if !env.is_empty() {
                fields.push((shape.env, Field::Map(values(env)?)));
            }
        }
        Reach::Remote {
            transport,
            url,
            headers,
        } => {
            fields.extend(match transport {
                Remote::Http => kind(|kinds| kinds.http),
                Remote::Sse => kind(|kinds| Some(kinds.sse)),
            });
            fields.push((shape.url, Field::Text(url.clone())));
            if !headers.is_empty() {
                fields.push((shape.headers, Field::Map(values(headers)?)));
            }
        }
    }
    Ok(fields)
}

/// What an entry's fields say: the server and its keys, or `None` when it is
/// not a server Hide can read in this shape. `read` answers a field the
/// entry holds as `Some(None)` when its value is not of a type the field
/// takes (a number among the arguments, an `env` value that is no string):
/// the entry is then unreadable, never read as if the field were absent,
/// which would drop it from a copy.
fn server_of(
    shape: &Shape,
    name: &str,
    read: &dyn Fn(&str) -> Option<Option<Field>>,
) -> Option<(Server, Keys)> {
    let read_fields = shape
        .kind
        .as_ref()
        .map(|kinds| kinds.key)
        .into_iter()
        .chain(["command", "args", shape.env, shape.url, shape.headers])
        .filter(|key| reads(shape, key));
    for key in read_fields {
        if matches!(read(key), Some(None)) {
            return None;
        }
    }
    let get = |key: &str| read(key).flatten();
    let text = |key: &str| match get(key) {
        Some(Field::Text(text)) => Some(text),
        _ => None,
    };
    let map = |key: &str| -> Option<Vec<(String, String)>> {
        match get(key) {
            None => Some(Vec::new()),
            Some(Field::Map(map)) => Some(map),
            Some(_) => None,
        }
    };
    let mut keys = Keys::new();
    let mut names = |pairs: Vec<(String, String)>| -> Vec<String> {
        pairs
            .into_iter()
            .map(|(key, value)| {
                keys.insert(key.clone(), Secret::new(value));
                key
            })
            .collect()
    };
    let kind = shape.kind.as_ref().map(|kinds| (kinds, text(kinds.key)));
    // A server is remote by its kind where the shape names one, and by its
    // address where it does not.
    let remote = match &kind {
        Some((kinds, Some(named))) if Some(named.as_str()) == kinds.local || named == "stdio" => {
            false
        }
        Some((kinds, Some(named)))
            if Some(named.as_str()) == kinds.http
                || *named == kinds.sse
                || named == "http"
                || named == "streamable-http" =>
        {
            true
        }
        Some((_, Some(_))) => return None,
        Some((_, None)) | None => get(shape.url).is_some(),
    };
    let reach = if remote {
        let url = text(shape.url)?;
        let transport = match &kind {
            Some((kinds, Some(named))) if *named == kinds.sse && Some(kinds.sse) != kinds.http => {
                Remote::Sse
            }
            _ => Remote::Http,
        };
        Reach::Remote {
            transport,
            url,
            headers: names(map(shape.headers)?),
        }
    } else {
        let (command, args) = if shape.command_list {
            match get("command")? {
                Field::List(mut words) if !words.is_empty() => {
                    let command = words.remove(0);
                    (command, words)
                }
                _ => return None,
            }
        } else {
            let args = match get("args") {
                None => Vec::new(),
                Some(Field::List(args)) => args,
                Some(_) => return None,
            };
            (text("command")?, args)
        };
        Reach::Local {
            command,
            args,
            env: names(map(shape.env)?),
        }
    };
    Some((
        Server {
            name: name.to_owned(),
            reach,
        },
        keys,
    ))
}

/// Whether the shape's reader reads the field `key` of an entry.
fn reads(shape: &Shape, key: &str) -> bool {
    shape.kind.as_ref().is_some_and(|kinds| kinds.key == key)
        || key == "command"
        || (!shape.command_list && key == "args")
        || [shape.env, shape.url, shape.headers].contains(&key)
}

/// Whether `entry` holds only fields the reader reads ([`super::Found::whole`]).
pub(super) fn json_whole(shape: &Shape, entry: &Value) -> bool {
    entry
        .as_object()
        .is_some_and(|entry| entry.keys().all(|key| reads(shape, key)))
}

/// [`json_whole`] for a TOML entry.
pub(super) fn toml_whole(shape: &Shape, entry: &Item) -> bool {
    entry
        .as_table_like()
        .is_some_and(|entry| entry.iter().all(|(key, _)| reads(shape, key)))
}

pub(super) fn json_entry(shape: &Shape, server: &Server, keys: &Keys) -> Result<Value, KeyRefusal> {
    let mut entry = Map::new();
    for (key, field) in fields(shape, server, keys)? {
        let value = match field {
            Field::Text(text) => Value::String(text),
            Field::List(list) => Value::Array(list.into_iter().map(Value::String).collect()),
            Field::Map(map) => Value::Object(
                map.into_iter()
                    .map(|(key, value)| (key, Value::String(value)))
                    .collect(),
            ),
        };
        entry.insert(key.to_owned(), value);
    }
    Ok(Value::Object(entry))
}

pub(super) fn json_server(shape: &Shape, name: &str, entry: &Value) -> Option<(Server, Keys)> {
    let entry = entry.as_object()?;
    server_of(shape, name, &|key| {
        Some(match entry.get(key)? {
            Value::String(text) => Some(Field::Text(text.clone())),
            Value::Array(list) => list
                .iter()
                .map(|word| word.as_str().map(str::to_owned))
                .collect::<Option<_>>()
                .map(Field::List),
            Value::Object(map) => map
                .iter()
                .map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
                .collect::<Option<_>>()
                .map(Field::Map),
            _ => None,
        })
    })
}

pub(super) fn toml_entry(shape: &Shape, server: &Server, keys: &Keys) -> Result<Table, KeyRefusal> {
    let mut entry = Table::new();
    for (key, field) in fields(shape, server, keys)? {
        let value = match field {
            Field::Text(text) => toml_edit::value(text),
            Field::List(list) => toml_edit::value(list.into_iter().collect::<Array>()),
            Field::Map(map) => {
                let mut table = InlineTable::new();
                for (key, value) in map {
                    table.insert(&key, value.into());
                }
                toml_edit::value(table)
            }
        };
        entry.insert(key, value);
    }
    Ok(entry)
}

pub(super) fn toml_server(shape: &Shape, name: &str, entry: &Item) -> Option<(Server, Keys)> {
    let entry = entry.as_table_like()?;
    server_of(shape, name, &|key| {
        let item = entry.get(key)?;
        if let Some(text) = item.as_str() {
            return Some(Some(Field::Text(text.to_owned())));
        }
        if let Some(list) = item.as_array() {
            return Some(
                list.iter()
                    .map(|word| word.as_str().map(str::to_owned))
                    .collect::<Option<_>>()
                    .map(Field::List),
            );
        }
        Some(item.as_table_like().and_then(|map| {
            map.iter()
                .map(|(key, value)| Some((key.to_owned(), value.as_str()?.to_owned())))
                .collect::<Option<_>>()
                .map(Field::Map)
        }))
    })
}
