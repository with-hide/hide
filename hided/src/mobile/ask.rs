//! The ask a phone's detail answers on the agent's own screen: its question
//! with the choices and what the agent says under each (Doop 03), its plan
//! waiting for approval, or a tool call waiting for permission with the
//! command it runs, cut (Doop 04). It reaches the phone in a frame of its own
//! (`ask`), only while that agent's detail is open, never in the list frame.
//!
//! A choice is never sent as text: a pasted choice is read as words and the
//! Enter after it picks whatever the screen has highlighted (live-checks,
//! 2026-11-23). The core types the keys the agent's own screen takes for it
//! (`hide_core::operator_input::choice_keys`), after checking the agent is
//! still on the same ask; an agent, or a kind of ask, without those keys
//! shows its choices and is answered in the terminal.

use super::projection::PhoneAgent;
use hide_core::operator_input::{self, Ask};
use hide_session::turns::{UserTurnContent, UserTurnKind};
use serde_json::{Value, json};

/// What `agent` waits on that a choice answers: its own question of one
/// choice, its plan, or a tool call's permission. `None` while it asks
/// nothing, or asks what the record does not hold.
pub fn kind_of(agent: &PhoneAgent) -> Option<Ask> {
    if !matches!(agent.demand.as_str(), "question" | "approval") {
        return None;
    }
    match agent.user_turn.as_ref() {
        Some(fact) => match (fact.kind, fact.content.as_ref()) {
            (UserTurnKind::PlanApproval, _) => Some(Ask::PlanApproval),
            (UserTurnKind::Question, Some(content)) => {
                Some(Ask::Question(content.choices().len()))
            }
            // A question whose words the record does not hold is answered
            // with a reply.
            (UserTurnKind::Question, None) => None,
        },
        None if agent.demand == "approval" => Some(Ask::Permission),
        None => None,
    }
}

/// The turn an answer is written for: the agent's last change of state,
/// which a notice's Reply carries too.
pub fn turn_of(agent: &PhoneAgent) -> Option<String> {
    agent.changed_at_unix_ms.map(|at| at.to_string())
}

/// The keys that answer `choice` on what `agent` asks, if it still asks it
/// in `turn`: `Err` names why nothing may be typed.
pub fn keys(agent: &PhoneAgent, turn: &str, choice: &str) -> Result<Vec<String>, &'static str> {
    let kind = kind_of(agent).ok_or("stale")?;
    if turn_of(agent).as_deref() != Some(turn) {
        return Err("stale");
    }
    // A question picked by its place only when it asks one choice of one
    // question: any other form takes more than one pick.
    if let Ask::Question(_) = kind
        && !question(agent).is_some_and(UserTurnContent::pick_one)
    {
        return Err("unknown_choice");
    }
    operator_input::choice_keys(&agent.agent_kind, kind, choice).ok_or("unknown_choice")
}

fn question(agent: &PhoneAgent) -> Option<&UserTurnContent> {
    agent.user_turn.as_ref()?.content.as_ref()
}

