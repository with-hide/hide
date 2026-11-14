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
    /// The name of the external tool whose connection started this agent
    /// (PRD hide-external-control B31); absent for every other start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_tool: Option<String>,
    pub project: Option<String>,
    /// The profile the agent runs with, named after its agent (`Claude ·
    /// work`); null for its agent's built-in default (PRD agent-profiles
    /// B17).
    pub profile: Option<String>,
    pub runtime: Runtime,
    pub connection: Connection,
    pub registered: bool,
    /// The watch on this agent, if one is running.
    pub watch: Option<Watch>,
}

/// `hide agent end`: the registration as it ended, what became of the pane
/// its agent ran in, and of the conversation that ran there.
#[derive(Serialize, JsonSchema)]
pub struct AgentEnd {
    #[serde(flatten)]
    pub agent: AgentView,
    pub pane: PaneEnd,
    pub conversation: ConversationEnd,
}

/// The pane of an ended registration.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaneEnd {
    /// Closed: its holder, shell, agent and everything under them ended.
    Closed,
    /// Left open, as `--keep-pane` asked.
    Kept,
    /// No pane of the registration runs: it closed before, or its id now
    /// names another terminal.
    Gone,
    /// Another conversation runs in the pane now, so it was left open.
    OtherAgent,
    /// Left open: the registration is from before Hide recorded its pane's
    /// terminal, and no agent shows in the pane, so nothing proves the pane
    /// is still the registration's.
    Left,
}

/// The conversation of an ended registration.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConversationEnd {
    /// Kept under 보관됨, where 이어가기 resumes it.
    Archived,
    /// Not kept: the archive holds as many sessions as it keeps, and none of
    /// them is at rest to make room.
    ArchiveFull,
    /// Nothing was archived: the pane stayed, was gone, showed no
    /// conversation Hide knows, or Hide could not form the record (its
    /// reason is in the diagnostic log).
    None,
}

/// `hide agent show`: the registration and what its agent is doing now.
#[derive(Serialize, JsonSchema)]
pub struct AgentShow {
    #[serde(flatten)]
    pub agent: AgentView,
    pub state: AgentState,
}

/// What an agent is doing now, as the sidebar, `hide agent list` and `hide
/// ps` read it from the core, for a parent deciding what to do about a child.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
pub struct AgentState {
    pub activity: Activity,
    /// Where the agent is in its life, as `runtime` says; null for a spawn
    /// whose agent never registered.
    pub lifecycle: Option<Runtime>,
    /// When the core last read the agent's state; the state holds until a
    /// later read changes it. Absent when the core has never read it.
    pub observed_at_unix_ms: Option<u64>,
    /// What the agent waits on: null only when the core knows it waits on
    /// nothing, `cause: unknown` when it cannot tell.
    pub waiting: Option<Waiting>,
    /// Input Hide typed into the agent that the agent has not recorded.
    pub pending_inputs: Vec<PendingInput>,
}

/// Whether the agent is running a turn. A state the core cannot read now,
/// a stale read or a device that does not answer, is `unknown`, never
/// `stopped`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Activity {
    Working,
    Stopped,
    Unknown,
}

/// What an agent waits on, by the core's own cause.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
pub struct Waiting {
    pub cause: WaitCause,
    /// Who can answer: `operator`, or the registration id of the agent
    /// that should (a delegated child's parent). Absent when unknown.
    pub on: Option<String>,
    pub since_unix_ms: Option<u64>,
    /// The letter the wait is about.
    pub letter_id: Option<String>,
    /// The spawn whose brief the wait is about.
    pub spawn_id: Option<String>,
    /// A short line saying what is asked, when the core has one. Copy for a
    /// reader, never a reason to act: `cause` is.
    pub text: Option<String>,
    /// For `human_turn`: the declaration, its kind, place and when it was
    /// made (PRD ask-human B7). The device is the agent's `machine`.
    pub human: Option<crate::coordination::human_turn::HumanTurn>,
}

/// The causes an agent waits by: its own demand (docs/status-model.md, Where
/// each axis comes from), a question the core laid on its row, or one of the
/// escalation causes (with-hide/hide-private docs/task-identity.md, Delegated escalation).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WaitCause {
    Unknown,
    Question,
    Approval,
    Blocked,
    PlanReview,
    MissionDecision,
    MissionAttention,
    InputUnconfirmed,
    /// The agent declared a step only a person can take (`hide agent
    /// ask-human`), its own or, for a delegated child, raised to the operator.
    HumanTurn,
    Draft,
    ParentBlocked,
    BellExhausted,
    Undelivered,
    ObserverUnconfirmed,
}

/// One input Hide typed that the agent has not recorded
/// (docs/delivery.md, Input an agent did not take).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
pub struct PendingInput {
    pub kind: crate::delivery::typed::InputKind,
    pub spawn_id: Option<String>,
    pub request_id: Option<String>,
    pub attempts: u8,
    pub max_attempts: u8,
    pub phase: InputPhase,
    pub typed_at_unix_ms: u64,
    /// When its parent or sender was told it is unconfirmed.
    pub noticed_at_unix_ms: Option<u64>,
}

