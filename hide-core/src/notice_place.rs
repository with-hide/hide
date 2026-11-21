//! The one place a notification goes (PRD device-connect-iroh D-60, D-78).
//!
//! The core that answers for a piece of work judges its notifications
//! (`whose_work::Reach::notifies`), and each one goes to one place: this
//! machine while someone uses it, else the computer in use that may open
//! this machine, the one used last when there are several, else the phone.
//! A computer that is on and not in use gets nothing. [`places`] is that
//! order; whoever sends walks it and moves on when a computer does not say
//! within [`TAKEN_WITHIN`] that it took the notice (B71).
//!
//! "In use" is the desktop host's to observe (keyboard or mouse input in
//! the last ten minutes, unlocked and awake, `docs/UI_BEHAVIOR.md`,
//! Notifications), so a core with no app is never in use. Every input here
//! is a length of time, never a clock reading: two machines' clocks are not
//! compared.

use std::time::Duration;

/// How long the computer a notice was sent to has to say it took it before
/// the next place is tried.
pub const TAKEN_WITHIN: Duration = Duration::from_secs(5);

/// A phone's action counts as later than a computer's input only when it
/// came more than this after it: the desktop host reads input once a
/// second, so inside that second the computer wins and the ambiguous case
/// stays quiet.
pub const COMPUTER_WINS_WITHIN: Duration = Duration::from_secs(1);

/// A computer someone may sit at.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Seat {
    /// How long ago its last reported input was while it is in use; `None`
    /// while it is not (idle past ten minutes, locked, asleep, or no app).
    pub input: Option<Duration>,
    /// Whether a window there shows the operator's turn: one that has its
    /// notifications on. A computer in use that shows nothing cannot stand
    /// in for the phone.
    pub shows: bool,
    /// How long ago a phone paired with that machine last acted, when one
    /// did. The operator who answered from a phone is at the phone until
    /// the next computer input.
    pub phone: Option<Duration>,
}

/// Another computer that may open this one, connected now.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Peer {
    /// Its connection id.
    pub key: String,
    pub seat: Seat,
}

/// Where a notification goes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Place {
    /// This machine's own windows.
    Here,
    /// The computer under this connection id.
    Peer(String),
    /// The phones paired with this machine.
    Phone,
}

/// The places to try for one notification, in order (D-60). Empty when no
/// one is at a computer and no phone is paired: the row still stands in
/// Needs You.
pub fn places(here: Seat, peers: &[Peer], phone_paired: bool) -> Vec<Place> {
    // The latest phone action any of the machines knows of: whoever is at
    // a phone is at no computer.
    let phone = peers
        .iter()
        .filter_map(|peer| peer.seat.phone)
        .chain(here.phone)
        .min();
    let at = |seat: &Seat| {
        seat.input.filter(|input| {
            seat.shows && phone.is_none_or(|phone| *input <= phone + COMPUTER_WINS_WITHIN)
        })
    };
    let mut order = Vec::new();
    if at(&here).is_some() {
        // Someone is here: nothing falls through to another place.
        order.push(Place::Here);
        return order;
    }
    let mut seated: Vec<(Duration, &str)> = peers
        .iter()
        .filter_map(|peer| at(&peer.seat).map(|input| (input, peer.key.as_str())))
        .collect();
    seated.sort();
    order.extend(
        seated
            .into_iter()
            .map(|(_, key)| Place::Peer(key.to_owned())),
    );
    if phone_paired {
        order.push(Place::Phone);
    }
    order
}

/// Whether the phones rest for a notification made now: some computer
/// takes it first.
pub fn phone_rests(order: &[Place]) -> bool {
    !matches!(order.first(), None | Some(Place::Phone))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    fn used(seconds: u64) -> Seat {
        Seat {
            input: Some(secs(seconds)),
            shows: true,
            phone: None,
        }
    }

    const AWAY: Seat = Seat {
        input: None,
        shows: true,
        phone: None,
    };

    fn peer(key: &str, seat: Seat) -> Peer {
        Peer {
            key: key.to_owned(),
            seat,
        }
    }

    /// B59: the machine that owns the work while it is in use, and nowhere
    /// else then.
    #[test]
    fn a_machine_in_use_takes_its_own_notices_and_nothing_else_rings() {
        let order = places(used(3), &[peer("mac", used(1))], true);
        assert_eq!(order, [Place::Here]);
        assert!(phone_rests(&order));
    }

    /// B59: else one computer in use that may open it, the one used last
    /// first, and the phone after them.
    #[test]
    fn an_idle_owner_hands_the_notice_to_the_computer_used_last_then_the_phone() {
        let order = places(
            AWAY,
            &[
                peer("mini", used(40)),
                peer("mac", used(2)),
                peer("old", AWAY),
            ],
            true,
        );
        assert_eq!(
            order,
            [
                Place::Peer("mac".into()),
                Place::Peer("mini".into()),
                Place::Phone
            ]
        );
        assert!(phone_rests(&order));
    }

    /// B59: a computer that is on and has had no input for ten minutes is
    /// not in use, so the phone rings; with no phone paired nothing does.
    #[test]
    fn with_no_computer_in_use_the_phone_rings_and_without_a_phone_nothing_does() {
        let order = places(AWAY, &[peer("mac", AWAY)], true);
        assert_eq!(order, [Place::Phone]);
        assert!(!phone_rests(&order));
        let order = places(AWAY, &[peer("mac", AWAY)], false);
        assert!(order.is_empty());
        assert!(!phone_rests(&order));
    }

    /// The rule the Mac and the phone already had, kept for every computer
    /// (D-60): a computer in use that shows no notice cannot stand in for
    /// the phone.
    #[test]
    fn a_computer_that_shows_no_notice_is_passed_over() {
        let silent = Seat {
            shows: false,
            ..used(1)
        };
        assert_eq!(places(silent, &[], true), [Place::Phone]);
        assert_eq!(
            places(silent, &[peer("mac", silent), peer("mini", used(9))], true),
            [Place::Peer("mini".into()), Place::Phone]
        );
    }

    /// D-60: an action on a phone wins until the next computer input, on
    /// whichever machine the phone acted; inside the host's one-second
    /// read the computer wins.
    #[test]
    fn a_later_phone_action_takes_every_computer_until_the_next_input() {
        let answered_on_phone = Seat {
            phone: Some(secs(4)),
            ..used(10)
        };
        assert_eq!(places(answered_on_phone, &[], true), [Place::Phone]);
        // The phone acted on another machine: still no computer.
        let other = peer(
            "mac",
            Seat {
                phone: Some(secs(4)),
                ..used(30)
            },
        );
        assert_eq!(
            places(used(10), std::slice::from_ref(&other), true),
            [Place::Phone]
        );
        // Input after the phone's action takes it back.
        assert_eq!(places(used(2), &[other], true), [Place::Here]);
        // One second is one read of the host: the computer wins the tie.
        let tie = Seat {
            phone: Some(secs(9)),
            ..used(10)
        };
        assert_eq!(places(tie, &[], true), [Place::Here]);
        let later = Seat {
            phone: Some(Duration::from_millis(8_900)),
            ..used(10)
        };
        assert_eq!(places(later, &[], true), [Place::Phone]);
    }
}
