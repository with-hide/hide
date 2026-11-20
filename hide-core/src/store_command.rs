//! `hide skill` / `hide mcp`: a machine's store of skills and MCP items,
//! changed the way Settings › 스킬·MCP changes it (PRD skill-mcp-manager
//! B31-B33, D-19, D-29). The runtime plans the command and queues its store
//! work (`runtime::store_cli`); the daemon waits on its answer off the
//! owner thread with [`Prepared::run`].
//!
//! An install, an update and an MCP added or changed wait for the operator's
//! confirm on their machine's 내 차례, as the asking agent's person's turn,
//! and run only on [설치]: no option skips it. A key's value is never an
//! argument; the operator types it on the confirm.

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Mutex, Weak};
use std::time::Duration;

use hide_agent_hooks::mcp::Reach;
use hide_kit::store::{
    CellState, ChangeOutcome, Column, ItemKind, ItemReport, SourceSpec, Version,
};
use schemars::JsonSchema;
use schemars::r#gen::SchemaGenerator;
use schemars::schema::Schema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The store refused the change; the detail is the store's cause and its
/// sentence.
pub const REFUSED: &str = "store_refused";
/// The operator pressed [취소] on the confirm.
pub const DECLINED: &str = "store_declined";
/// The operator did not answer the confirm in time; it was taken down.
pub const UNANSWERED: &str = "store_unanswered";
/// The machine named has no store report yet, or no item of that name.
pub const NOT_FOUND: &str = "store_not_found";
/// The store could not take another request now.
pub const BUSY: &str = "store_busy";
/// A word of the command the store cannot take (an agent, a name).
pub const INVALID: &str = "store_invalid";
/// The machine's store could not run the pass (its link went, its kit does
/// not run there).
pub const UNAVAILABLE: &str = "store_unavailable";

/// The refusals that carry a detail (`coordination::split_refusal`).
pub const DETAILED: &[&str] = &[REFUSED, NOT_FOUND, BUSY, INVALID, UNAVAILABLE];

/// How long a switch or a delete waits for its pass: a pass on a device may
/// fetch nothing, but its link may be slow to come up.
pub const PASS_WAIT: Duration = Duration::from_secs(200);
/// How long an install or an update waits for the operator, its source's
/// read and its pass included.
pub const CONFIRM_WAIT: Duration = Duration::from_secs(30 * 60);

/// The longest word a command carries.
const WORD_LIMIT: usize = 4096;

/// The most skills one `hide skill add` names (`--name`).
pub const NAMES_LIMIT: usize = 64;

/// `hide skill list` / `hide mcp list`: the machine's items of one kind,
/// each with its version and one cell per column.
#[derive(Serialize, JsonSchema)]
pub struct StoreList {
    pub machine: String,
    #[schemars(schema_with = "item_kind_schema")]
    pub kind: ItemKind,
    /// The kind's columns, each with the agents its switch reaches.
    #[schemars(schema_with = "columns_schema")]
    pub columns: Vec<Column>,
    pub items: Vec<StoreLine>,
}

/// One item as `list` answers it: no file list, the cells by column.
#[derive(Serialize, JsonSchema)]
pub struct StoreLine {
    pub id: String,
    pub name: String,
    #[schemars(schema_with = "version_schema")]
    pub version: Version,
    /// The newer version a check found, null with none.
    #[schemars(schema_with = "update_schema")]
    pub update: Option<Version>,
    pub on: bool,
    pub cells: Vec<StoreLineCell>,
}

#[derive(Serialize, JsonSchema)]
pub struct StoreLineCell {
    pub column: String,
    #[schemars(schema_with = "cell_state_schema")]
    pub state: CellState,
}

/// `hide skill show` / `hide mcp show`: the item as the machine's store
/// reports it.
#[derive(Serialize, JsonSchema)]
pub struct StoreShown {
    pub machine: String,
    #[schemars(schema_with = "item_schema")]
    pub item: ItemReport,
}

/// A change that ran (`add`, `update`, `on`, `off`, `remove`): the item it
/// was about, null for an install of a source's skills, and each change's
/// outcome, a refused one with its cause.
#[derive(Serialize, JsonSchema)]
pub struct StoreChanged {
    pub machine: String,
    pub item: Option<String>,
    #[schemars(schema_with = "outcomes_schema")]
    pub outcomes: Vec<ChangeOutcome>,
}

impl From<&ItemReport> for StoreLine {
    fn from(item: &ItemReport) -> Self {
        Self {
            id: item.id.clone(),
            name: item.name.clone(),
            version: item.version.clone(),
            update: item.update.clone(),
            on: item.on,
            cells: item
                .cells
                .iter()
                .map(|cell| StoreLineCell {
                    column: cell.column.clone(),
                    state: cell.state,
                })
                .collect(),
        }
    }
}

