//! What one walk of a published ledger's letters says against the ledger it
//! replaces (with-hide/hide-private#78).
//!
//! A publish asks three things of the letters: whether the Overview graph's
//! immutable letter facts moved, which letters turned Delivered since the
//! last publish, and whether who waits for a reply, from whom, moved. Each was
//! its own pass, and the second looked every Delivered letter up in the
//! previous ledger by scanning it, so a ledger of thousands of delivered
//! letters cost millions of comparisons under the runtime lock per publish.
//! One walk over the new letters answers all three: the previous ledger is read
//! at the same position, and only a letter that moved is looked up, in an index
//! built once and only when one did.

use std::collections::{BTreeSet, HashMap};

use crate::delivery::ledger::{Ledger, Letter, State};
use crate::model::OverviewGraphSnapshot;

/// Who waits for a reply: the sender, the recipient, and whether the recipient
/// took the letter in. Only a change in this moves a row's wait.
type Waiter = ((String, Option<String>), (String, Option<String>), bool);

/// What the walk found.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct LetterWalk {
    /// The Overview graph's page of letters still says what the ledger's
    /// letters do: the same letters in the same order with the same facts, and
    /// as many of them still retained (a delivery step or a receipt moves none
    /// of it, a session change, a letter leaving or a retention expiry does).
    pub(super) graph_unchanged: bool,
    /// The recipient pane of each letter that is Delivered now and was in
    /// another state in the previous ledger.
    pub(super) delivered: Vec<String>,
    /// The set of waiting senders and recipients differs from the previous
    /// ledger's.
    pub(super) waiters_moved: bool,
}

fn waiter(letter: &Letter) -> Waiter {
    (
        (letter.sender.pane_id.clone(), letter.sender.session.clone()),
        (
            letter.recipient.pane_id.clone(),
            letter.recipient.session.clone(),
        ),
        letter.awaiting_intake(),
    )
}

/// The facts of a letter the graph draws; a state or a receipt moves none.
fn same_graph_facts(before: &Letter, after: &Letter) -> bool {
    let same_party = |a: &crate::delivery::Actor, b: &crate::delivery::Actor| {
        a.device_id == b.device_id && a.pane_id == b.pane_id && a.session == b.session
    };
    before.id == after.id
        && before.kind == after.kind
        && before.created_at_unix_ms == after.created_at_unix_ms
        && same_party(&before.sender, &after.sender)
        && same_party(&before.recipient, &after.recipient)
}

