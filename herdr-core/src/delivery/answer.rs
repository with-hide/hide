//! The answers `hide agent`, `hide request`, `hide inbox` and `hide watch`
//! return, as types, so the CLI's exported contract describes each answer
//! from the type that builds it (`answer_schemas`) and a changed field
//! changes the contract.

use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Value, json};

use super::ledger::Letter;
use super::mailbox::Intake;
use super::watch::Watch;

/// One registered agent, as `hide agent show`, `register`, `end` and `spawn`
/// answer it.
#[derive(Serialize, JsonSchema)]
pub struct AgentView {
    /// Naming has its own receipt; it never changes successful spawn identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub names: Option<crate::coordination::naming::Names>,
    pub id: String,
    pub name: String,
    pub machine: String,
    #[serde(rename = "hostScope")]
    pub host_scope: String,
    pub session: String,
    pub instance: String,
    pub pane: String,
    pub parent: Option<String>,
    /// The spawner, independently of responsibility; null for ordinary roots.
    pub origin: Option<String>,
    pub project: Option<String>,
    pub runtime: Runtime,
    pub connection: Connection,
    pub registered: bool,
    /// The watch on this agent, if one is running.
    pub watch: Option<Watch>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Runtime {
    Running,
    Ended,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Connection {
    Connected,
    Disconnected,
}

/// `hide agent list`.
#[derive(Serialize, JsonSchema)]
pub struct AgentList {
    pub items: Vec<AgentView>,
}

/// `hide ps`: every live pane, where it sits and the agent it holds, and
/// the registered agents no live pane holds (`runtime::ps`).
#[derive(Debug, Serialize, JsonSchema)]
pub struct Ps {
    /// In the order the sidebar lists them: by project, checkout and tab.
    pub panes: Vec<PsPane>,
    /// Registered agents with no pane on screen: asleep, on a device that
    /// is not connected, or whose pane Hide no longer sees; with `--all`
    /// the ended ones too.
    pub agents: Vec<PsAgent>,
    /// The mission record could not be read, so no row names its mission.
    pub missions_unreadable: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PsPane {
    /// The device id the pane runs on.
    pub machine: String,
    /// The name another device was registered under, or its id when it has
    /// none; null on this machine.
    pub machine_label: Option<String>,
    pub project: String,
    pub project_path: String,
    /// The checkout's branch, or its folder's name.
    pub checkout: String,
    pub checkout_path: String,
    pub worktree: bool,
    pub tab: Option<String>,
    pub tab_id: Option<String>,
    /// Herdr's pane id on `machine`.
    pub pane: String,
    /// The pane's own title, for a pane that holds no agent.
    pub title: Option<String>,
    /// The pane `hide ps` was run from.
    pub you: bool,
    pub agent: Option<PsAgent>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PsAgent {
    /// The registration id; null for an agent Hide did not register.
    pub id: Option<String>,
    pub name: Option<String>,
    pub kind: String,
    pub state: PsState,
    /// The title every surface calls the agent by.
    pub label: Option<String>,
    /// The row's second line: what it asks, did or waits on.
    pub line: Option<String>,
    pub machine: String,
    pub pane: String,
    /// The live agent this one was delegated by, as the screen nests it.
    pub parent_machine: Option<String>,
    pub parent_pane: Option<String>,
    /// The registration id of the parent the registration names.
    pub parent: Option<String>,
    /// The name of the agent watching this one.
    pub watched_by: Option<String>,
    pub mission: Option<PsMission>,
    /// The project path the registration names.
    pub project: Option<String>,
}

/// The row's mark as every surface draws it (docs/status-model.md, The
/// mark), `disconnected` for a row on a device that does not answer, and
/// `ended` for an ended registration.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PsState {
    Turn,
    /// The operator has a View this agent opened to look at.
    Look,
    Going,
    Waiting,
    Done,
    Unread,
    Quiet,
    Unknown,
    Asleep,
    Disconnected,
    Ended,
}

impl From<crate::agent_state::RowMark> for PsState {
    fn from(mark: crate::agent_state::RowMark) -> Self {
        use crate::agent_state::RowMark;
        match mark {
            RowMark::Turn => Self::Turn,
            RowMark::Look => Self::Look,
            RowMark::Going => Self::Going,
            RowMark::Waiting => Self::Waiting,
            RowMark::Done => Self::Done,
            RowMark::Unread => Self::Unread,
            RowMark::Quiet => Self::Quiet,
            RowMark::Unknown => Self::Unknown,
            RowMark::Asleep => Self::Asleep,
        }
    }
}

/// The open mission an agent conducts or works a node of.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct PsMission {
    pub id: String,
    pub title: String,
    /// `conductor`, or the key of the node the agent works.
    pub role: String,
}

/// `hide agent register --check` for a caller not yet registered.
#[derive(Serialize, JsonSchema)]
pub struct RegisterCheck {
    pub registered: bool,
    pub name: String,
    pub pane: String,
}

/// One letter of `hide inbox`.
#[derive(Serialize, JsonSchema)]
pub struct InboxEntry {
    #[serde(flatten)]
    pub letter: Letter,
    /// The command that acknowledges the letter, for an agent with no
    /// prompt hook to confirm it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ack_command: Option<String>,
}

/// `hide inbox --confirm`.
#[derive(Serialize, JsonSchema)]
pub struct Confirmed {
    pub confirmed: Vec<String>,
}

/// `hide watch stop`.
#[derive(Serialize, JsonSchema)]
pub struct Stopped {
    pub stopped: String,
}

/// The JSON Schema of every answer, by the name the CLI contract gives it.
///
/// An answer is written, never read back, so every field it serializes is
/// present, a `None` as `null`; the schemas say so (`present`), where
/// schemars, describing what deserializing would accept, would leave an
/// optional or defaulted field out of `required`.
pub fn answer_schemas() -> Value {
    let mut schemas = json!({
        "agent": schemars::schema_for!(AgentView),
        "agent_list": schemars::schema_for!(AgentList),
        "agent_register_check": schemars::schema_for!(RegisterCheck),
        "ps": schemars::schema_for!(Ps),
        "letter": schemars::schema_for!(Letter),
        "inbox": schemars::schema_for!(Vec<InboxEntry>),
        "intake": schemars::schema_for!(Intake),
        "confirmed": schemars::schema_for!(Confirmed),
        "watch": schemars::schema_for!(Watch),
        "watch_list": schemars::schema_for!(Vec<Watch>),
        "stopped": schemars::schema_for!(Stopped),
    });
    present(&mut schemas);
    schemas
}

/// The fields an answer leaves out when they have no value
/// (`skip_serializing_if`).
const OMITTED: &[&str] = &["ack_command", "names"];

/// Marks every property of every object schema in `schema` required, but
/// those in `OMITTED`.
fn present(schema: &mut Value) {
    match schema {
        Value::Object(map) => {
            if let Some(Value::Object(properties)) = map.get("properties") {
                let required: Vec<Value> = properties
                    .keys()
                    .filter(|key| !OMITTED.contains(&key.as_str()))
                    .map(|key| Value::String(key.clone()))
                    .collect();
                map.insert("required".to_owned(), Value::Array(required));
            }
            map.values_mut().for_each(present);
        }
        Value::Array(items) => items.iter_mut().for_each(present),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A field the answer always writes is required in its schema, a `null`
    /// one included, and the one it can leave out is not.
    #[test]
    fn every_written_field_is_required() {
        let schemas = answer_schemas();
        let agent = schemas["agent"]["required"].as_array().unwrap();
        assert!(!agent.contains(&json!("names")));
        assert!(
            !schemas["agent_list"]["definitions"]["AgentView"]["required"]
                .as_array()
                .unwrap()
                .contains(&json!("names"))
        );
        for field in [
            "parent",
            "origin",
            "project",
            "watch",
            "runtime",
            "connection",
            "hostScope",
        ] {
            assert!(agent.contains(&json!(field)), "agent {field}");
        }
        let watch = &schemas["agent"]["definitions"]["Watch"]["required"];
        assert!(watch.as_array().unwrap().contains(&json!("generation")));
        let entry = &schemas["inbox"]["items"]["required"];
        let entry = match entry {
            Value::Array(entry) => entry.clone(),
            _ => schemas["inbox"]["definitions"]["InboxEntry"]["required"]
                .as_array()
                .unwrap()
                .clone(),
        };
        assert!(entry.contains(&json!("reply_to")));
        assert!(!entry.contains(&json!("ack_command")));
        assert_eq!(
            schemas["agent"]["definitions"]["Runtime"]["enum"],
            json!(["running", "ended"])
        );
    }

    #[test]
    fn origin_is_present_and_nullable_in_single_and_list_answers() {
        let schemas = answer_schemas();
        for schema in [
            &schemas["agent"],
            &schemas["agent_list"]["definitions"]["AgentView"],
        ] {
            assert!(
                schema["required"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("origin"))
            );
            assert_eq!(
                schema["properties"]["origin"]["type"],
                json!(["string", "null"])
            );
        }
        for (parent, origin) in [
            (None, None),
            (Some("agent-1"), Some("agent-1")),
            (None, Some("agent-1")),
        ] {
            let agent = AgentView {
                names: None,
                id: "agent-2".into(),
                name: "worker".into(),
                machine: "local".into(),
                host_scope: "fixture".into(),
                session: "native-worker".into(),
                instance: "terminal-2".into(),
                pane: "worker".into(),
                parent: parent.map(str::to_owned),
                origin: origin.map(str::to_owned),
                project: None,
                runtime: Runtime::Running,
                connection: Connection::Connected,
                registered: true,
                watch: None,
            };
            let single = serde_json::to_value(&agent).unwrap();
            let list = serde_json::to_value(AgentList { items: vec![agent] }).unwrap();
            for answer in [&single, &list["items"][0]] {
                assert!(!answer.as_object().unwrap().contains_key("names"));
                assert!(answer.as_object().unwrap().contains_key("origin"));
                assert_eq!(answer["origin"], json!(origin));
                assert_eq!(answer["parent"], json!(parent));
            }
        }
    }
}