/// The schemas of the store's answers, for the CLI contract.
pub fn answer_schemas() -> Vec<(&'static str, Value)> {
    let schema = |root: schemars::schema::RootSchema| {
        serde_json::to_value(root).expect("a schema serializes")
    };
    vec![
        ("store_list", schema(schemars::schema_for!(StoreList))),
        ("store_item", schema(schemars::schema_for!(StoreShown))),
        ("store_changed", schema(schemars::schema_for!(StoreChanged))),
    ]
}

// The kit's store types carry no schema of their own: these describe what
// they serialize to, their string values named by the wire enum list that
// holds them (`contracts/snapshot-wire-enums.json`).

fn described(schema: Value) -> Schema {
    serde_json::from_value(schema).expect("a schema literal")
}

fn wire_enum(name: &str) -> Value {
    json!({"type": "string", "description": format!("one of {name} in contracts/snapshot-wire-enums.json")})
}

fn version() -> Value {
    json!({
        "type": "object",
        "description": "the version beside the name: a tag, a commit, or what the source is (folder, copy, remote, npm_latest, pinned, unpinned)",
        "properties": {"kind": {"type": "string"}, "value": {"type": "string"}},
        "required": ["kind"],
    })
}

fn item_kind_schema(_: &mut SchemaGenerator) -> Schema {
    described(wire_enum("store_item_kind"))
}

fn cell_state_schema(_: &mut SchemaGenerator) -> Schema {
    described(wire_enum("store_cell_state"))
}

fn version_schema(_: &mut SchemaGenerator) -> Schema {
    described(version())
}

fn update_schema(_: &mut SchemaGenerator) -> Schema {
    described(json!({"anyOf": [version(), {"type": "null"}]}))
}

fn columns_schema(_: &mut SchemaGenerator) -> Schema {
    described(json!({
        "type": "array",
        "items": {
            "type": "object",
            "properties": {
                "id": {"type": "string"},
                "agents": {"type": "array", "items": {"type": "string"}},
                "also": {"type": "array", "items": {"type": "string"}},
            },
            "required": ["id", "agents"],
        },
    }))
}

fn item_schema(_: &mut SchemaGenerator) -> Schema {
    described(json!({
        "type": "object",
        "description": "the item as KitSnapshot.store reports it",
        "properties": {
            "id": {"type": "string"},
            "kind": wire_enum("store_item_kind"),
            "name": {"type": "string"},
            "source": {"type": "object"},
            "version": version(),
            "update": version(),
            "on": {"type": "boolean"},
            "off_for": {"type": "array", "items": {"type": "string"}},
            "folder_missing": {"type": "boolean"},
            "description": {"type": "string"},
            "files": {"type": "array", "items": {"type": "object"}},
            "server": {"type": "object", "description": "an MCP server: its name and reach, with the names of its keys and never a value"},
            "cells": {"type": "array", "items": {
                "type": "object",
                "properties": {
                    "column": {"type": "string"},
                    "state": wire_enum("store_cell_state"),
                    "reason": {"type": "string"},
                    "changed_at_unix_ms": {"type": "integer"},
                },
                "required": ["column", "state"],
            }},
        },
        "required": ["id", "kind", "name", "source", "version", "on", "cells"],
    }))
}

fn outcomes_schema(_: &mut SchemaGenerator) -> Schema {
    described(json!({
        "type": "array",
        "items": {
            "type": "object",
            "properties": {
                "change": {
                    "type": "object",
                    "description": "the change as it ran, a key's value emptied",
                    "properties": {"op": {"type": "string"}},
                    "required": ["op"],
                },
                "refused": {
                    "type": "object",
                    "properties": {"cause": wire_enum("store_refusal_cause"), "reason": {"type": "string"}},
                    "required": ["cause", "reason"],
                },
            },
            "required": ["change"],
        },
    }))
}

/// One `hide skill` / `hide mcp` command.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "verb", rename_all = "snake_case")]
pub enum StoreCommand {
    List {
        kind: ItemKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine: Option<String>,
    },
    Show {
        kind: ItemKind,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine: Option<String>,
    },
    /// A skill from a GitHub address or a folder of the machine; `names`
    /// picks among the skills it holds, every new one when empty.
    Add {
        source: SourceSpec,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        names: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine: Option<String>,
    },
    /// An MCP server: how it is reached and the names of its keys, never a
    /// value.
    AddMcp {
        name: String,
        reach: Reach,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine: Option<String>,
    },
    /// A GitHub skill's newer version, after the operator saw what changes.
    Update {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine: Option<String>,
    },
    /// An MCP server's new command or address.
    UpdateMcp {
        name: String,
        reach: Reach,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine: Option<String>,
    },
    /// The item's own switch, or one agent's.
    Set {
        kind: ItemKind,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent: Option<String>,
        on: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine: Option<String>,
    },
    /// Out of every agent and into the undo list (B18).
    Remove {
        kind: ItemKind,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine: Option<String>,
    },
}

