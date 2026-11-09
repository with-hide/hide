//! What a phone sees of the missions (PRD mobile-companion-ui B28, B31-B36,
//! D-14): the record the desktop's Mission tool reads (`rest.missions`),
//! trimmed to the fields the shared mission rules and the phone's two screens
//! draw. It carries no path, repository, session or review text: only the
//! structs below can reach the wire, so a field the core adds to a mission
//! stays on the Mac until it is named here.
//!
//! Which group a mission stands in, what waits on the operator and the
//! progress cells are the web's own rules (`web/src/missionList.ts`,
//! `missions.ts`), run on the phone over this value; nothing here decides them.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{AgentKey, LIST_BUDGET, Projection, wire_len};
use crate::missions::Question;

/// How many open and how many closed missions one frame carries, the newest
/// first; the rest are counted in `omitted` and logged (engineering #15).
/// The frame's bytes are capped too (`LIST_BUDGET`): see `fit`.
pub const MAX_OPEN: usize = 24;
pub const MAX_CLOSED: usize = 24;
/// The longest title or name kept, in characters.
const TEXT_CHARS: usize = 200;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct PhoneMissions {
    pub items: Vec<PhoneMission>,
    /// The record could not be read at start: the phone says so instead of
    /// drawing a number, as the desktop does.
    pub failed: bool,
    /// Missions past the caps, which no frame carries.
    #[serde(skip_serializing_if = "is_zero")]
    pub omitted: usize,
}

impl PhoneMissions {
    /// The steps of finished missions the frame left out to stay within its
    /// budget, summed over the missions it carries.
    pub fn nodes_omitted(&self) -> usize {
        self.items.iter().map(|mission| mission.nodes_omitted).sum()
    }
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PhoneMission {
    pub id: String,
    pub title: String,
    pub device_id: String,
    /// The device's name for a mission away from the paired Mac; `None` on the Mac.
    pub device_label: Option<String>,
    pub status: String,
    pub decision: Option<String>,
    pub question: Option<Question>,
    pub answer: Option<PhoneAnswer>,
    pub answer_failed: Option<String>,
    pub conductor: PhoneConductor,
    /// The Conductor's letters only the operator can let through
    /// (`MissionView::conductor_letters`), which the shared 내 차례 rule reads.
    pub conductor_letters: Option<PhoneHeldLetters>,
    pub nodes: Vec<PhoneNode>,
    /// The steps a finished mission left behind on the Mac because the frame
    /// had no room for them; `nodes` and `landing_order` are empty then.
    #[serde(skip_serializing_if = "is_zero")]
    pub nodes_omitted: usize,
    pub landing_order: Vec<String>,
    pub created_at_unix_ms: u64,
    pub started_at_unix_ms: Option<u64>,
    pub closed_at_unix_ms: Option<u64>,
    /// The mission's device does not answer, so the values are the last ones.
    pub stale: bool,
}

/// How many letters wait for the Conductor, what the operator does about
/// them and since when (`delivery::held::HeldLetters`).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PhoneHeldLetters {
    pub count: u32,
    pub reason: String,
    pub since_unix_ms: u64,
}

/// Where the operator's answer is: `sending`, `waiting` or `unconfirmed`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PhoneAnswer {
    pub key: u64,
    pub text: String,
    #[serde(default)]
    pub option: Option<u8>,
    pub state: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PhoneConductor {
    pub name: String,
    pub agent_kind: String,
    pub present: bool,
    pub takes_answers: bool,
    /// The Conductor's row in the phone's list, when it has one, so `대화 ›` opens it.
    pub agent: Option<AgentKey>,
}

/// What waits on someone to look at a node (`AttentionView`).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PhoneAttention {
    pub reason: String,
    #[serde(default)]
    pub pr: Option<u32>,
    pub to: String,
    pub since_unix_ms: u64,
    pub on_row: bool,
    pub item: bool,
}