/// Walks `next`'s letters once against `previous`. With no previous ledger
/// (it was unavailable) everything has moved and nothing is newly delivered.
pub(super) fn walk(
    previous: Option<&Ledger>,
    next: &Ledger,
    graph: &OverviewGraphSnapshot,
    now: u64,
) -> LetterWalk {
    let Some(previous) = previous else {
        return LetterWalk {
            graph_unchanged: false,
            delivered: Vec::new(),
            waiters_moved: true,
        };
    };
    let mut graph_unchanged = graph.available && previous.letters.len() == next.letters.len();
    let mut retained = 0usize;
    let mut delivered = Vec::new();
    // The previous ledger's states by id, built when the first letter that
    // is Delivered now is not at its old position.
    let mut states: Option<HashMap<&str, State>> = None;
    let mut waiting: BTreeSet<Waiter> = BTreeSet::new();
    for (position, letter) in next.letters.iter().enumerate() {
        let before = previous.letters.get(position);
        if letter.retained(now) {
            retained += 1;
        }
        if graph_unchanged {
            graph_unchanged = before.is_some_and(|before| same_graph_facts(before, letter));
        }
        if letter.waiting_answer {
            waiting.insert(waiter(letter));
        }
        if letter.state == State::Delivered {
            let was = match before.filter(|before| before.id == letter.id) {
                Some(before) => Some(before.state),
                None => states
                    .get_or_insert_with(|| {
                        previous
                            .letters
                            .iter()
                            .map(|old| (old.id.as_str(), old.state))
                            .collect()
                    })
                    .get(letter.id.as_str())
                    .copied(),
            };
            if was.is_some_and(|was| was != State::Delivered) {
                delivered.push(letter.recipient.pane_id.clone());
            }
        }
    }
    // The previous ledger's waiting pairs are read last, from its flags alone.
    let waiters_moved = previous
        .letters
        .iter()
        .filter(|letter| letter.waiting_answer)
        .map(waiter)
        .collect::<BTreeSet<_>>()
        != waiting;
    LetterWalk {
        graph_unchanged: graph_unchanged && retained == graph.retained_count,
        delivered,
        waiters_moved,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::{Actor, mailbox};

    const NOW: u64 = 10_000;

    fn actor(name: &str) -> Actor {
        Actor {
            pane_id: name.into(),
            name: name.into(),
            kind: "claude".into(),
            device_id: "local".into(),
            session: Some(format!("{name}-session")),
        }
    }

    /// A ledger of `count` letters from `sender-N` to `recipient-N`, each
    /// Pending and none waiting for an answer (a request waits by default), and
    /// the graph that describes it.
    fn ledger(count: usize) -> (Ledger, OverviewGraphSnapshot) {
        let mut ledger = Ledger::default();
        for n in 0..count {
            mailbox::send(
                &mut ledger,
                &actor(&format!("sender-{n}")),
                &actor(&format!("recipient-{n}")),
                &format!("intent-{n}"),
                "body",
                "request",
                None,
                1,
            )
            .unwrap();
        }
        ledger
            .letters
            .iter_mut()
            .for_each(|letter| letter.waiting_answer = false);
        let graph = OverviewGraphSnapshot::from_ledger(&ledger, NOW);
        (ledger, graph)
    }

    /// What the earlier passes answered, one scan each: the oracle the walk
    /// must agree with.
    fn passes(
        previous: &Ledger,
        next: &Ledger,
        graph: &OverviewGraphSnapshot,
    ) -> (bool, Vec<String>, bool) {
        let waiters = |ledger: &Ledger| {
            ledger
                .letters
                .iter()
                .filter(|letter| letter.waiting_answer)
                .map(waiter)
                .collect::<BTreeSet<_>>()
        };
        (
            // The earlier pass: the graph's facts, letter by letter, and its count.
            graph.available
                && previous.letters.len() == next.letters.len()
                && next
                    .letters
                    .iter()
                    .filter(|letter| letter.retained(NOW))
                    .count()
                    == graph.retained_count
                && previous
                    .letters
                    .iter()
                    .zip(&next.letters)
                    .all(|(before, after)| same_graph_facts(before, after)),
            next.letters
                .iter()
                .filter(|letter| {
                    letter.state == State::Delivered
                        && previous
                            .letters
                            .iter()
                            .any(|old| old.id == letter.id && old.state != letter.state)
                })
                .map(|letter| letter.recipient.pane_id.clone())
                .collect(),
            waiters(previous) != waiters(next),
        )
    }

    fn walked(
        previous: &Ledger,
        next: &Ledger,
        graph: &OverviewGraphSnapshot,
    ) -> (bool, Vec<String>, bool) {
        let walk = walk(Some(previous), next, graph, NOW);
        (walk.graph_unchanged, walk.delivered, walk.waiters_moved)
    }

    #[test]
    fn a_ledger_that_did_not_move_says_nothing_moved() {
        let (ledger, graph) = ledger(6);
        assert_eq!(
            walked(&ledger, &ledger.clone(), &graph),
            (true, vec![], false)
        );
    }

    #[test]
    fn a_letter_that_turned_delivered_names_its_recipient_wherever_it_sits() {
        let (before, graph) = ledger(6);
        let mut after = before.clone();
        after.letters[4].state = State::Delivered;
        after.letters[1].state = State::Delivered;
        let (unchanged, delivered, _) = walked(&before, &after, &graph);
        assert!(unchanged, "a state moves no fact the graph draws");
        assert_eq!(delivered, ["recipient-1", "recipient-4"]);
        // A letter that was Delivered already is not newly delivered.
        let (_, delivered, _) = walked(&after, &after.clone(), &graph);
        assert!(delivered.is_empty());
    }

    #[test]
    fn a_letter_that_moved_in_the_vector_is_still_found_by_its_id() {
        let (before, graph) = ledger(6);
        let mut after = before.clone();
        // The first letter left the ledger: every later one sits one place up.
        after.letters.remove(0);
        after.letters[2].state = State::Delivered;
        let (unchanged, delivered, _) = walked(&before, &after, &graph);
        assert!(!unchanged, "a letter left the graph's page");
        assert_eq!(delivered, ["recipient-3"]);
        assert_eq!(
            walked(&before, &after, &graph),
            passes(&before, &after, &graph)
        );
    }

    #[test]
    fn a_letter_the_previous_ledger_never_had_is_not_newly_delivered() {
        let (before, graph) = ledger(3);
        let (mut after, _) = ledger(3);
        // Same positions, other ids: a ledger replaced rather than moved.
        after.letters.iter_mut().for_each(|letter| {
            letter.id.push_str("-again");
            letter.state = State::Delivered;
        });
        assert!(walked(&before, &after, &graph).1.is_empty());
    }

    #[test]
    fn a_waiting_pair_that_appears_or_ends_moves_the_waiters_and_a_step_of_one_does_not() {
        let (mut before, graph) = ledger(4);
        before.letters[0].waiting_answer = true;
        let mut after = before.clone();
        // A waiting letter's own state steps, its recipient not yet taking it in
        // staying the same.
        after.letters[0].bell_sent = true;
        assert!(!walked(&before, &after, &graph).2);
        // The pair is joined by a second one.
        after.letters[2].waiting_answer = true;
        assert!(walked(&before, &after, &graph).2);
        // And one that ends.
        assert!(walked(&after, &before, &graph).2);
        // Two letters of one pair: the set holds one, so the second changes nothing.
        let mut twin = before.clone();
        let mut copy = twin.letters[0].clone();
        copy.id = "twin".into();
        twin.letters.push(copy);
        assert_eq!(
            walked(&before, &twin, &graph).2,
            passes(&before, &twin, &graph).2
        );
    }

    #[test]
    fn a_letter_that_gains_the_intake_receipt_moves_the_waiters() {
        let (mut before, graph) = ledger(2);
        before.letters[0].waiting_answer = true;
        let mut after = before.clone();
        // Pending awaits intake; Acknowledged with no hook confirmation still does, one with it does not.
        after.letters[0].state = State::Acknowledged;
        after.letters[0].hook_confirmed = Some(true);
        assert_eq!(
            walked(&before, &after, &graph).2,
            passes(&before, &after, &graph).2
        );
        assert!(walked(&before, &after, &graph).2);
    }

    #[test]
    fn the_walk_agrees_with_the_passes_it_replaced_over_every_kind_of_step() {
        let (before, graph) = ledger(8);
        type Step = Box<dyn Fn(&mut Ledger)>;
        let steps: Vec<Step> = vec![
            Box::new(|ledger| ledger.letters[3].state = State::Delivered),
            Box::new(|ledger| ledger.letters[3].waiting_answer = true),
            Box::new(|ledger| {
                ledger.letters.remove(2);
            }),
            Box::new(|ledger| {
                let mut letter = ledger.letters[0].clone();
                letter.id = "extra".into();
                letter.created_at_unix_ms += 1;
                ledger.letters.push(letter);
            }),
            Box::new(|ledger| ledger.letters[5].kind = "report".into()),
            Box::new(|ledger| ledger.letters[6].recipient.session = None),
            Box::new(|ledger| ledger.letters.swap(1, 2)),
            Box::new(|ledger| ledger.letters[7].finished_at_unix_ms = Some(1)),
        ];
        for (n, step) in steps.iter().enumerate() {
            let mut after = before.clone();
            step(&mut after);
            assert_eq!(
                walked(&before, &after, &graph),
                passes(&before, &after, &graph),
                "step {n}"
            );
        }
    }

    #[test]
    fn a_receipt_reuses_the_graph_but_a_session_change_or_an_expiry_does_not() {
        let (mut before, _) = ledger(1);
        before.letters[0].state = State::Acknowledged;
        before.letters[0].finished_at_unix_ms = Some(1);
        let graph = OverviewGraphSnapshot::from_ledger(&before, 2);
        let mut after = before.clone();
        after.letters[0].hook_confirmed = Some(true);
        after.letters[0].bell_sent = true;
        assert!(walk(Some(&before), &after, &graph, 3).graph_unchanged);
        // The letter outlived its retention: the page holds one the ledger no longer keeps.
        assert!(
            !walk(
                Some(&before),
                &after,
                &graph,
                crate::delivery::RETENTION_MS + 1
            )
            .graph_unchanged
        );
        after.letters[0].recipient.session = Some("replacement-session".into());
        assert!(!walk(Some(&before), &after, &graph, 3).graph_unchanged);
    }

    #[test]
    fn with_no_previous_ledger_everything_has_moved_and_nothing_is_delivered() {
        let (mut ledger, graph) = ledger(2);
        ledger.letters[0].state = State::Delivered;
        assert_eq!(
            walk(None, &ledger, &graph, NOW),
            LetterWalk {
                graph_unchanged: false,
                delivered: vec![],
                waiters_moved: true
            }
        );
    }
}