impl StoreCommand {
    pub fn machine(&self) -> Option<&str> {
        match self {
            Self::List { machine, .. }
            | Self::Show { machine, .. }
            | Self::Add { machine, .. }
            | Self::AddMcp { machine, .. }
            | Self::Update { machine, .. }
            | Self::UpdateMcp { machine, .. }
            | Self::Set { machine, .. }
            | Self::Remove { machine, .. } => machine.as_deref(),
        }
    }

    /// Every word is short and holds no NUL; a key value cannot be among
    /// them, since the command carries only key names.
    pub fn valid(&self) -> bool {
        let text = |text: &str| text.len() <= WORD_LIMIT && !text.contains('\0');
        let word = |word: &str| !word.is_empty() && text(word);
        let reach = |reach: &Reach| match reach {
            Reach::Local { command, args, env } => {
                word(command)
                    && args.len() <= hide_agent_hooks::mcp::MAX_ARGS
                    && args.iter().all(|arg| text(arg))
                    && env.len() <= hide_agent_hooks::mcp::MAX_KEY_NAMES
                    && env.iter().all(|name| word(name))
            }
            Reach::Remote { url, headers, .. } => {
                word(url)
                    && headers.len() <= hide_agent_hooks::mcp::MAX_KEY_NAMES
                    && headers.iter().all(|name| word(name))
            }
        };
        self.machine().is_none_or(word)
            && match self {
                Self::List { .. } => true,
                Self::Show { name, .. } | Self::Update { name, .. } | Self::Remove { name, .. } => {
                    word(name)
                }
                Self::Add { source, names, .. } => {
                    let source = match source {
                        SourceSpec::Git { url } => url,
                        SourceSpec::Folder { path } => path,
                    };
                    word(source)
                        && names.len() <= NAMES_LIMIT
                        && names.iter().all(|name| word(name))
                }
                Self::AddMcp {
                    name, reach: how, ..
                }
                | Self::UpdateMcp {
                    name, reach: how, ..
                } => word(name) && reach(how),
                Self::Set { name, agent, .. } => word(name) && agent.as_deref().is_none_or(word),
            }
    }

    /// Whether the command waits for the operator's confirm.
    pub fn confirms(&self) -> bool {
        matches!(
            self,
            Self::Add { .. } | Self::AddMcp { .. } | Self::Update { .. } | Self::UpdateMcp { .. }
        )
    }
}

/// What the daemon prints next for a refusal `code`.
pub fn next_action(code: &str) -> &'static str {
    match code {
        REFUSED => "The machine's store refused the change; the detail says why",
        DECLINED => "The operator declined it; ask them before trying again",
        UNANSWERED => {
            "The operator did not answer the confirm; ask them, then run the command again"
        }
        NOT_FOUND => "hide skill list or hide mcp list shows what the machine holds",
        BUSY => "The store has many requests waiting; try again in a moment",
        INVALID => "hide skill help or hide mcp help shows the words each command takes",
        UNAVAILABLE => "Settings › Devices shows whether the machine is connected",
        _ => "hide skill help or hide mcp help shows the commands",
    }
}

/// A command planned under the lock: answered already, or waiting on the
/// store and, for an install or an update, the operator.
pub struct Prepared {
    pub(crate) answer: Waiting,
    pub(crate) runtime: Weak<Mutex<crate::runtime::Runtime>>,
    pub(crate) notifier: crate::handle::ChangeNotifier,
}

pub(crate) enum Waiting {
    Ready(Result<Value, String>),
    On {
        run: String,
        receiver: Receiver<Result<Value, String>>,
        wait: Duration,
    },
}

impl Prepared {
    /// Waits, with no lock held, for the store's answer and, where the
    /// command asks one, the operator's. A wait that runs out takes the
    /// confirm down, so nothing waits on an answer no one reads.
    pub fn run(self) -> Result<Value, String> {
        let (run, receiver, wait) = match self.answer {
            Waiting::Ready(answer) => return answer,
            Waiting::On {
                run,
                receiver,
                wait,
            } => (run, receiver, wait),
        };
        match receiver.recv_timeout(wait) {
            Ok(answer) => answer,
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                if let Some(runtime) = self.runtime.upgrade() {
                    let changed = runtime
                        .lock()
                        .map(|mut runtime| runtime.abandon_store_run(&run))
                        .unwrap_or(false);
                    if changed {
                        self.notifier.notify();
                    }
                }
                Err(UNANSWERED.to_owned())
            }
        }
    }
}