/// Where a node's work is on its way to main (`NodeLanding`), copied variant
/// by variant: a state or a field the core adds is not on the wire until it is
/// named here, and the contract test over the published record fails first.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PhoneLanding {
    Landed { proof: PhoneLandingProof },
    Left { action: PhoneLandingAction },
    Unread { action: PhoneLandingAction },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum PhoneLandingProof {
    Merged,
    InMain { commit: String },
    Outside,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PhoneLandingAction {
    Merge,
    Join,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PhoneNode {
    pub key: String,
    pub title: String,
    pub kind: String,
    pub depends_on: Vec<String>,
    pub status: String,
    pub blocked: Option<String>,
    pub delivery: Option<String>,
    pub pr: Option<u32>,
    pub pr_state: Option<String>,
    pub base_pr: Option<u32>,
    pub stage: Option<u8>,
    pub attention: Option<PhoneAttention>,
    /// Where the work is on its way to main.
    pub landing: Option<PhoneLanding>,
    pub agent_runtime: Option<String>,
    /// The agent kind of the node's worker.
    pub worker_agent: Option<String>,
    /// The worker's row in the phone's list, when it has one, so the step opens it (B36).
    pub agent: Option<AgentKey>,
    /// The core raised the worker past its Conductor to the operator.
    pub escalation: Option<PhoneEscalation>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PhoneEscalation {
    pub cause: String,
    /// The line its blocked turn left, when it left one.
    pub line: Option<String>,
    /// The person's turn it declared, which the step says as its waiting
    /// reason (PRD ask-human D-18, B25); absent for every other cause.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub human: Option<PhoneHuman>,
}

/// What a person's turn asks and where, as a step's waiting reason says it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PhoneHuman {
    pub kind: String,
    pub place: String,
}

#[derive(Deserialize)]
struct MissionIn {
    id: String,
    title: String,
    device_id: String,
    status: String,
    #[serde(default)]
    decision: Option<String>,
    #[serde(default)]
    question: Option<Question>,
    #[serde(default)]
    answer: Option<PhoneAnswer>,
    #[serde(default)]
    answer_failed: Option<String>,
    conductor: ConductorIn,
    #[serde(default)]
    conductor_letters: Option<PhoneHeldLetters>,
    #[serde(default)]
    nodes: Vec<NodeIn>,
    #[serde(default)]
    landing_order: Vec<String>,
    created_at_unix_ms: u64,
    #[serde(default)]
    started_at_unix_ms: Option<u64>,
    #[serde(default)]
    closed_at_unix_ms: Option<u64>,
    #[serde(default)]
    stale: bool,
}

#[derive(Deserialize)]
struct ConductorIn {
    pane_id: String,
    name: String,
    agent_kind: String,
    present: bool,
    takes_answers: bool,
}

#[derive(Deserialize)]
struct NodeIn {
    key: String,
    title: String,
    kind: String,
    #[serde(default)]
    depends_on: Vec<String>,
    status: String,
    #[serde(default)]
    blocked: Option<String>,
    #[serde(default)]
    delivery: Option<String>,
    #[serde(default)]
    pane_id: Option<String>,
    #[serde(default)]
    pr: Option<u32>,
    #[serde(default)]
    pr_state: Option<String>,
    #[serde(default)]
    base_pr: Option<u32>,
    #[serde(default)]
    stage: Option<u8>,
    #[serde(default)]
    attention: Option<PhoneAttention>,
    #[serde(default)]
    landing: Option<PhoneLanding>,
    #[serde(default)]
    agent_runtime: Option<String>,
    #[serde(default)]
    worker: Option<WorkerIn>,
}

#[derive(Deserialize)]
struct WorkerIn {
    agent: String,
}

/// The facts of one raw agent row the missions join on: whether the core
/// raised it past its Conductor, and the line its blocked turn left.
struct RawAgent<'a> {
    escalation: Option<&'a str>,
    blocked_line: Option<&'a str>,
    /// The person's turn the row leads with, while that is its escalation.
    human: Option<PhoneHuman>,
}

impl PhoneMissions {
    /// The phone's missions from the same `rest` the agent list is built
    /// from, with `agents` the list built from it, so a step or a Conductor
    /// opens only a row the phone can open.
    pub fn project(rest: &Value, node: &str, agents: &Projection) -> Self {
        let Some(book) = rest.get("missions") else {
            return Self::default();
        };
        let failed = book.get("failed").is_some_and(|reason| !reason.is_null());
        let raw = raw_agents(rest);
        let mut open = Vec::new();
        let mut closed = Vec::new();
        for item in book
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let mission = match MissionIn::deserialize(item) {
                Ok(mission) => mission,
                Err(error) => {
                    crate::diagnostic!(serde_json::json!({
                        "component": "mobile_phone", "kind": "mission.projection_invalid",
                        "message": error.to_string(),
                    }));
                    continue;
                }
            };
            let target = if matches!(mission.status.as_str(), "done" | "cancelled") {
                &mut closed
            } else {
                &mut open
            };
            target.push(mission);
        }
        open.sort_by_key(|mission| std::cmp::Reverse(mission.created_at_unix_ms));
        closed.sort_by_key(|mission| {
            std::cmp::Reverse(
                mission
                    .closed_at_unix_ms
                    .unwrap_or(mission.created_at_unix_ms),
            )
        });
        let mut omitted =
            open.len().saturating_sub(MAX_OPEN) + closed.len().saturating_sub(MAX_CLOSED);
        open.truncate(MAX_OPEN);
        closed.truncate(MAX_CLOSED);
        let labels = device_labels(rest);
        let project = |mission| phone_mission(mission, node, &labels, &raw, agents);
        let mut open: Vec<PhoneMission> = open.into_iter().map(project).collect();
        let mut closed: Vec<PhoneMission> = closed.into_iter().map(project).collect();
        omitted += fit(&mut open, &mut closed, LIST_BUDGET);
        open.append(&mut closed);
        Self {
            items: open,
            failed,
            omitted,
        }
    }

    pub fn find(&self, id: &str) -> Option<&PhoneMission> {
        self.items.iter().find(|mission| mission.id == id)
    }
}

