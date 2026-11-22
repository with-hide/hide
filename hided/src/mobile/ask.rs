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
//! still on the ask the phone was shown, of the same kind, and while its
//! screen shows that ask: its rows and the command a permission was shown
//! with before the first key, Esc included, and the highlight where each
//! key that counts from it expects it. An agent, or a
//! kind of ask, without those keys, or whose highlight its table cannot
//! read, shows its choices and is answered in the terminal.
//!
//! A permission the record does not name is tied to the session's open
//! calls ([`Pending`]): the one open call's command, or its plan, when it is
//! Claude Code's `ExitPlanMode`. With no such call, or several, the card
//! shows no command and is answered in the terminal, since the screen could
//! not be checked against it; an agent that records a call only once it is
//! allowed (`unnamed`, Codex) is answered as it stands.

use super::calls::Pending;
use super::projection::PhoneAgent;
use hide_core::operator_input::{self, Ask, Screen};
use hide_session::turns::{UserTurnContent, UserTurnKind};
use serde_json::{Value, json};

/// What `agent` waits on that a choice answers, as its record says: its own
/// question of one choice, its plan, or a tool call's permission. `None`
/// while it asks nothing, or asks what the record does not hold.
pub fn kind_of(agent: &PhoneAgent) -> Option<Ask> {
    if !matches!(agent.demand.as_str(), "question" | "approval") {
        return None;
    }
    match agent.user_turn.as_ref() {
        Some(fact) => match (fact.kind, fact.content.as_ref()) {
            (UserTurnKind::PlanApproval, _) => Some(Ask::PlanApproval),
            (UserTurnKind::Question, Some(content)) => Some(Ask::Question(content.choices().len())),
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

/// The ask as the phone was last shown it, which a choice is checked
/// against before its keys are typed.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub kind: Ask,
    pub turn: String,
    /// The command the card shows, which the agent's screen must show too.
    pub command: Option<String>,
    pub answerable: bool,
    /// The ask frame's body.
    pub frame: Value,
}

/// The ask as the phone draws it, or `None`: its kind, the turn an answer
/// names, since when it waits, its words, its choices, the call a permission
/// is tied to with the command it waits on, and whether the choices can be
/// picked here. `pending` is what the session's open calls say, `None` when
/// they are not read.
pub fn of(agent: &PhoneAgent, pending: Option<&Pending<'_>>) -> Option<Shown> {
    let turn = turn_of(agent)?;
    let (kind, call, command) = match kind_of(agent)? {
        Ask::Permission if operator_input::unnamed_permission(&agent.agent_kind) => {
            (Ask::Permission, None, None)
        }
        Ask::Permission => match pending {
            Some(Pending::One(open)) if open.plan => (Ask::PlanApproval, Some(&open.id), None),
            // A command of no words cannot be checked on the screen.
            Some(Pending::One(open)) => (
                Ask::Permission,
                Some(&open.id),
                open.command
                    .clone()
                    .filter(|command| !command.trim().is_empty()),
            ),
            _ => (Ask::Permission, None, None),
        },
        recorded => (recorded, None, None),
    };
    let offered = operator_input::choices(&agent.agent_kind, kind);
    let mut frame = json!({
        "kind": kind.name(), "turn": turn, "since_ms": agent.changed_at_unix_ms,
    });
    if let Some(call) = call {
        frame["call"] = json!(call);
    }
    let answerable;
    let choices: Vec<Value> = match kind {
        Ask::Question(_) => {
            let content = question(agent)?;
            frame["text"] = json!(content.text());
            frame["truncated"] = json!(content.truncated());
            answerable = content.pick_one() && !offered.is_empty();
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
                frame["text"] = json!(content.text());
                frame["truncated"] = json!(content.truncated());
            }
            answerable = !offered.is_empty();
            offered.iter().map(|id| json!({"id": id})).collect()
        }
        Ask::Permission => {
            if let Some(command) = &command {
                let (shown, cut) = super::calls::cut(command);
                frame["command"] = json!(shown);
                frame["command_truncated"] = json!(cut);
            }
            // A permission whose command the screen cannot be checked
            // against is answered where the operator sees what it allows,
            // and lists no choices: it may be a plan's, which has others.
            answerable = !offered.is_empty()
                && (command.is_some() || operator_input::unnamed_permission(&agent.agent_kind));
            if answerable {
                offered.iter().map(|id| json!({"id": id})).collect()
            } else {
                Vec::new()
            }
        }
    };
    frame["answerable"] = json!(answerable);
    frame["choices"] = Value::Array(choices);
    Some(Shown {
        kind,
        turn,
        command,
        answerable,
        frame,
    })
}

/// The keys that answer `choice` on the ask the phone was shown (`shown`),
/// declared `kind` in `turn`, with what the screen must show while they are
/// typed: `Err` names why nothing may be typed, `stale` when the agent no
/// longer asks it.
pub fn keys(
    agent: &PhoneAgent,
    shown: Option<&Shown>,
    turn: &str,
    kind: &str,
    choice: &str,
) -> Result<(Vec<String>, Screen), &'static str> {
    let shown = shown.ok_or("stale")?;
    if kind_of(agent).is_none()
        || turn_of(agent).as_deref() != Some(turn)
        || shown.turn != turn
        || shown.kind.name() != kind
    {
        return Err("stale");
    }
    // A question of several picks, a permission the screen cannot be
    // checked against, or an agent without keys was shown for the terminal.
    if !shown.answerable {
        return Err("unknown_choice");
    }
    let keys = operator_input::choice_keys(&agent.agent_kind, shown.kind, choice)
        .ok_or("unknown_choice")?;
    let labels = match shown.kind {
        Ask::Question(_) => question(agent).map(|content| content.choices().to_vec()),
        _ => None,
    };
    let screen = operator_input::choice_screen(
        &agent.agent_kind,
        shown.kind,
        labels.as_deref().unwrap_or_default(),
    );
    let screen = match &shown.command {
        Some(command) => screen.showing(command),
        None => screen,
    };
    Ok((keys, screen))
}

