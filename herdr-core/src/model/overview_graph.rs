//! Read-only facts for the Overview graph. No letter body or guessed model crosses this boundary.
use serde::Serialize;

pub const GRAPH_LETTER_LIMIT: usize = 2_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphParty {
    pub device_id: String,
    pub pane_id: String,
    pub session: String,
}

/// An independently proven Herdr input submission, not a letter or model receipt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GraphDirectInput {
    pub id: String,
    pub sender: GraphParty,
    pub recipient: GraphParty,
    pub at_unix_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GraphLetter {
    pub id: String,
    pub sender: GraphParty,
    pub recipient: GraphParty,
    pub kind: String,
    pub at_unix_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GraphSession {
    pub device_id: String,
    pub pane_id: String,
    pub session_digest: String,
    /// The explicit choice Hide launched, never a claim about a later native model switch.
    pub launch_model: Option<String>,
    /// The effort of that same launch choice, under the same proof.
    pub launch_effort: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OverviewGraphSnapshot {
    pub available: bool,
    pub limit: usize,
    pub retained_count: usize,
    pub truncated: bool,
    pub retention_ms: u64,
    pub letters: Vec<GraphLetter>,
    pub direct_inputs: Vec<GraphDirectInput>,
    pub sessions: Vec<GraphSession>,
}

impl Default for OverviewGraphSnapshot {
    fn default() -> Self {
        Self {
            available: false,
            limit: GRAPH_LETTER_LIMIT,
            retained_count: 0,
            truncated: false,
            retention_ms: crate::delivery::RETENTION_MS,
            letters: Vec::new(),
            direct_inputs: Vec::new(),
            sessions: Vec::new(),
        }
    }
}

impl OverviewGraphSnapshot {
    /// Both input pages are newest first. Share one budget across relationship kinds.
    pub(crate) fn with_direct_inputs(mut self, mut inputs: Vec<GraphDirectInput>) -> Self {
        let total = self.letters.len() + inputs.len();
        let (mut letters, mut direct) = (0, 0);
        for _ in 0..GRAPH_LETTER_LIMIT.min(total) {
            let take_letter = match (self.letters.get(letters), inputs.get(direct)) {
                (Some(letter), Some(input)) => {
                    (letter.at_unix_ms, &letter.id) >= (input.at_unix_ms, &input.id)
                }
                (Some(_), None) => true,
                _ => false,
            };
            if take_letter {
                letters += 1;
            } else {
                direct += 1;
            }
        }
        self.truncated |= total > GRAPH_LETTER_LIMIT;
        self.letters.truncate(letters);
        inputs.truncate(direct);
        self.direct_inputs = inputs;
        self
    }

    /// Delivery intake/watch transitions do not change the graph's immutable letter facts.
    /// Compare borrowed inputs and the retained count before sorting or cloning the bounded page.
    pub(crate) fn matches_ledger(
        &self,
        previous: &crate::delivery::ledger::Ledger,
        next: &crate::delivery::ledger::Ledger,
        now: u64,
    ) -> bool {
        self.available
            && previous.letters.len() == next.letters.len()
            && next
                .letters
                .iter()
                .filter(|letter| letter.retained(now))
                .count()
                == self.retained_count
            && previous
                .letters
                .iter()
                .zip(&next.letters)
                .all(|(before, after)| {
                    let same_party = |a: &crate::delivery::Actor, b: &crate::delivery::Actor| {
                        a.device_id == b.device_id
                            && a.pane_id == b.pane_id
                            && a.session == b.session
                    };
                    before.id == after.id
                        && before.kind == after.kind
                        && before.created_at_unix_ms == after.created_at_unix_ms
                        && same_party(&before.sender, &after.sender)
                        && same_party(&before.recipient, &after.recipient)
                })
    }

    /// Called on an accepted delivery revision, never on a terminal key or snapshot read.
    /// The ledger itself is capped at 5,000; only the newest retained 2,000 are copied.
    pub(crate) fn from_ledger(ledger: &crate::delivery::ledger::Ledger, now: u64) -> Self {
        let mut retained: Vec<_> = ledger
            .letters
            .iter()
            .filter(|letter| letter.retained(now))
            .collect();
        retained.sort_unstable_by(|a, b| {
            b.created_at_unix_ms
                .cmp(&a.created_at_unix_ms)
                .then_with(|| b.id.cmp(&a.id))
        });
        let retained_count = retained.len();
        let party = |actor: &crate::delivery::Actor| {
            Some(GraphParty {
                device_id: actor.device_id.clone(),
                pane_id: actor.pane_id.clone(),
                session: actor.session.clone()?,
            })
        };
        let letters = retained
            .into_iter()
            .take(GRAPH_LETTER_LIMIT)
            .filter_map(|letter| {
                Some(GraphLetter {
                    id: letter.id.clone(),
                    sender: party(&letter.sender)?,
                    recipient: party(&letter.recipient)?,
                    kind: letter.kind.clone(),
                    at_unix_ms: letter.created_at_unix_ms,
                })
            })
            .collect();
        Self {
            available: true,
            retained_count,
            truncated: retained_count > GRAPH_LETTER_LIMIT,
            letters,
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::ledger::{Ledger, Letter};

    fn letter(id: u64, at: u64) -> Letter {
        serde_json::from_value(serde_json::json!({
            "id": format!("letter-{id}"), "intent": format!("intent-{id}"),
            "sender": { "device_id": "device-a", "pane_id": "pane-a", "session": "digest-a", "name": "private sender", "kind": "claude" },
            "recipient": { "device_id": "device-b", "pane_id": "remote:device-b:pane:pane-b", "session": "digest-b", "name": "private recipient", "kind": "codex" },
            "kind": "progress", "body": "private letter content", "state": "acknowledged", "waiting_answer": false,
            "reply_to": null, "created_at_unix_ms": at, "finished_at_unix_ms": at,
            "bell_errors": 0, "bell_sent": false
        })).unwrap()
    }

    #[test]
    fn publishes_only_newest_retained_metadata_and_never_extends_retention() {
        let now = crate::delivery::RETENTION_MS + 5_000;
        let mut ledger = Ledger {
            letters: (0..2_001).map(|id| letter(id, now - 2_001 + id)).collect(),
            ..Ledger::default()
        };
        ledger
            .letters
            .push(letter(9_999, now - crate::delivery::RETENTION_MS));
        let graph = OverviewGraphSnapshot::from_ledger(&ledger, now);
        assert_eq!(graph.retained_count, 2_001);
        assert_eq!(graph.letters.len(), 2_000);
        assert!(graph.truncated);
        assert_eq!(graph.letters.first().unwrap().id, "letter-2000");
        assert!(
            !graph
                .letters
                .iter()
                .any(|letter| letter.id == "letter-0" || letter.id == "letter-9999")
        );
        assert_eq!(graph.retention_ms, crate::delivery::RETENTION_MS);
        assert_eq!(ledger.letters.len(), 2_002);
        let wire = serde_json::to_value(&graph).unwrap();
        let first = wire["letters"][0].as_object().unwrap();
        assert_eq!(first.len(), 5);
        assert!(!wire.to_string().contains("private"));
        assert_eq!(wire["letters"][0]["recipient"]["device_id"], "device-b");
        assert_eq!(wire["letters"][0]["recipient"]["session"], "digest-b");
    }

    #[test]
    fn unproven_parties_have_no_graph_correspondence_but_open_letters_keep_existing_retention() {
        let mut unproven = letter(1, 1);
        unproven.sender.session = None;
        let mut open = letter(2, 2);
        open.state = crate::delivery::ledger::State::Pending;
        open.finished_at_unix_ms = None;
        let ledger = Ledger {
            letters: vec![unproven, open],
            ..Ledger::default()
        };
        let graph = OverviewGraphSnapshot::from_ledger(&ledger, crate::delivery::RETENTION_MS * 2);
        assert_eq!(graph.letters.len(), 1);
        assert_eq!(graph.letters[0].id, "letter-2");
        assert!(ledger.letters[1].open());
    }

    #[test]
    fn intake_transitions_reuse_metadata_but_expiration_and_session_changes_invalidate_it() {
        let before = Ledger {
            letters: vec![letter(1, 1)],
            ..Ledger::default()
        };
        let graph = OverviewGraphSnapshot::from_ledger(&before, 2);
        let mut after = before.clone();
        after.letters[0].hook_confirmed = Some(true);
        after.letters[0].bell_sent = true;
        assert!(graph.matches_ledger(&before, &after, 3));
        assert!(!graph.matches_ledger(&before, &after, crate::delivery::RETENTION_MS + 1));
        after.letters[0].recipient.session = Some("replacement-session".into());
        assert!(!graph.matches_ledger(&before, &after, 3));
    }

    #[test]
    fn letters_and_direct_inputs_share_the_newest_metadata_budget() {
        let ledger = Ledger {
            letters: (0..2_000).map(|id| letter(id, id)).collect(),
            ..Ledger::default()
        };
        let input = GraphDirectInput {
            id: "direct".into(),
            sender: GraphParty {
                device_id: "device-a".into(),
                pane_id: "pane-a".into(),
                session: "digest-a".into(),
            },
            recipient: GraphParty {
                device_id: "device-b".into(),
                pane_id: "pane-b".into(),
                session: "digest-b".into(),
            },
            at_unix_ms: 2_000,
        };
        let graph =
            OverviewGraphSnapshot::from_ledger(&ledger, 2_000).with_direct_inputs(vec![input]);
        assert_eq!(
            graph.letters.len() + graph.direct_inputs.len(),
            GRAPH_LETTER_LIMIT
        );
        assert_eq!(graph.direct_inputs.len(), 1);
        assert_eq!(graph.letters.len(), 1_999);
        assert!(!graph.letters.iter().any(|letter| letter.id == "letter-0"));
        assert!(graph.truncated);
        assert!(graph.matches_ledger(&ledger, &ledger, 2_000));
        // Rebuilding from retained sources restores a letter displaced by an input.
        let graph = OverviewGraphSnapshot::from_ledger(&ledger, 2_001).with_direct_inputs(vec![]);
        assert_eq!(graph.letters.len(), GRAPH_LETTER_LIMIT);
        assert!(!graph.truncated);
    }
}