/// Keeps the missions within `budget` bytes of JSON items, `open` and
/// `closed` each newest first, and answers how many missions it dropped. What
/// matters least goes first: the steps of finished missions, the oldest first
/// (the row stays, with `nodes_omitted`), then finished missions, then open
/// ones, the oldest first. A step of an open mission is never dropped alone,
/// because its progress cells and its decision read every step.
fn fit(open: &mut Vec<PhoneMission>, closed: &mut Vec<PhoneMission>, budget: usize) -> usize {
    let sizes = |items: &[PhoneMission]| items.iter().map(wire_len).collect::<Vec<_>>();
    let (open_sizes, mut closed_sizes) = (sizes(open), sizes(closed));
    // Every item and the comma before it; the brackets are the frame's.
    let mut total: usize = open_sizes
        .iter()
        .chain(&closed_sizes)
        .map(|size| size + 1)
        .sum();
    for (mission, size) in closed.iter_mut().zip(&mut closed_sizes).rev() {
        if total <= budget {
            return 0;
        }
        if mission.nodes.is_empty() {
            continue;
        }
        mission.nodes_omitted = mission.nodes.len();
        mission.nodes.clear();
        mission.landing_order.clear();
        let stripped = wire_len(mission);
        total -= *size - stripped;
        *size = stripped;
    }
    let mut kept_closed = closed.len();
    while total > budget && kept_closed > 0 {
        kept_closed -= 1;
        total -= closed_sizes[kept_closed] + 1;
    }
    let mut kept_open = open.len();
    while total > budget && kept_open > 0 {
        kept_open -= 1;
        total -= open_sizes[kept_open] + 1;
    }
    let dropped = (closed.len() - kept_closed) + (open.len() - kept_open);
    closed.truncate(kept_closed);
    open.truncate(kept_open);
    dropped
}

