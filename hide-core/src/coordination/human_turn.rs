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
    /// Where the person acts, cut of a URL's query and fragment.
    pub place: String,
    pub declared_at_unix_ms: u64,
}

/// The longest place, in characters, after it is cut (D-21).
pub const PLACE_LIMIT: usize = 300;

/// The place as it is kept: one line, each word the `url` crate reads as a
/// URL without its user, password, query and fragment, which can carry
/// one-time tokens that would otherwise reach the screen, a notification
/// and the phone (D-04), and an `http` or `https` one without its scheme.
/// Every other word is kept as written.
pub fn place_of(raw: &str) -> Result<String, &'static str> {
    let place = raw
        .split_whitespace()
        .map(|word| url_word(word).unwrap_or_else(|| word.to_owned()))
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if place.is_empty() {
        return Err("human_place_required");
    }
    if place.chars().count() > PLACE_LIMIT {
        return Err("human_place_too_long");
    }
    Ok(place)
}

/// A word as its URL is kept, or None to keep the word as written. A word
/// with no scheme is read as `https://<word>` only when that names a dotted
/// host or an address followed by a path, query or fragment
/// (`github.com/login?…`), so an email address or a bare host stays as
/// written, and any word is kept as written when it carries nothing to
/// remove.
fn url_word(word: &str) -> Option<String> {
    // A word read with no host (`user:pass@host.com/x` as the scheme `user`)
    // is read as a bare host first, which finds its user and password.
    let direct = match url::Url::parse(word) {
        // A URL with no host keeps nothing a person could go to.
        Err(url::ParseError::EmptyHost) => return Some(String::new()),
        parsed => parsed.ok(),
    };
    let (mut url, given) = match direct {
        Some(url) if url.has_host() => (url, true),
        direct => match (bare_url(word), direct) {
            (Some(url), _) => (url, false),
            (None, Some(url)) => (url, true),
            (None, None) => return None,
        },
    };
    let http = matches!(url.scheme(), "http" | "https");
    let secret = !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some();
    if !secret && !(given && http) {
        return None;
    }
    // A URL that cannot be a base has no user to take away.
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    let kept = url.as_str();
    let kept = if http {
        kept.split_once("://").map_or(kept, |(_, rest)| rest)
    } else {
        kept
    };
    Some(kept.trim_end_matches('/').to_owned())
}

/// `word` read as `https://<word>`, when that names a dotted host or an
/// address followed by a path, query or fragment.
fn bare_url(word: &str) -> Option<url::Url> {
    let url = url::Url::parse(&format!("https://{word}")).ok()?;
    if url.path() == "/" && url.query().is_none() && url.fragment().is_none() {
        return None;
    }
    match url.host()? {
        url::Host::Domain(domain) if domain.contains('.') => Some(url),
        url::Host::Ipv4(_) | url::Host::Ipv6(_) => Some(url),
        url::Host::Domain(_) => None,
    }
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

/// The first line of the letter Done sends the agent (B18), in the words
/// the operator saw, then what to do in English for any agent.
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
        "운영자가 처리함: {word} · {place}\nThe operator finished the {english} at {place} that you asked for with hide agent ask-human. Continue from where you stopped; if it is still not done, declare it again.",
        place = turn.place
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
    fn a_url_place_keeps_neither_its_query_nor_its_fragment() {
        assert_eq!(
            place_of("https://github.com/login/oauth?code=abc&state=x#token=y").unwrap(),
            "github.com/login/oauth"
        );
        assert_eq!(
            place_of("github.com/login?return_to=%2F").unwrap(),
            "github.com/login"
        );
        assert_eq!(
            place_of("https://accounts.example.com/#/signin").unwrap(),
            "accounts.example.com"
        );
        assert_eq!(
            place_of("x-apple.systempreferences://Privacy?Camera").unwrap(),
            "x-apple.systempreferences://Privacy"
        );
        // A port, a machine name, an address and a user and password are a
        // URL's too, and a URL inside a sentence keeps nothing secret either.
        for (raw, kept) in [
            ("localhost:3000/cb?code=1", "localhost:3000/cb"),
            ("example.com:8443/cb?code=abc", "example.com:8443/cb"),
            ("192.168.1.5:8080/x?t=1", "192.168.1.5:8080/x"),
            (
                "https://user:token@host.example/path?x=1",
                "host.example/path",
            ),
            ("http://localhost:8080/#/login", "localhost:8080"),
            (
                "Sign in at https://github.com/login?return_to=secret then",
                "Sign in at github.com/login then",
            ),
            ("[::1]:3000/cb?t=1", "[::1]:3000/cb"),
            ("my_host.internal:9000/cb?t=1", "my_host.internal:9000/cb"),
            ("url=github.com/login?t=1", "url=github.com/login"),
            ("at:localhost:3000/cb?t=1#f", "at:localhost:3000/cb"),
            ("me@example.com/x?t=1", "example.com/x"),
            ("user:pass@host.example/x", "host.example/x"),
            ("mailto:me@example.com", "mailto:me@example.com"),
            // Words that are no URL are kept as written.
            ("Privacy & Security > Camera", "Privacy & Security > Camera"),
            ("src/lib.rs#L10 README.md", "src/lib.rs#L10 README.md"),
            ("Sign in as me@example.com", "Sign in as me@example.com"),
            ("example.com:8443", "example.com:8443"),
        ] {
            assert_eq!(place_of(raw).unwrap(), kept, "{raw}");
        }
    }

    #[test]
    fn a_place_that_is_not_a_url_is_kept_as_written_on_one_line() {
        assert_eq!(place_of("1Password").unwrap(), "1Password");
        assert_eq!(
            place_of("System Settings › Privacy & Security?\n Camera").unwrap(),
            "System Settings › Privacy & Security? Camera"
        );
        assert_eq!(place_of("v1.2 release").unwrap(), "v1.2 release");
    }

    #[test]
    fn an_empty_or_overlong_place_is_refused() {
        assert_eq!(place_of("  \n "), Err("human_place_required"));
        assert_eq!(place_of("https://?code=1"), Err("human_place_required"));
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
