//! The operator's turn an agent declares when the next step is one only a
//! person can take (PRD ask-human): a sign-in or a permission, and where.
//!
//! The declaration is the agent's own state, kept in the delivery ledger by
//! the agent's registration id (`Ledger::human_turns`), so it outlives a
//! sleep, a wake into another pane and a restart, and it is separate from
//! anything Hide AI reads from the turn (D-07). It goes only on the
//! operator's Done, the operator's own message to the agent, the agent's
//! `--clear`, or the agent's end (D-08).
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{AgentRecord, here};
use crate::delivery::Actor;
use crate::delivery::ledger::Ledger;

/// The two steps only a person can take (D-03); any other word is refused,
/// so a judgement call still goes to the parent as a letter.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HumanKind {
    /// Proving who you are: a sign-in, a re-authentication, an unlock.
    Auth,
    /// Granting access: an app connection, an OS permission.
    Permission,
}

impl HumanKind {
    pub fn parse(word: &str) -> Result<Self, &'static str> {
        match word {
            "auth" => Ok(Self::Auth),
            "permission" => Ok(Self::Permission),
            _ => Err("human_kind_unknown"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auth => "auth",
            Self::Permission => "permission",
        }
    }
}

/// One agent's declaration. `id` names this declaration, not the agent: a
/// different declaration is a new id, so a Done for an earlier one settles
/// nothing and a letter is sent once per declaration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HumanTurn {
    pub id: String,
    pub kind: HumanKind,
    /// Where the person acts, as the agent wrote it on one line.
    pub place: String,
    pub declared_at_unix_ms: u64,
}

/// The longest place, in characters (D-21).
pub const PLACE_LIMIT: usize = 300;

/// The place as it is kept (D-04): as written, a URL's query and fragment
/// included, since they can name what the operator needs (a project, a
/// device code, a request number), on one line. It lives only while the
/// declaration stands and never reaches a letter or the diagnostic log.
pub fn place_of(raw: &str) -> Result<String, &'static str> {
    let place = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if place.is_empty() {
        return Err("human_place_required");
    }
    if place.chars().count() > PLACE_LIMIT {
        return Err("human_place_too_long");
    }
    Ok(place)
}

/// What a declaration did (B1, B5, B6).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// A new declaration; the operator is told.
    Declared,
    /// The same kind and place as the standing one: nothing moved.
    Unchanged,
    /// A different declaration replaced the standing one; the operator is
    /// told again.
    Replaced,
    /// `--clear` took the standing declaration down.
    Cleared,
    /// `--clear` found no declaration to take down. Not a failure.
    NothingToClear,
}

/// What `hide agent ask-human` answers.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
pub struct AskHumanAnswer {
    pub outcome: Outcome,
    /// The caller's registration.
    pub agent: String,
    /// The device the agent runs on, which the operator goes to.
    pub machine: String,
    /// The declaration standing after the command; null once cleared.
    pub declaration: Option<HumanTurn>,
    /// Whether the operator was told: a new or replacing declaration.
    pub operator_notified: bool,
}

/// The caller's declaration: `Some((kind, place))` to declare, `None` to
/// clear. The caller is its own registration (`here`).
pub(crate) fn declare(
    ledger: &mut Ledger,
    caller: &Actor,
    turn: Option<&(HumanKind, String)>,
    now: u64,
) -> Result<Value, String> {
    let record = here(ledger, caller)?.clone();
    let standing = ledger.human_turns.get(&record.id).cloned();
    let (outcome, declaration) = match (turn, standing) {
        (None, Some(_)) => {
            ledger.human_turns.remove(&record.id);
            (Outcome::Cleared, None)
        }
        (None, None) => (Outcome::NothingToClear, None),
        (Some((kind, place)), Some(standing))
            if standing.kind == *kind && standing.place == *place =>
        {
            (Outcome::Unchanged, Some(standing))
        }
        (Some((kind, place)), standing) => {
            let turn = HumanTurn {
                id: format!("{}.{now}", record.id),
                kind: *kind,
                place: place.clone(),
                declared_at_unix_ms: now,
            };
            ledger.human_turns.insert(record.id.clone(), turn.clone());
            let outcome = if standing.is_some() {
                Outcome::Replaced
            } else {
                Outcome::Declared
            };
            (outcome, Some(turn))
        }
    };
    log(
        &record,
        ledger,
        outcome_event(outcome),
        declaration.as_ref(),
    );
    Ok(json!(AskHumanAnswer {
        outcome,
        agent: record.id.clone(),
        machine: record.machine.clone(),
        operator_notified: matches!(outcome, Outcome::Declared | Outcome::Replaced),
        declaration,
    }))
}

/// How the operator settled a declaration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Settled {
    /// The operator pressed Done; the agent gets a letter.
    Done,
    /// The operator sent the agent a message of their own; no letter.
    Spoke,
}

/// Takes down the declaration `id` if it still stands. Only the operator's
/// own identity settles one. Answers the agent whose declaration it was,
/// or null when it no longer stands (a second press, a newer declaration).
pub(crate) fn settle(
    ledger: &mut Ledger,
    caller: &Actor,
    id: &str,
    by: Settled,
) -> Result<Value, String> {
    if !caller.operator_owned() {
        return Err("operator_authority_required".into());
    }
    let Some(agent) = ledger
        .human_turns
        .iter()
        .find(|(_, turn)| turn.id == id)
        .map(|(agent, _)| agent.clone())
    else {
        return Ok(Value::Null);
    };
    let turn = ledger.human_turns.remove(&agent);
    if let Some(record) = ledger.agents.iter().find(|record| record.id == agent) {
        let event = match by {
            Settled::Done => "ask_human.done",
            Settled::Spoke => "ask_human.spoke",
        };
        log(record, ledger, event, turn.as_ref());
    }
    Ok(json!({ "agent": agent }))
}