fn question(agent: &PhoneAgent) -> Option<&UserTurnContent> {
    agent.user_turn.as_ref()?.content.as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobile::calls::OpenCall;
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

    fn open(id: &str, command: Option<&str>, plan: bool) -> OpenCall {
        OpenCall {
            id: id.to_owned(),
            command: command.map(str::to_owned),
            plan,
            since_ms: Some(90),
        }
    }

    /// The agent's own question of one choice reaches the detail with each
    /// choice's note, and its choice is typed as the keys its agent's screen
    /// takes; a question of several choices at once is shown and answered in
    /// the terminal, and a turn or a kind the agent has left types nothing.
    #[test]
    fn a_question_is_asked_with_its_notes_and_picked_by_its_agents_keys() {
        let question = |pick_one: bool| {
            listed(
                json!({"agent_kind": "claude", "status_code": "question", "demand": "question",
                "user_turn": {"kind": "question", "content": {"text": "어느 쪽?", "choices": ["전부", "나눠서"],
                "notes": ["리뷰 4건을 이 PR에서", ""], "pick_one": pick_one, "truncated": false}}}),
            )
        };
        let one = question(true);
        let shown = of(agent(&one), None).unwrap();
        assert_eq!(
            shown.frame,
            json!({"kind": "question", "turn": "100", "since_ms": 100, "text": "어느 쪽?",
                "truncated": false, "answerable": true, "choices": [
                    {"id": "1", "label": "전부", "note": "리뷰 4건을 이 PR에서"},
                    {"id": "2", "label": "나눠서"}]})
        );
        let keys_of = |turn: &str, kind: &str, choice: &str| {
            keys(agent(&one), Some(&shown), turn, kind, choice)
        };
        // Down and Enter count from the first row, so they wait for it.
        let (typed, screen) = keys_of("100", "question", "2").unwrap();
        assert_eq!(typed, ["down", "enter"]);
        assert!(screen.shows("❯ 1. 전부\n  2. 나눠서", Some(0)));
        assert!(!screen.shows("  1. 전부\n❯ 2. 나눠서", Some(0)));
        assert_eq!(keys_of("99", "question", "2"), Err("stale"));
        assert_eq!(keys_of("100", "permission", "2"), Err("stale"));
        assert_eq!(
            keys(agent(&one), None, "100", "question", "2"),
            Err("stale")
        );
        assert_eq!(keys_of("100", "question", "3"), Err("unknown_choice"));
        assert_eq!(keys_of("100", "question", "allow"), Err("unknown_choice"));
        let several = question(false);
        let shown = of(agent(&several), None).unwrap();
        assert_eq!(shown.frame["answerable"], false);
        assert_eq!(
            keys(agent(&several), Some(&shown), "100", "question", "1"),
            Err("unknown_choice")
        );
    }

    /// An approval the record does not name is tied to the session's one
    /// open call: its command, cut, which the agent's screen must show while
    /// the keys are typed, or its plan. With no such call the card names no
    /// command and is answered in the terminal, except for an agent that
    /// records a call only once it is allowed (Codex); a choice declared for
    /// another kind than the one shown types nothing.
    #[test]
    fn an_approval_is_tied_to_the_one_open_call_and_answered_while_the_screen_shows_it() {
        let waiting = |kind: &str| {
            listed(json!({"agent_kind": kind, "status_code": "approval", "demand": "approval"}))
        };
        let claude = waiting("claude");
        let command = open("toolu_1", Some("cargo test\nTOKEN=x\nmore"), false);
        let shown = of(agent(&claude), Some(&Pending::One(&command))).unwrap();
        assert_eq!(
            shown.frame,
            json!({"kind": "permission", "turn": "100", "since_ms": 100, "call": "toolu_1",
                "command": "cargo test\nTOKEN=x", "command_truncated": true, "answerable": true,
                "choices": [{"id": "allow"}, {"id": "allow_always"}, {"id": "deny"}]})
        );
        // Every key waits for the command's first line, Esc included.
        let (typed, screen) =
            keys(agent(&claude), Some(&shown), "100", "permission", "deny").unwrap();
        assert_eq!(typed, ["esc"]);
        assert!(!screen.shows(" Do you want to proceed?\n ❯ 1. Yes", None));
        assert!(screen.shows("   cargo test\n Do you want to proceed?\n ❯ 1. Yes", None));
        let (_, screen) = keys(agent(&claude), Some(&shown), "100", "permission", "allow").unwrap();
        assert!(screen.shows("   cargo test\n ❯ 1. Yes", Some(0)));
        assert!(!screen.shows("   ls -la\n ❯ 1. Yes", Some(0)));

        let plan = open("toolu_2", None, true);
        let shown = of(agent(&claude), Some(&Pending::One(&plan))).unwrap();
        assert_eq!(
            (
                shown.kind,
                shown.frame["call"].clone(),
                shown.frame["choices"].clone()
            ),
            (
                Ask::PlanApproval,
                json!("toolu_2"),
                json!([{"id": "approve"}, {"id": "keep_planning"}])
            )
        );
        assert_eq!(
            keys(agent(&claude), Some(&shown), "100", "permission", "allow"),
            Err("stale")
        );
        assert!(
            keys(
                agent(&claude),
                Some(&shown),
                "100",
                "plan_approval",
                "approve"
            )
            .is_ok()
        );

        let write = open("toolu_3", None, false);
        for untied in [
            None,
            Some(Pending::Nothing),
            Some(Pending::One(&write)),
            Some(Pending::Unknown { plan: false }),
            Some(Pending::Unknown { plan: true }),
        ] {
            let shown = of(agent(&claude), untied.as_ref()).unwrap();
            assert_eq!(shown.kind, Ask::Permission, "{untied:?}");
            assert_eq!(shown.frame.get("command"), None, "{untied:?}");
            assert_eq!(shown.frame["answerable"], false, "{untied:?}");
            assert_eq!(shown.frame["choices"], json!([]), "{untied:?}");
            assert_eq!(
                keys(agent(&claude), Some(&shown), "100", "permission", "allow"),
                Err("unknown_choice")
            );
        }

        // Codex records the call only once it is allowed: no command, ever.
        let codex = waiting("codex");
        let shown = of(agent(&codex), Some(&Pending::One(&command))).unwrap();
        assert_eq!(
            (
                shown.frame.get("command"),
                shown.frame.get("call"),
                shown.answerable
            ),
            (None, None, true)
        );
        // Esc waits only for its rows, so it never stops what the agent runs.
        let (typed, screen) =
            keys(agent(&codex), Some(&shown), "100", "permission", "deny").unwrap();
        assert_eq!(typed, ["esc"]);
        assert!(screen.shows("  › 1. Yes, proceed (y)", None));
        assert!(!screen.shows("• Running cargo test\n\n› ", None));

        for unread in ["pi", "opencode"] {
            let unread = waiting(unread);
            let shown = of(agent(&unread), Some(&Pending::One(&command))).unwrap();
            assert_eq!(
                (
                    shown.frame["answerable"].clone(),
                    shown.frame["choices"].clone()
                ),
                (json!(false), json!([]))
            );
            assert_eq!(
                keys(agent(&unread), Some(&shown), "100", "permission", "allow"),
                Err("unknown_choice")
            );
        }
        let resting =
            listed(json!({"agent_kind": "claude", "status_code": "idle", "demand": "none"}));
        assert_eq!(of(agent(&resting), Some(&Pending::One(&command))), None);
    }
}
