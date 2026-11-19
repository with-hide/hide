//! Whose a piece of work is and where it runs, between machines that each
//! run Hide (PRD device-connect-iroh D-78).
//!
//! Two machines that are paired see some of each other's work: the lists,
//! the phone, what a machine sends the ones that may open it, the marked
//! rows of another machine's work on this screen, and which machine judges
//! a notification all follow from two facts, the core that answers for the
//! work and the machine it runs on. [`judge`] reads those two facts and
//! [`reach`] is the one table of what follows; hided, the phone bridge and
//! the sender to other machines take the answer and decide nothing of their
//! own.
//!
//! This is the axis between cores. Whose a row is inside one core (the
//! operator's, a delegated child's, Hide AI's, an external tool's) is
//! `agent_state::Ownership`, and a row of this core passes that rule as
//! before.

use serde::{Deserialize, Serialize};

/// Whose the work is and where it runs, seen from this machine.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Whose {
    /// This core's work on this machine.
    MineHere,
    /// This core's work on another machine: a mission's worker placed
    /// there, an agent this core's agent started there with a parent
    /// (D-59).
    MineAway,
    /// Another core's work on this machine: a guest's pane (B10).
    TheirsHere,
    /// Another core's work on its own machine, told over the connection
    /// to a machine that may open it (D-50).
    TheirsAway,
}

/// What follows from whose the work is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Reach {
    /// It stands in this machine's Needs You and Done.
    pub list: bool,
    /// It goes to the phones paired with this machine (D-56).
    pub phone: bool,
    /// It goes, as counts and rows, to the machines that may open this one
    /// (D-50).
    pub peers: bool,
    /// It is drawn on this screen as another machine's work, with that
    /// machine's mark (B10).
    pub marked: bool,
    /// This core judges its notifications and picks the one place each
    /// goes (D-60). A row another machine sent rings nothing here on its
    /// own (B71).
    pub notifies: bool,
}

/// Whose the work is: `owner` is the device id of the core that answers for
/// it, `runs_on` the device id of the machine it runs on, and `here` this
/// machine's own.
pub fn judge(here: &str, owner: &str, runs_on: &str) -> Whose {
    match (owner == here, runs_on == here) {
        (true, true) => Whose::MineHere,
        (true, false) => Whose::MineAway,
        (false, true) => Whose::TheirsHere,
        (false, false) => Whose::TheirsAway,
    }
}

/// The one table (D-78).
pub const fn reach(whose: Whose) -> Reach {
    match whose {
        Whose::MineHere => Reach {
            list: true,
            phone: true,
            peers: true,
            marked: false,
            notifies: true,
        },
        Whose::MineAway => Reach {
            list: true,
            phone: true,
            peers: false,
            marked: false,
            notifies: true,
        },
        Whose::TheirsHere => Reach {
            list: false,
            phone: false,
            peers: false,
            marked: true,
            notifies: false,
        },
        Whose::TheirsAway => Reach {
            list: true,
            phone: false,
            peers: false,
            marked: false,
            notifies: false,
        },
    }
}

impl Whose {
    pub const fn reach(self) -> Reach {
        reach(self)
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::MineHere => "mine_here",
            Self::MineAway => "mine_away",
            Self::TheirsHere => "theirs_here",
            Self::TheirsAway => "theirs_away",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_is_judged_by_the_core_that_answers_for_it_and_the_machine_it_runs_on() {
        assert_eq!(judge("mac", "mac", "mac"), Whose::MineHere);
        assert_eq!(judge("mac", "mac", "windows"), Whose::MineAway);
        assert_eq!(judge("windows", "mac", "windows"), Whose::TheirsHere);
        assert_eq!(judge("mac", "windows", "windows"), Whose::TheirsAway);
        // A third machine's work on a second one is no more this machine's
        // than the second one's own.
        assert_eq!(judge("mac", "windows", "mini"), Whose::TheirsAway);
    }

    /// PRD D-78, read row by row: my lists and my phone hold what is mine;
    /// a machine that may open me is told what is mine and runs here; my
    /// screen marks what is not mine and runs here; and I judge a
    /// notification only for what is mine. D-50 adds the rows another
    /// machine told me to my lists, and D-56 keeps them off my phone.
    #[test]
    fn the_table_gives_each_audience_its_rows() {
        let table = [
            (Whose::MineHere, [true, true, true, false, true]),
            (Whose::MineAway, [true, true, false, false, true]),
            (Whose::TheirsHere, [false, false, false, true, false]),
            (Whose::TheirsAway, [true, false, false, false, false]),
        ];
        for (whose, [list, phone, peers, marked, notifies]) in table {
            assert_eq!(
                reach(whose),
                Reach {
                    list,
                    phone,
                    peers,
                    marked,
                    notifies
                },
                "{}",
                whose.name()
            );
        }
    }

    /// No row is told to another machine and shown here as that machine's:
    /// what is sent is never what was received, so a row cannot travel on
    /// to a third machine (D-50).
    #[test]
    fn nothing_received_is_sent_on_and_nothing_rings_in_two_cores() {
        for whose in [
            Whose::MineHere,
            Whose::MineAway,
            Whose::TheirsHere,
            Whose::TheirsAway,
        ] {
            let reach = reach(whose);
            if reach.peers {
                assert_eq!(whose, Whose::MineHere);
            }
            assert_eq!(
                reach.notifies,
                matches!(whose, Whose::MineHere | Whose::MineAway)
            );
            assert!(!(reach.phone && !reach.notifies));
        }
    }
}