fn phone_mission(
    mission: MissionIn,
    node: &str,
    labels: &HashMap<&str, &str>,
    raw: &HashMap<&str, RawAgent<'_>>,
    agents: &Projection,
) -> PhoneMission {
    let listed = |pane: &str| {
        agents
            .agents()
            .find(|agent| agent.pane_id == pane)
            .map(|agent| agent.key())
    };
    let device_label = (mission.device_id != node).then(|| {
        labels
            .get(mission.device_id.as_str())
            .map_or_else(|| mission.device_id.clone(), |label| (*label).to_owned())
    });
    let conductor = PhoneConductor {
        agent: listed(&mission.conductor.pane_id),
        name: cut(&mission.conductor.name),
        agent_kind: mission.conductor.agent_kind,
        present: mission.conductor.present,
        takes_answers: mission.conductor.takes_answers,
    };
    let nodes = mission
        .nodes
        .into_iter()
        .map(|source| {
            let pane = source.pane_id.as_deref();
            let escalation = pane.and_then(|pane| raw.get(pane)).and_then(|agent| {
                Some(PhoneEscalation {
                    cause: agent.escalation?.to_owned(),
                    line: agent.blocked_line.map(cut),
                    human: agent.human.clone(),
                })
            });
            PhoneNode {
                key: source.key,
                title: cut(&source.title),
                kind: source.kind,
                depends_on: source.depends_on,
                status: source.status,
                blocked: source.blocked,
                delivery: source.delivery,
                pr: source.pr,
                pr_state: source.pr_state,
                base_pr: source.base_pr,
                stage: source.stage,
                attention: source.attention,
                landing: source.landing,
                agent_runtime: source.agent_runtime,
                worker_agent: source.worker.map(|worker| worker.agent),
                agent: pane.and_then(listed),
                escalation,
            }
        })
        .collect();
    PhoneMission {
        id: mission.id,
        title: cut(&mission.title),
        device_id: mission.device_id,
        device_label,
        status: mission.status,
        decision: mission.decision,
        question: mission.question,
        answer: mission.answer,
        answer_failed: mission.answer_failed,
        conductor,
        conductor_letters: mission.conductor_letters,
        nodes,
        nodes_omitted: 0,
        landing_order: mission.landing_order,
        created_at_unix_ms: mission.created_at_unix_ms,
        started_at_unix_ms: mission.started_at_unix_ms,
        closed_at_unix_ms: mission.closed_at_unix_ms,
        stale: mission.stale,
    }
}

fn cut(text: &str) -> String {
    text.chars().take(TEXT_CHARS).collect()
}

/// A device's name by id, as the desktop's device list gives it.
fn device_labels(rest: &Value) -> HashMap<&str, &str> {
    rest.pointer("/navigator/devices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|device| {
            let label = device.get("label").and_then(Value::as_str)?;
            (!label.is_empty()).then_some((device.get("id")?.as_str()?, label))
        })
        .collect()
}