/// An ended registration declares nothing (D-08).
pub(crate) fn end(ledger: &mut Ledger, id: &str) {
    if let Some(turn) = ledger.human_turns.remove(id)
        && let Some(record) = ledger.agents.iter().find(|record| record.id == id)
    {
        log(record, ledger, "ask_human.ended", Some(&turn));
    }
}

fn outcome_event(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Declared => "ask_human.declared",
        Outcome::Unchanged => "ask_human.unchanged",
        Outcome::Replaced => "ask_human.replaced",
        Outcome::Cleared => "ask_human.cleared",
        Outcome::NothingToClear => "ask_human.nothing_to_clear",
    }
}

/// The agent's lineage root: its farthest live responsibility ancestor,
/// itself when it has none.
fn lineage_root<'a>(ledger: &'a Ledger, record: &'a AgentRecord) -> &'a AgentRecord {
    let mut at = record;
    for _ in 0..ledger.agents.len() {
        let Some(parent) = at.parent.as_deref().and_then(|parent| {
            ledger
                .agents
                .iter()
                .find(|record| record.id == parent && !record.ended)
        }) else {
            break;
        };
        at = parent;
    }
    at
}

/// The structured event for a declaration's step (B29): who, the lineage
/// root, the device and the kind; never the place, which can name an
/// account or a path.
fn log(record: &AgentRecord, ledger: &Ledger, event: &str, turn: Option<&HumanTurn>) {
    crate::diagnostic!(json!({
        "component": "coordination",
        "kind": event,
        "agent_id": record.id,
        "root_id": lineage_root(ledger, record).id,
        "device_id": record.machine,
        "declaration_id": turn.map(|turn| turn.id.as_str()),
        "human_kind": turn.map(|turn| turn.kind.as_str()),
    }));
}

/// The letter Done sends the agent (B18): the kind in the words the
/// operator saw, then what to do in English for any agent. Never the
/// place, since the letter stays after the declaration is gone (D-04).
pub(crate) fn done_letter(turn: &HumanTurn) -> String {
    let word = match turn.kind {
        HumanKind::Auth => "인증",
        HumanKind::Permission => "허용",
    };
    let english = match turn.kind {
        HumanKind::Auth => "sign-in",
        HumanKind::Permission => "permission",
    };
    format!(
        "운영자가 처리함: {word}\nThe operator finished the {english} you asked for with hide agent ask-human. Continue from where you stopped; if it is still not done, declare it again."
    )
}

/// The intent Done's letter is sent under: one letter per declaration
/// however many surfaces press it (B18).
pub(crate) fn done_intent(turn: &HumanTurn) -> String {
    format!("{DONE_INTENT}{}", turn.id)
}

/// Whether a letter's intent is a Done's (`done_intent`).
pub(crate) fn is_done_intent(intent: &str) -> bool {
    intent.starts_with(DONE_INTENT)
}

const DONE_INTENT: &str = "ask-human-done:";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_place_is_kept_as_written_on_one_line() {
        for (raw, kept) in [
            (
                "https://github.com/login/device?user_code=WDJB-MJHT#step=2",
                "https://github.com/login/device?user_code=WDJB-MJHT#step=2",
            ),
            ("x-apple.systempreferences://Privacy?Camera", "x-apple.systempreferences://Privacy?Camera"),
            ("1Password", "1Password"),
            (
                "System Settings › Privacy & Security?\n Camera",
                "System Settings › Privacy & Security? Camera",
            ),
        ] {
            assert_eq!(place_of(raw).unwrap(), kept, "{raw}");
        }
    }

    #[test]
    fn the_done_letter_names_the_kind_and_never_the_place() {
        let turn = HumanTurn {
            id: "agent-1.1".into(),
            kind: HumanKind::Auth,
            place: "https://github.com/login/device?user_code=WDJB-MJHT".into(),
            declared_at_unix_ms: 1,
        };
        let letter = done_letter(&turn);
        assert_eq!(letter.lines().next(), Some("운영자가 처리함: 인증"));
        assert!(!letter.contains("github.com"), "{letter}");
        assert!(!letter.contains("WDJB-MJHT"), "{letter}");
    }

    #[test]
    fn an_empty_or_overlong_place_is_refused() {
        assert_eq!(place_of("  \n "), Err("human_place_required"));
        let long = "a".repeat(PLACE_LIMIT + 1);
        assert_eq!(place_of(&long), Err("human_place_too_long"));
        assert!(place_of(&"가".repeat(PLACE_LIMIT)).is_ok());
    }

    #[test]
    fn only_the_two_kinds_are_taken() {
        assert_eq!(HumanKind::parse("auth"), Ok(HumanKind::Auth));
        assert_eq!(HumanKind::parse("permission"), Ok(HumanKind::Permission));
        for word in ["decision", "login", "Auth", ""] {
            assert_eq!(HumanKind::parse(word), Err("human_kind_unknown"));
        }
    }
}