/// The ask as the phone draws it, or `None`: its kind, the turn an answer
/// names, since when it waits, its words, its choices, the command a
/// permission waits on, and whether the choices can be picked here.
pub fn of(agent: &PhoneAgent, command: Option<&str>) -> Option<Value> {
    let kind = kind_of(agent)?;
    let turn = turn_of(agent)?;
    let offered = operator_input::choices(&agent.agent_kind, kind);
    let mut ask = json!({
        "kind": kind.name(), "turn": turn, "since_ms": agent.changed_at_unix_ms,
    });
    let choices: Vec<Value> = match kind {
        Ask::Question(_) => {
            let content = question(agent)?;
            ask["text"] = json!(content.text());
            ask["truncated"] = json!(content.truncated());
            ask["answerable"] = json!(content.pick_one() && !offered.is_empty());
            content
                .choices()
                .iter()
                .enumerate()
                .map(|(at, label)| {
                    let mut choice = json!({"id": (at + 1).to_string(), "label": label});
                    if let Some(note) = content.note(at) {
                        choice["note"] = json!(note);
                    }
                    choice
                })
                .collect()
        }
        Ask::PlanApproval => {
            if let Some(content) = question(agent) {
                ask["text"] = json!(content.text());
                ask["truncated"] = json!(content.truncated());
            }
            ask["answerable"] = json!(!offered.is_empty());
            offered.iter().map(|id| json!({"id": id})).collect()
        }
        Ask::Permission => {
            if let Some(command) = command {
                let (shown, cut) = super::calls::cut(command);
                ask["command"] = json!(shown);
                ask["command_truncated"] = json!(cut);
            }
            ask["answerable"] = json!(!offered.is_empty());
            offered.iter().map(|id| json!({"id": id})).collect()
        }
    };
    ask["choices"] = Value::Array(choices);
    Some(ask)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobile::projection::{AgentKey, Projection, project};

    fn listed(agent: Value) -> Projection {
        let agent = json!({"pane_id": "w1:p1", "group": "needs_you", "mark": "turn",
            "identity_label": "Review", "changed_at_unix_ms": 100})
        .as_object()
        .unwrap()
        .clone()
        .into_iter()
        .chain(agent.as_object().unwrap().clone())
        .collect::<serde_json::Map<_, _>>();
        project(&json!({"navigator": {"agents": [agent]}}), "test-node")
    }

    fn agent(projection: &Projection) -> &PhoneAgent {
        projection
            .find(&AgentKey {
                device_id: "test-node".into(),
                pane_id: "w1:p1".into(),
            })
            .unwrap()
    }

    /// The agent's own question of one choice reaches the detail with each
    /// choice's note, and its choice is typed as the keys its agent's screen
    /// takes; a question of several choices at once is shown and answered in
    /// the terminal, and a turn the agent has left types nothing.
    #[test]
    fn a_question_is_asked_with_its_notes_and_picked_by_its_agents_keys() {
        let question = |pick_one: bool| {
            listed(json!({"agent_kind": "claude", "status_code": "question", "demand": "question",
                "user_turn": {"kind": "question", "content": {"text": "어느 쪽?", "choices": ["전부", "나눠서"],
                "notes": ["리뷰 4건을 이 PR에서", ""], "pick_one": pick_one, "truncated": false}}}))
        };
        let one = question(true);
        let asked = of(agent(&one), None).unwrap();
        assert_eq!(
            asked,
            json!({"kind": "question", "turn": "100", "since_ms": 100, "text": "어느 쪽?",
                "truncated": false, "answerable": true, "choices": [
                    {"id": "1", "label": "전부", "note": "리뷰 4건을 이 PR에서"},
                    {"id": "2", "label": "나눠서"}]})
        );
        assert_eq!(
            keys(agent(&one), "100", "2"),
            Ok(vec!["down".to_owned(), "enter".to_owned()])
        );
        assert_eq!(keys(agent(&one), "99", "2"), Err("stale"));
        assert_eq!(keys(agent(&one), "100", "3"), Err("unknown_choice"));
        assert_eq!(keys(agent(&one), "100", "allow"), Err("unknown_choice"));
        let several = question(false);
        assert_eq!(of(agent(&several), None).unwrap()["answerable"], false);
        assert_eq!(keys(agent(&several), "100", "1"), Err("unknown_choice"));
    }

    /// A tool call waiting for permission is shown with the first of the
    /// command it waits on, and the choices its agent's screen takes; an
    /// agent without them shows the command alone, for the terminal.
    #[test]
    fn a_permission_shows_the_cut_command_and_its_agents_choices() {
        let waiting = |kind: &str| {
            listed(json!({"agent_kind": kind, "status_code": "approval", "demand": "approval"}))
        };
        let opencode = waiting("opencode");
        let asked = of(agent(&opencode), Some("cargo test\nTOKEN=x\nmore")).unwrap();
        assert_eq!(asked["kind"], "permission");
        assert_eq!(asked["command"], "cargo test\nTOKEN=x");
        assert_eq!(asked["command_truncated"], true);
        assert_eq!(
            asked["choices"],
            json!([{"id": "allow"}, {"id": "allow_always"}, {"id": "deny"}])
        );
        assert_eq!(keys(agent(&opencode), "100", "deny"), Ok(vec!["esc".to_owned()]));
        let pi = waiting("pi");
        let asked = of(agent(&pi), Some("ls")).unwrap();
        assert_eq!((asked["answerable"].clone(), asked["choices"].clone()), (json!(false), json!([])));
        assert_eq!(keys(agent(&pi), "100", "allow"), Err("unknown_choice"));
        let resting = listed(json!({"agent_kind": "claude", "status_code": "idle", "demand": "none"}));
        assert_eq!(of(agent(&resting), Some("ls")), None);
    }
}