/// Where one typed input stands: typed and its record not yet owed or
/// arrived, owed and missing, or missing with no typing left.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InputPhase {
    AwaitingRecord,
    Unconfirmed,
    Exhausted,
}

/// `hide agent show <spawn id or intent>` for a spawn whose agent has not
/// registered: the spawn and its pane's state.
#[derive(Serialize, JsonSchema)]
pub struct SpawnShow {
    pub spawn: SpawnRef,
    pub state: AgentState,
}

/// A spawn, by its stable id, and the pane it made.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
pub struct SpawnRef {
    pub id: String,
    pub intent: String,
    pub name: String,
    pub kind: String,
    /// The device the child is on when it is not the caller's own.
    pub machine: Option<String>,
    /// The pane as `hide pane send` takes it; absent until the spawn made
    /// one.
    pub pane: Option<String>,
    /// The child's registration id once it registered.
    pub child: Option<String>,
}

// Where the registration's agent is in its life, which every answer reads
// from one decision (`coordination::life`): running, put to sleep by Hide
// (waking, or not reopened, included), closed while Reopen closed can bring
// it back, or ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Runtime {
    Running,
    Asleep,
    Closed,
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
    /// The pane's id on `machine`.
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
    /// The name of the external tool whose connection started this agent;
    /// absent for every other start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_tool: Option<String>,
    /// The name of the agent watching this one.
    pub watched_by: Option<String>,
    pub mission: Option<PsMission>,
    /// The project path the registration names.
    pub project: Option<String>,
}

/// The row's mark as every surface draws it (docs/status-model.md, The
/// mark), `disconnected` for a row on a device that does not answer,
/// `closed` for a registration whose pane was closed for Reopen closed, and
/// `ended` for an ended registration.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PsState {
    Turn,
    Look,
    Going,
    Waiting,
    Done,
    Unread,
    Quiet,
    Unknown,
    Asleep,
    Closed,
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

/// `hide inbox`: the caller's letters as one bundle, as a handover gives
/// them (`docs/delivery.md`, One bundle per handover).
#[derive(Serialize, JsonSchema)]
pub struct Inbox {
    /// The letters shown in full, grouped by sender.
    pub letters: Vec<Letter>,
    /// The caller's own letters that went undelivered, returned to it: each
    /// names the recipient that did not get it, and `bell_failure` why its
    /// last bell failed when one did.
    pub returned: Vec<Letter>,
    /// Per sender, the older letters the bundle folded, oldest first; each
    /// opens with `hide request show <id>`.
    pub folded: Vec<Folded>,
    /// The one command that acknowledges exactly the pending letters this
    /// answer lists, folded ones included, by a token naming the bundle, for
    /// an agent with no prompt hook to confirm it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ack_command: Option<String>,
}

/// One sender's folded letters in a bundle.
#[derive(Serialize, JsonSchema)]
pub struct Folded {
    pub sender: String,
    pub sender_kind: String,
    pub ids: Vec<String>,
}

/// `hide inbox --ack`: the bundle it took and how many letters were in it.
#[derive(Serialize, JsonSchema)]
pub struct Acknowledged {
    pub bundle: String,
    pub acknowledged: usize,
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
        "agent_show": schemars::schema_for!(AgentShow),
        "agent_end": schemars::schema_for!(AgentEnd),
        "spawn_show": schemars::schema_for!(SpawnShow),
        "agent_list": schemars::schema_for!(AgentList),
        "agent_register_check": schemars::schema_for!(RegisterCheck),
        "ps": schemars::schema_for!(Ps),
        "letter": schemars::schema_for!(Letter),
        "inbox": schemars::schema_for!(Inbox),
        "acknowledged": schemars::schema_for!(Acknowledged),
        "intake": schemars::schema_for!(Intake),
        "confirmed": schemars::schema_for!(Confirmed),
        "watch": schemars::schema_for!(Watch),
        "watch_list": schemars::schema_for!(Vec<Watch>),
        "stopped": schemars::schema_for!(Stopped),
        "ask_human": schemars::schema_for!(crate::coordination::human_turn::AskHumanAnswer),
    });
    present(&mut schemas);
    schemas
}

/// The fields an answer leaves out when they have no value
/// (`skip_serializing_if`).
const OMITTED: &[&str] = &["ack_command", "bell_failure", "names"];

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
        let inbox = schemas["inbox"]["required"].as_array().unwrap();
        assert!(inbox.contains(&json!("folded")));
        assert!(!inbox.contains(&json!("ack_command")));
        let letter = &schemas["inbox"]["definitions"]["Letter"]["required"];
        assert!(letter.as_array().unwrap().contains(&json!("reply_to")));
        assert_eq!(
            schemas["agent"]["definitions"]["Runtime"]["enum"],
            json!(["running", "asleep", "closed", "ended"])
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
                profile: None,
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
                origin_tool: None,
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
                // Only a tool's start names a tool (B37).
                assert!(!answer.as_object().unwrap().contains_key("origin_tool"));
                assert!(answer.as_object().unwrap().contains_key("origin"));
                assert_eq!(answer["origin"], json!(origin));
                assert_eq!(answer["parent"], json!(parent));
            }
        }
    }
}
