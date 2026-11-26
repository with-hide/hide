//! What the operator can do about letters the doorbell holds: a reading of
//! the holds the doorbell publishes (`Runtime::publish_delivery_holds`),
//! never a verdict of its own, so a mission's map and 내 차례 say why its
//! Conductor's letters wait and how to let them through
//! (with-hide/hide-private#678). The doorbell decides each hold and its
//! clocks; this only names the operator's part in one.

use std::collections::BTreeMap;

use serde::Serialize;

use super::Actor;
use super::doorbell::{Hold, WAITING_MARK_REST_MS};
use super::ledger::{Ledger, State};

/// What the operator does to let held letters through, for the holds only
/// the operator ends, in the order one is named when a recipient's letters
/// wait for both (PRD letters-missions-simplify D-05, B14). A pane Hide
/// cannot reach, another conversation in it, or bells the agent did not
/// take are Hide's to settle (the agent's end hands its letters on, a wake
/// or a later turn takes them), never the operator's.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HeldFor {
    /// The agent waits for the operator's approval on a menu or a plan.
    Approval,
    /// Text that may be the operator's stands unsent in its input.
    Draft,
}

/// The operator's part in `hold`, none for a hold that ends on its own
/// (work, the quiet period, a launch, a moved state, an agent that left its
/// pane, bells not taken, a progress letter riding the next handover) or
/// that only another build would end (a kind that takes no bell). Every hold is named, so a hold the doorbell adds is
/// placed here before it builds.
pub(crate) fn held_for(hold: Hold) -> Option<HeldFor> {
    match hold {
        Hold::Blocked | Hold::AwaitingOperator => Some(HeldFor::Approval),
        Hold::Draft => Some(HeldFor::Draft),
        Hold::Absent
        | Hold::Session
        | Hold::Exhausted
        | Hold::Kind
        | Hold::Working
        | Hold::SessionUnread
        | Hold::Status
        | Hold::Quiet
        | Hold::NotReady
        | Hold::Unprojected
        | Hold::Announced
        | Hold::Launching
        | Hold::Identity
        | Hold::Moved
        | Hold::Input
        | Hold::Letter
        | Hold::ComposerUnread
        | Hold::Rides => None,
    }
}

/// The letters waiting for one recipient that only the operator can let
/// through: how many, what the operator does (the first `HeldFor` among
/// them) and when the oldest arrived.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HeldLetters {
    pub count: u32,
    pub reason: HeldFor,
    pub since_unix_ms: u64,
}

/// The letters `recipient` has not got that only the operator can let
/// through: each pending one whose last published hold is the operator's to
/// lift and that arrived `WAITING_MARK_REST_MS` or more before `now`, the
/// two minutes a pane's waiting mark rests, so a hold a pass or two clears
/// never shows. Also when the next pending one comes of that age, so a
/// view made before then is made again. Memory only, bounded by the ledger.
pub(crate) fn letters_held(
    ledger: &Ledger,
    holds: &BTreeMap<String, Hold>,
    recipient: &Actor,
    now: u64,
) -> (Option<HeldLetters>, Option<u64>) {
    let mut held: Option<HeldLetters> = None;
    let mut next = None::<u64>;
    for letter in &ledger.letters {
        if letter.state != State::Pending || !letter.recipient.same_identity(recipient) {
            continue;
        }
        let Some(reason) = holds.get(&letter.id).copied().and_then(held_for) else {
            continue;
        };
        let due = letter
            .created_at_unix_ms
            .saturating_add(WAITING_MARK_REST_MS);
        if now < due {
            next = Some(next.map_or(due, |next: u64| next.min(due)));
            continue;
        }
        let since = letter.created_at_unix_ms;
        held = Some(match held {
            None => HeldLetters {
                count: 1,
                reason,
                since_unix_ms: since,
            },
            Some(held) => HeldLetters {
                count: held.count + 1,
                reason: held.reason.min(reason),
                since_unix_ms: held.since_unix_ms.min(since),
            },
        });
    }
    (held, next)
}