/// Every agent row of this Mac and of each device, by pane id, as the
/// desktop's `agentsByPane` joins a node to its worker.
fn raw_agents(rest: &Value) -> HashMap<&str, RawAgent<'_>> {
    let local = rest
        .pointer("/navigator/agents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    let remote = rest
        .pointer("/status/remote")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|status| status.pointer("/session/agents")?.as_array())
        .flatten();
    local
        .chain(remote)
        .filter_map(|agent| {
            let pane = agent.get("pane_id")?.as_str()?;
            let blocked = agent
                .pointer("/request/end")
                .and_then(Value::as_str)
                .is_some_and(|end| end == "blocked");
            Some((
                pane,
                RawAgent {
                    human: agent
                        .pointer("/escalation/cause")
                        .filter(|cause| *cause == "human_turn")
                        .and_then(|_| {
                            Some(PhoneHuman {
                                kind: agent.pointer("/human_turn/kind")?.as_str()?.to_owned(),
                                place: agent.pointer("/human_turn/place")?.as_str()?.to_owned(),
                            })
                        }),
                    escalation: agent.pointer("/escalation/cause").and_then(Value::as_str),
                    blocked_line: blocked
                        .then(|| agent.pointer("/request/line").and_then(Value::as_str))
                        .flatten()
                        .filter(|line| !line.is_empty()),
                },
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::agent_state::phone::project;

    fn node(key: &str, extra: Value) -> Value {
        let mut value = json!({
            "key": key, "title": format!("step {key}"), "kind": "change", "depends_on": [],
            "status": "working", "blocked": null, "delivery": "pr", "worker": {"agent": "codex", "model": "x"},
            "agent_id": "agent-9", "pane_id": null, "machine": null, "pr": null, "prs": [], "base_pr": null,
            "branch": "/private/never-send", "pr_url": "https://github.com/private/never-send",
            "pr_state": null, "stage": 2, "line": "private line", "criteria": [{"text": "private finish line"}],
            "attention": null, "landing": null, "agent_runtime": "running", "review": "private review",
        });
        for (name, field) in extra.as_object().unwrap() {
            value[name] = field.clone();
        }
        value
    }

    fn mission(id: &str, created: u64, status: &str, nodes: Vec<Value>) -> Value {
        json!({
            "id": id, "title": format!("mission {id}"), "workspace_id": "w1",
            "project": "/private/never-send/repo", "checkout_path": "/private/never-send/repo-wt",
            "device_id": "local", "status": status, "scope": "merge", "cap": 3, "exempt": false,
            "strength": "normal",
            "conductor": {"pane_id": "w1:p1", "name": "Conductor", "agent_kind": "claude",
                          "session": {"device_id": "local", "session_id": "private-session"},
                          "present": true, "takes_answers": true},
            "nodes": nodes, "merge_order": [], "landing_order": ["a"], "stack_waiting": 0,
            "check": "private check", "github": "private/repo", "decision": null, "question": null,
            "answer": null, "answer_failed": null, "end_asked": false, "delay": null,
            "report": "private report", "created_at_unix_ms": created, "started_at_unix_ms": created,
            "closed_at_unix_ms": if status == "done" { json!(created + 5) } else { Value::Null },
            "running": 1, "stale": false, "github_stale": false, "github_read_at_unix_ms": null,
        })
    }

    fn agent(pane: &str, extra: Value) -> Value {
        let mut value = json!({
            "pane_id": pane, "group": "working", "mark": "going", "identity_label": format!("task {pane}"),
            "agent_kind": "claude", "changed_at_unix_ms": 1_790_000_000_000_u64, "status_code": "idle", "demand": "none",
        });
        for (name, field) in extra.as_object().unwrap() {
            value[name] = field.clone();
        }
        value
    }

    fn rest(missions: Vec<Value>, extra: Value) -> Value {
        let mut rest = json!({
            "navigator": {"agents": [agent("w1:p1", json!({})), agent("w1:p2", json!({}))], "devices": [{"id": "local", "label": null}, {"id": "mini", "label": "Mac mini"}]},
            "status": {"remote": []},
            "missions": {"items": missions},
        });
        for (name, field) in extra.as_object().unwrap() {
            rest[name] = field.clone();
        }
        rest
    }

    fn projected(rest: &Value) -> PhoneMissions {
        PhoneMissions::project(rest, "local", &project(rest, "local"))
    }

    #[test]
    fn a_mission_reaches_the_phone_without_a_path_a_session_or_a_review() {
        let rest = rest(
            vec![mission("m1", 10, "running", vec![node("a", json!({}))])],
            json!({}),
        );
        let missions = projected(&rest);
        assert_eq!(missions.items.len(), 1);
        let sent = serde_json::to_string(&missions).unwrap();
        for private in [
            "never-send",
            "private",
            "agent-9",
            "workspace",
            "checkout",
            "criteria",
            "model",
        ] {
            assert!(!sent.contains(private), "reached the phone: {private}");
        }
        assert_eq!(missions.items[0].title, "mission m1");
        assert_eq!(
            missions.items[0].nodes[0].worker_agent.as_deref(),
            Some("codex")
        );
    }

    #[test]
    fn a_steps_landing_reaches_the_phone_by_its_named_fields_only() {
        let landed = json!({"state": "landed", "proof": {"by": "in_main", "commit": "abc1234", "path": "/private/never-send"}, "note": "private"});
        let left = json!({"state": "left", "action": "merge", "branch": "private-branch"});
        let rest = rest(
            vec![mission(
                "m1",
                10,
                "running",
                vec![
                    node("a", json!({"landing": landed})),
                    node("b", json!({"landing": left})),
                ],
            )],
            json!({}),
        );
        let nodes = &projected(&rest).items[0].nodes;
        let sent = serde_json::to_value(nodes).unwrap();
        assert_eq!(
            sent[0]["landing"],
            json!({"state": "landed", "proof": {"by": "in_main", "commit": "abc1234"}})
        );
        assert_eq!(
            sent[1]["landing"],
            json!({"state": "left", "action": "merge"})
        );
        assert!(!sent.to_string().contains("private"));
    }

    #[test]
    fn a_step_and_the_conductor_open_only_rows_the_phone_lists() {
        let rest = rest(
            vec![mission(
                "m1",
                10,
                "running",
                vec![
                    node("a", json!({"pane_id": "w1:p2"})),
                    node("b", json!({"pane_id": "w9:p9"})),
                ],
            )],
            json!({}),
        );
        let mission = &projected(&rest).items[0];
        let key = |pane: &str| {
            Some(AgentKey {
                device_id: "local".into(),
                pane_id: pane.into(),
            })
        };
        assert_eq!(mission.conductor.agent, key("w1:p1"));
        assert_eq!(mission.nodes[0].agent, key("w1:p2"));
        assert_eq!(
            mission.nodes[1].agent, None,
            "a pane the list does not hold has no row to open"
        );
    }

    #[test]
    fn a_worker_the_core_raised_carries_its_cause_and_the_line_its_blocked_turn_left_or_its_persons_turn()
     {
        let mut agents = vec![
            agent("w1:p1", json!({})),
            agent(
                "w1:p2",
                json!({
                    "escalation": {"cause": "child_blocked", "letter_id": null}, "request": {"end": "blocked", "line": "Login expired"},
                }),
            ),
        ];
        agents.push(agent("w1:p3", json!({"escalation": {"cause": "draft"}, "request": {"end": "done", "line": "not blocked"}})));
        agents.push(agent("w1:p4", json!({"escalation": {"cause": "human_turn"}, "human_turn": {"id": "w.1", "kind": "auth", "place": "github.com/login", "pane_id": "w1:p4", "title": "w", "since_unix_ms": 1, "own": true}})));
        let rest = rest(
            vec![mission(
                "m1",
                10,
                "running",
                vec![
                    node("a", json!({"pane_id": "w1:p2"})),
                    node("b", json!({"pane_id": "w1:p3"})),
                    node("c", json!({})),
                    node("d", json!({"pane_id": "w1:p4"})),
                ],
            )],
            json!({"navigator": {"agents": agents, "devices": []}}),
        );
        let nodes = &projected(&rest).items[0].nodes;
        assert_eq!(
            nodes[0].escalation,
            Some(PhoneEscalation {
                cause: "child_blocked".into(),
                line: Some("Login expired".into()),
                human: None,
            })
        );
        assert_eq!(
            nodes[1].escalation,
            Some(PhoneEscalation {
                cause: "draft".into(),
                line: None,
                human: None,
            })
        );
        assert_eq!(nodes[2].escalation, None);
        // A person's turn says what and where, as the step's waiting reason (PRD ask-human B25).
        assert_eq!(
            nodes[3].escalation,
            Some(PhoneEscalation {
                cause: "human_turn".into(),
                line: None,
                human: Some(PhoneHuman {
                    kind: "auth".into(),
                    place: "github.com/login".into(),
                }),
            })
        );
    }

    #[test]
    fn a_mission_on_another_device_names_it_and_one_on_this_mac_does_not() {
        let mut away = mission("away", 20, "running", vec![]);
        away["device_id"] = json!("mini");
        let rest = rest(
            vec![mission("home", 10, "running", vec![]), away],
            json!({}),
        );
        let missions = projected(&rest);
        assert_eq!(missions.find("home").unwrap().device_label, None);
        assert_eq!(
            missions.find("away").unwrap().device_label.as_deref(),
            Some("Mac mini")
        );
    }

    #[test]
    fn the_frame_keeps_the_newest_missions_and_counts_the_rest() {
        let open: Vec<Value> = (0..MAX_OPEN as u64 + 3)
            .map(|at| mission(&format!("o{at}"), 100 + at, "running", vec![]))
            .collect();
        let closed: Vec<Value> = (0..MAX_CLOSED as u64 + 2)
            .map(|at| mission(&format!("c{at}"), 100 + at, "done", vec![]))
            .collect();
        let rest = rest(open.into_iter().chain(closed).collect(), json!({}));
        let missions = projected(&rest);
        assert_eq!(missions.items.len(), MAX_OPEN + MAX_CLOSED);
        assert_eq!(missions.omitted, 5);
        assert_eq!(
            missions.items[0].id,
            format!("o{}", MAX_OPEN + 2),
            "the newest open mission leads"
        );
        assert!(
            missions.find("o0").is_none() && missions.find("c0").is_none(),
            "the oldest of each are the ones left out"
        );
    }

    /// A mission with `nodes` steps whose titles are as long as the frame
    /// keeps them, in a script of three bytes a character.
    fn full(id: &str, created: u64, status: &str, nodes: usize) -> Value {
        let long = "가".repeat(TEXT_CHARS);
        let mut value = mission(
            id,
            created,
            status,
            (0..nodes)
                .map(|at| {
                    node(
                        &format!("n{at}"),
                        json!({"title": long, "pane_id": "w1:p2"}),
                    )
                })
                .collect(),
        );
        value["title"] = json!(long);
        value
    }

    #[test]
    fn the_most_missions_with_the_most_steps_fit_one_relay_message_and_what_is_left_out_is_counted()
    {
        let missions: Vec<Value> = (0..MAX_OPEN as u64)
            .map(|at| full(&format!("o{at}"), 1_000 + at, "running", 64))
            .chain((0..MAX_CLOSED as u64).map(|at| full(&format!("c{at}"), 100 + at, "done", 64)))
            .collect();
        let rest = rest(missions, json!({}));
        let projected = projected(&rest);
        let frame = json!({"type": "missions", "items": projected.items, "failed": projected.failed, "omitted": projected.omitted});
        let bytes = frame.to_string().len();
        assert!(bytes <= super::super::FRAME_LIMIT, "{bytes} bytes");
        let carried = projected.items.len();
        assert_eq!(carried + projected.omitted, MAX_OPEN + MAX_CLOSED);
        assert!(
            projected.omitted > 0,
            "64 steps of 48 missions cannot all fit"
        );
        assert!(
            projected
                .items
                .iter()
                .all(|mission| mission.status == "running"),
            "every finished mission goes before an open one"
        );
        assert_eq!(
            projected.items[0].id,
            format!("o{}", MAX_OPEN - 1),
            "the newest open mission stays"
        );
        assert!(
            projected
                .items
                .iter()
                .all(|mission| mission.nodes.len() == 64),
            "an open mission keeps every step"
        );
    }

    #[test]
    fn finished_missions_lose_their_steps_oldest_first_before_any_mission_is_left_out() {
        // Two open missions of the longest steps take about half the budget.
        let missions: Vec<Value> = (0..2u64)
            .map(|at| full(&format!("o{at}"), 1_000 + at, "running", 64))
            .chain((0..MAX_CLOSED as u64).map(|at| full(&format!("c{at}"), 100 + at, "done", 64)))
            .collect();
        let projected = projected(&rest(missions, json!({})));
        assert_eq!(projected.omitted, 0, "every mission stays listed");
        assert_eq!(projected.items.len(), 2 + MAX_CLOSED);
        let newest = projected.find(&format!("c{}", MAX_CLOSED - 1)).unwrap();
        let oldest = projected.find("c0").unwrap();
        assert_eq!((newest.nodes.len(), newest.nodes_omitted), (64, 0));
        assert_eq!((oldest.nodes.len(), oldest.nodes_omitted), (0, 64));
        assert!(oldest.landing_order.is_empty());
        assert_eq!(projected.nodes_omitted() % 64, 0);
        assert!(projected.nodes_omitted() > 0);
        let sent = serde_json::to_value(oldest).unwrap();
        assert_eq!(sent["nodes_omitted"], 64);
        assert!(
            serde_json::to_value(newest)
                .unwrap()
                .get("nodes_omitted")
                .is_none()
        );
    }

    #[test]
    fn a_frame_within_the_budget_leaves_nothing_out() {
        let missions: Vec<Value> = (0..3u64)
            .map(|at| full(&format!("c{at}"), 100 + at, "done", 8))
            .collect();
        let projected = projected(&rest(missions, json!({})));
        assert_eq!((projected.omitted, projected.nodes_omitted()), (0, 0));
        assert!(
            projected
                .items
                .iter()
                .all(|mission| mission.nodes.len() == 8)
        );
    }

    #[test]
    fn a_mission_record_the_core_could_not_read_is_failed_and_an_invalid_item_is_skipped() {
        let mut broken = rest(vec![json!({"id": "bad"})], json!({}));
        broken["missions"]["failed"] = json!("unreadable");
        let missions = projected(&broken);
        assert!(missions.failed);
        assert!(missions.items.is_empty());
        assert_eq!(
            projected(&json!({"navigator": {"agents": []}})),
            PhoneMissions::default()
        );
    }
}
