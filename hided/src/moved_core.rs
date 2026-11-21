//! What a core moved to another machine before 1.2.0 left behind (PRD
//! device-connect-iroh D-35, D-46). No build moves a core now: the machine
//! that took one keeps it as its own, and the machine that gave it runs the
//! node role from its placement record until the two are split.
//!
//! [`split`] is how the two become two Hides (B45): the node's machine pairs
//! with its core's over the SSH login the move used, both ways and without
//! a question, hands the folder its panes run in to the core's work there,
//! meets the core's machine once on the connection two Hides use, and only
//! then sets its placement record aside and runs a core of its own.
//!
//! A move wrote three records: the driver's journal (`core-move.json`),
//! which stays after the move with the phase it ended in; the handover
//! (`core-handover.json`) on the other machine, which the move's last step
//! removed; and, for a move given up or taken back while the other machine
//! still owed its own end of it, an entry in `core-move-abandoned.json`,
//! removed once that machine had done it. A folder whose journal names a
//! phase that is not an end, or that still holds a handover or such an
//! entry, was left in the middle of a move, and only a build that still
//! moves cores can finish or undo it.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use hide_kit::layout;
use hide_peer::key::MachineKey;
use hide_peer::pairing::{Accepted, Me};
use hide_peer::protocol::PeerKey;
use hide_peer::store::{Grants, Host, Known, KnownState, Via};
use serde::Deserialize;
use serde_json::json;

/// The largest record read: a journal carries the id table of the copy.
const RECORD_CAP: u64 = 4 * 1024 * 1024;
/// The last release whose start finishes or undoes a move.
const LAST_MOVING_RELEASE: &str = "1.2.10";

/// The part of the journal that says whether its move ended.
#[derive(Deserialize)]
struct Journal {
    intent: String,
    phase: Phase,
}

#[derive(Deserialize)]
struct Phase {
    direction: String,
    phase: String,
}

impl Phase {
    /// The phases a move ends in: done, undone, or given up.
    fn ended(&self) -> bool {
        matches!(
            (self.direction.as_str(), self.phase.as_str()),
            ("forward", "done" | "rolled_back" | "abandoned") | ("back", "done" | "rolled_back")
        )
    }
}

/// The part of the handover that names its move.
#[derive(Deserialize)]
struct Handover {
    intent: String,
}

/// The version of `core-move-abandoned.json` every build that wrote one
/// used.
const ABANDONED_VERSION: u32 = 1;

/// The moves this machine gave up, or took back, whose other machine still
/// owes its step: the journal of each already names an end.
#[derive(Deserialize)]
struct Abandoned {
    version: u32,
    moves: Vec<Owed>,
}

/// The part of such a move that names it.
#[derive(Deserialize)]
struct Owed {
    intent: String,
}

/// Why `state_dir` starts no role: a move from before 1.2.0 was left
/// unfinished in it. `None` for every other folder. A record that cannot be
/// read is an error, since nothing says how its move ended.
pub fn unfinished(state_dir: &Path) -> Result<Option<String>, String> {
    let journal: Option<Journal> = read(&layout::core_move_journal(state_dir))?;
    let handover: Option<Handover> = read(&layout::core_handover(state_dir))?;
    let owed = owed(&layout::core_move_abandoned(state_dir))?;
    let intent = match (journal, handover, owed) {
        (Some(journal), _, _) if !journal.phase.ended() => journal.intent,
        (_, Some(handover), _) => handover.intent,
        (_, _, Some(owed)) => owed.intent,
        _ => return Ok(None),
    };
    Ok(Some(format!(
        "a core move from before Hide 1.2.0 ({intent}) was left unfinished in {}, and this build moves no core: open Hide {LAST_MOVING_RELEASE} on both machines once, which finishes or undoes it, then update again",
        state_dir.display()
    )))
}

/// The first move `path` still names: one whose other machine never ended
/// or retired the core it held. A record of another version is one this
/// build cannot read.
fn owed(path: &Path) -> Result<Option<Owed>, String> {
    let Some(abandoned) = read::<Abandoned>(path)? else {
        return Ok(None);
    };
    if abandoned.version != ABANDONED_VERSION {
        return Err(format!(
            "{}: version {} is not one this build reads",
            path.display(),
            abandoned.version
        ));
    }
    Ok(abandoned.moves.into_iter().next())
}

/// The node's machine, about to run a core of its own.
pub struct Splitting<'a> {
    pub home: &'a Path,
    pub state_dir: &'a Path,
    /// This machine's id, which its connection key is kept for.
    pub machine: &'a str,
    /// This machine as its core's machine is told of it.
    pub me: &'a Me,
    /// The core's node id, as the placement record names it.
    pub core: &'a str,
    /// The installation this machine's panes name, which the core's node
    /// here goes on answering as.
    pub panes: layout::PanesInstall,
}

/// Why the two machines were not split: this machine is still the node of
/// its core, as before. `code` is what the window's device tile says.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SplitFailure {
    pub code: &'static str,
    pub detail: String,
}

impl SplitFailure {
    fn not_saved(what: &str, error: impl std::fmt::Display) -> Self {
        Self {
            code: "split_not_saved",
            detail: format!("{what}: {error}"),
        }
    }
}

/// The core's `hide device accept-ssh` for this machine, run on the core's
/// machine over the SSH login the move used: the request, this machine's
/// connection key and, when given, the grant this machine gives the core.
/// It answers the command's value, or the code of why it gave none.
pub type AskCore<'a> =
    dyn Fn(&str, &PeerKey, Option<&str>) -> Result<serde_json::Value, &'static str> + 'a;

/// Meets the core's machine once on the connection two Hides use, from an
/// endpoint of this machine's own key, with the grant that machine gave in
/// force on it (`peer_host::meets`). Why they did not meet, otherwise.
pub type MeetCore<'a> = dyn Fn(&Accepted) -> Result<(), String> + 'a;

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

fn refused(code: &'static str) -> SplitFailure {
    SplitFailure {
        code: match code {
            "ssh_not_reached" => "split_not_reached",
            _ => "split_refused",
        },
        detail: code.to_owned(),
    }
}

/// Makes the node's machine and its core's machine two Hides that are
/// paired both ways (PRD device-connect-iroh D-46, B45).
///
/// The core's Hide saves a grant for this machine and names itself; this
/// machine saves that host and an active grant for the core in its own
/// peer records, which the core role opens its endpoint on; the folder this
/// machine's node ran its panes in is recorded as the core's work here, so
/// those panes go on under the core's node; the core takes this machine's
/// grant; the two meet once on the connection they use from then on, since
/// an SSH login that works says nothing of it; and last the placement
/// record is set aside, which is the commit: the next role this process
/// runs is a core. Every step before it leaves the node as it was and is
/// the same when run again, so a split that stops is asked again whole.
pub fn split(
    splitting: &Splitting<'_>,
    ask: &AskCore<'_>,
    meet: &MeetCore<'_>,
) -> Result<(), SplitFailure> {
    let state = splitting.state_dir;
    let (key, _) = MachineKey::load_or_create(&layout::machine_key(state), splitting.machine)
        .map_err(|error| SplitFailure::not_saved("the machine key", error))?;
    let key = key.public();
    // One request per machine key: asked again, the core answers the grant
    // it saved the first time.
    let request = format!("split-{}", &key.to_hex()[..32]);
    let accepted = answered(ask(&request, &key, None).map_err(refused)?, splitting.core)?;
    let core = accepted.key;
    save_host(state, &accepted)?;

    let mut grants = Grants::open(layout::peer_grants(state))
        .map_err(|error| SplitFailure::not_saved("the grants this machine gives", error))?;
    let now = now_unix_ms();
    let held = grants.current(&core).map(|grant| grant.id.clone());
    let grant = match held {
        Some(grant) => grant,
        // The move was the consent (D-46): the grant is active at once, and
        // the core's confirm finds it so.
        None => {
            let pending = grants
                .grant_pending(core, &accepted.device, &accepted.name, Via::Ssh, now)
                .map_err(|error| SplitFailure::not_saved("the core's grant", error))?;
            grants
                .confirm(&core, &pending.id, now)
                .map_err(|error| SplitFailure::not_saved("the core's grant", error))?;
            pending.id
        }
    };

    hand_over_panes(splitting, &core)?;
    let adopted = ask(&request, &key, Some(&grant)).map_err(refused)?;
    if adopted["adopted"] != true {
        return Err(SplitFailure {
            code: "split_refused",
            detail: "the core did not take this machine's grant".to_owned(),
        });
    }
    // A core that started again between the two asks gave a new grant: the
    // one it holds now is the one this machine confirms with.
    let adopted = answered(adopted, splitting.core)?;
    if adopted.key != core {
        return Err(SplitFailure {
            code: "split_refused",
            detail: "the core's machine answered with another key".to_owned(),
        });
    }
    save_host(state, &adopted)?;
    // Until the two have met there, this machine goes on as the node it is
    // (B45): its core's machine keeps taking its link, since that machine
    // makes it a paired machine's device only on such a connection.
    meet(&adopted).map_err(|detail| SplitFailure {
        code: "split_not_connected",
        detail,
    })?;
    crate::placement::set_aside(state)
        .map_err(|error| SplitFailure::not_saved("the placement record", error))?;
    hide_core::diagnostic!(json!({
        "component": "moved_core",
        "kind": "split.done",
        "core": splitting.core,
        "peer": core.short(),
    }));
    Ok(())
}

/// What the core's machine answered an ask with: itself, and the grant it
/// gives this machine. An answer from any machine but the core's is refused.
fn answered(value: serde_json::Value, core: &str) -> Result<Accepted, SplitFailure> {
    let accepted: Accepted =
        serde_json::from_value(value["accepted"].clone()).map_err(|error| SplitFailure {
            code: "split_refused",
            detail: format!("the core's answer does not read: {error}"),
        })?;
    if accepted.device != core {
        return Err(SplitFailure {
            code: "split_refused",
            detail: format!(
                "the machine answers as {}, not as the core",
                accepted.device
            ),
        });
    }
    Ok(accepted)
}

/// Saves the core's machine as a host this machine confirms with at its
/// first connection. One already saved with the same grant is left as it
/// is, so an answer read again does not undo a confirm.
fn save_host(state: &Path, accepted: &Accepted) -> Result<(), SplitFailure> {
    let mut known = Known::open(layout::peer_known(state))
        .map_err(|error| SplitFailure::not_saved("the hosts this machine knows", error))?;
    let kept = known.of(&accepted.key).is_some_and(|host| {
        host.grant == accepted.grant
            && matches!(host.state, KnownState::Confirming | KnownState::Active)
    });
    if kept {
        return Ok(());
    }
    known
        .put(Host {
            key: accepted.key,
            device: accepted.device.clone(),
            name: accepted.name.clone(),
            grant: accepted.grant.clone(),
            state: KnownState::Confirming,
            candidates: accepted.candidates.clone(),
        })
        .map_err(|error| SplitFailure::not_saved("the core's machine", error))
}

/// Records the folder this machine's node ran its panes in as the core's
/// work here, so the core's node is started on it and the panes go on
/// (`guests::Guests::place`, `guest_node`). A folder another paired machine
/// already took stays that machine's, and one that was never made holds no
/// pane to hand over.
fn hand_over_panes(splitting: &Splitting<'_>, core: &PeerKey) -> Result<(), SplitFailure> {
    let folder = layout::device_node_state(splitting.home);
    if !folder.is_dir() {
        return Ok(());
    }
    let taken = layout::HelperGuest {
        key: core.to_hex(),
        root: layout::guest_install_root(splitting.state_dir, core),
        panes: Some(splitting.panes.clone()),
    };
    match layout::HelperGuest::read(&folder) {
        Some(held) if held == taken => return Ok(()),
        Some(held) if held.key != taken.key => {
            hide_core::diagnostic!(json!({
                "component": "moved_core",
                "kind": "split.panes_held",
                "core": splitting.core,
            }));
            return Ok(());
        }
        _ => {}
    }
    serde_json::to_vec(&taken)
        .map_err(std::io::Error::other)
        .and_then(|bytes| {
            hide_platform::fs::atomic::write_file(
                &layout::device_node_guest(splitting.home),
                &bytes,
                hide_platform::fs::Access::Private,
            )
        })
        .map(drop)
        .map_err(|error| SplitFailure::not_saved("the panes this machine's node ran", error))
}

/// The account's own private record at `path`, or `None` when there is
/// none.
fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    use std::io::Read;
    let refused = |error: &dyn std::fmt::Display| format!("{}: {error}", path.display());
    let file = match hide_platform::fs::private::open_own_file(path, false) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(refused(&error)),
    };
    let mut bytes = Vec::new();
    file.take(RECORD_CAP + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| refused(&error))?;
    if bytes.len() as u64 > RECORD_CAP {
        return Err(refused(&"the record is too large"));
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| refused(&error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hide_peer::store::GrantStatus;

    fn private(path: &Path, text: &str) {
        hide_platform::fs::atomic::write_file(
            path,
            text.as_bytes(),
            hide_platform::fs::Access::Private,
        )
        .unwrap();
    }

    fn journal(dir: &Path, phase: &str) {
        private(
            &hide_kit::layout::core_move_journal(dir),
            &format!(
                r#"{{"version":1,"intent":"move-1","peer":{{"device":"mini","alias":"mini","node":"c","program":"/x/hided","state_dir":"/s"}},"change":{{}},"ids":{{}},"phase":{phase}}}"#
            ),
        );
    }

    /// A folder with no record of a move, and one whose move ended (done,
    /// undone or given up, either way), starts as any folder does.
    #[test]
    fn a_folder_whose_move_ended_starts() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(unfinished(dir.path()), Ok(None));
        for phase in [
            r#"{"direction":"forward","phase":"done"}"#,
            r#"{"direction":"forward","phase":"abandoned"}"#,
            r#"{"direction":"forward","phase":"rolled_back","failed":"copy","cause":{"kind":"local","reason":"x"}}"#,
            r#"{"direction":"back","phase":"done"}"#,
            r#"{"direction":"back","phase":"rolled_back","failed":"copy","cause":{"kind":"local","reason":"x"}}"#,
        ] {
            journal(dir.path(), phase);
            assert_eq!(unfinished(dir.path()), Ok(None), "{phase}");
        }
    }

    /// A journal stopped between two steps, and a handover the move's last
    /// step never removed, each refuse the start and name the move and the
    /// release that can finish it.
    #[test]
    fn a_folder_left_in_the_middle_of_a_move_starts_nothing_and_says_what_finishes_it() {
        for phase in [
            r#"{"direction":"forward","phase":"attach_sent"}"#,
            r#"{"direction":"forward","phase":"committed"}"#,
            r#"{"direction":"back","phase":"retiring"}"#,
        ] {
            let dir = tempfile::tempdir().unwrap();
            journal(dir.path(), phase);
            let reason = unfinished(dir.path()).unwrap().expect(phase);
            assert!(
                reason.contains("move-1") && reason.contains("1.2.10"),
                "{reason}"
            );
        }
        let dir = tempfile::tempdir().unwrap();
        journal(dir.path(), r#"{"direction":"forward","phase":"done"}"#);
        private(
            &hide_kit::layout::core_handover(dir.path()),
            r#"{"version":1,"intent":"move-2","source":"m","target":"c","state":{"state":"active"}}"#,
        );
        let reason = unfinished(dir.path()).unwrap().unwrap();
        assert!(reason.contains("move-2"), "{reason}");
    }

    fn abandoned(dir: &Path, moves: &str) {
        private(
            &hide_kit::layout::core_move_abandoned(dir),
            &format!(r#"{{"version":1,"moves":[{moves}]}}"#),
        );
    }

    /// A move given up before the other machine ended its core, and a move
    /// back taken here before that machine retired, each wrote an end in the
    /// journal and left the other machine's step owed: the folder starts
    /// nothing until that step is done, and starts once it is.
    #[test]
    fn a_move_whose_other_machine_still_owes_its_step_starts_nothing() {
        let peer =
            r#"{"device":"mini","alias":"mini","node":"c","program":"/x/hided","state_dir":"/s"}"#;
        for (phase, step) in [
            (r#"{"direction":"forward","phase":"abandoned"}"#, "abandon"),
            (r#"{"direction":"back","phase":"done"}"#, "retire"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            journal(dir.path(), phase);
            abandoned(
                dir.path(),
                &format!(r#"{{"intent":"move-3","peer":{peer},"step":"{step}"}}"#),
            );
            let reason = unfinished(dir.path()).unwrap().expect(step);
            assert!(
                reason.contains("move-3") && reason.contains("1.2.10"),
                "{reason}"
            );
            abandoned(dir.path(), "");
            assert_eq!(unfinished(dir.path()), Ok(None), "{step}");
        }
    }

    /// A record that is not one this build can read says nothing of how its
    /// move ended, so the start stops on it.
    #[test]
    fn a_record_that_cannot_be_read_stops_the_start() {
        let dir = tempfile::tempdir().unwrap();
        private(&hide_kit::layout::core_move_journal(dir.path()), "{");
        assert!(unfinished(dir.path()).is_err());

        let dir = tempfile::tempdir().unwrap();
        journal(dir.path(), r#"{"direction":"forward","phase":"abandoned"}"#);
        private(&hide_kit::layout::core_move_abandoned(dir.path()), "[");
        assert!(unfinished(dir.path()).is_err());
        private(
            &hide_kit::layout::core_move_abandoned(dir.path()),
            r#"{"version":2,"moves":[]}"#,
        );
        assert!(unfinished(dir.path()).is_err());
    }

    const CORE: &str = "c-node";

    fn core_key() -> PeerKey {
        PeerKey::from_bytes([9; 32])
    }

    /// The node's machine as a move before 1.2.0 left it: a placement
    /// record that names its core, and the folder its node ran panes in.
    struct Moved {
        home: tempfile::TempDir,
        state: std::path::PathBuf,
    }

    impl Moved {
        fn new() -> Self {
            let home = tempfile::tempdir().unwrap();
            let state = home.path().join(".hide/state");
            hide_platform::fs::private::create_dir_all(&state).unwrap();
            crate::placement::write(
                &state,
                &crate::placement::Placement {
                    alias: "mini".to_owned(),
                    node: CORE.to_owned(),
                    program: "/x/hided".to_owned(),
                    state_dir: None,
                    disconnected: false,
                },
            )
            .unwrap();
            hide_platform::fs::private::create_dir_all(&layout::pane_records(
                &layout::device_node_state(home.path()),
            ))
            .unwrap();
            Self { home, state }
        }

        /// A split whose two machines meet on the new connection.
        fn split(&self, ask: &AskCore<'_>) -> Result<(), SplitFailure> {
            self.split_meeting(ask, &|_| Ok(()))
        }

        fn split_meeting(
            &self,
            ask: &AskCore<'_>,
            meet: &MeetCore<'_>,
        ) -> Result<(), SplitFailure> {
            let me = Me {
                device: "m-node".to_owned(),
                name: "MacBook".to_owned(),
            };
            split(
                &Splitting {
                    home: self.home.path(),
                    state_dir: &self.state,
                    machine: "machine-m",
                    me: &me,
                    core: CORE,
                    panes: self.panes(),
                },
                ask,
                meet,
            )
        }

        fn panes(&self) -> layout::PanesInstall {
            layout::PanesInstall {
                id: Some("app".to_owned()),
                cli: Some("/Applications/hide.app/Contents/Resources/hide".into()),
            }
        }

        fn is_node(&self) -> bool {
            crate::placement::read(&self.state, "m-node")
                .unwrap()
                .is_some()
        }

        fn given(&self) -> Option<(String, GrantStatus)> {
            Grants::open(layout::peer_grants(&self.state))
                .unwrap()
                .current(&core_key())
                .map(|grant| (grant.id.clone(), grant.state))
        }

        fn host(&self) -> Option<Host> {
            Known::open(layout::peer_known(&self.state))
                .unwrap()
                .of(&core_key())
                .cloned()
        }
    }

    /// The core's `hide device accept-ssh`, as its machine answers it: the
    /// grant it gives, and that it took this machine's when one came.
    fn core_answers<'a>(
        grant: &'static str,
        asked: &'a std::cell::RefCell<Vec<(String, Option<String>)>>,
    ) -> impl Fn(&str, &PeerKey, Option<&str>) -> Result<serde_json::Value, &'static str> + 'a {
        move |request, _, given| {
            asked
                .borrow_mut()
                .push((request.to_owned(), given.map(str::to_owned)));
            let accepted = Accepted {
                key: core_key(),
                device: CORE.to_owned(),
                name: "Mac mini".to_owned(),
                grant: grant.to_owned(),
                candidates: Vec::new(),
            };
            Ok(match given {
                Some(_) => json!({"accepted": accepted, "adopted": true}),
                None => json!({"accepted": accepted}),
            })
        }
    }

    /// A split leaves this machine a Hide of its own, paired with its
    /// core's machine both ways: the core's machine is a host it confirms
    /// with, the core holds an active grant here, the folder its node ran
    /// panes in is the core's work with the installation those panes name,
    /// and the placement record is set aside, not deleted (B45).
    #[test]
    fn a_split_pairs_the_two_machines_both_ways_and_sets_the_placement_aside() {
        let moved = Moved::new();
        let asked = std::cell::RefCell::new(Vec::new());
        moved.split(&core_answers("g-core", &asked)).unwrap();

        let host = moved.host().unwrap();
        assert_eq!(
            (host.device.as_str(), host.grant.as_str(), host.state),
            (CORE, "g-core", KnownState::Confirming)
        );
        let (given, state) = moved.given().unwrap();
        assert_eq!(state, GrantStatus::Active);
        let asked = asked.into_inner();
        assert_eq!(asked.len(), 2);
        assert_eq!(asked[0].0, asked[1].0, "one request, asked twice");
        assert_eq!((&asked[0].1, &asked[1].1), (&None, &Some(given)));
        assert_eq!(
            layout::HelperGuest::read(&layout::device_node_state(moved.home.path())),
            Some(layout::HelperGuest {
                key: core_key().to_hex(),
                root: layout::guest_install_root(&moved.state, &core_key()),
                panes: Some(moved.panes()),
            })
        );
        assert!(!moved.is_node());
        assert!(
            layout::moved_out(&moved.state)
                .join("core-placement.json")
                .is_file()
        );
    }

    /// A split the core's machine refuses leaves this machine the node it
    /// was, and asked again it goes through with the grant it gave the
    /// first time: one machine is never granted twice.
    #[test]
    fn a_refused_split_leaves_the_node_and_the_next_one_converges() {
        let moved = Moved::new();
        let refuse = |_: &str, _: &PeerKey, given: Option<&str>| match given {
            Some(_) => Err("device_unknown"),
            None => core_answers("g-core", &std::cell::RefCell::default())("r", &core_key(), None),
        };
        let failure = moved.split(&refuse).unwrap_err();
        assert_eq!(failure.code, "split_refused");
        assert!(moved.is_node());
        let first = moved.given().unwrap();

        let unreachable = |_: &str, _: &PeerKey, _: Option<&str>| Err("ssh_not_reached");
        assert_eq!(
            moved.split(&unreachable).unwrap_err().code,
            "split_not_reached"
        );
        assert!(moved.is_node());

        let asked = std::cell::RefCell::new(Vec::new());
        moved.split(&core_answers("g-core", &asked)).unwrap();
        assert_eq!(moved.given().unwrap(), first);
        assert!(!moved.is_node());
        // Split already: asked once more, nothing is asked of it twice over.
        moved.split(&core_answers("g-core", &asked)).unwrap();
        assert_eq!(moved.given().unwrap(), first);
    }

    /// An SSH login that works says nothing of the connection the two Hides
    /// use: while they have not met on it, nothing is set aside and this
    /// machine is the node it was, whatever the core's machine answered over
    /// SSH; once they meet, the same split goes through with the grant it
    /// gave the first time (B45).
    #[test]
    fn a_split_whose_machines_do_not_meet_on_the_new_connection_leaves_the_node() {
        let moved = Moved::new();
        let asked = std::cell::RefCell::new(Vec::new());
        let met = std::cell::RefCell::new(Vec::new());
        let apart = |host: &Accepted| {
            met.borrow_mut().push(host.grant.clone());
            Err("dial: timed out".to_owned())
        };
        let failure = moved
            .split_meeting(&core_answers("g-core", &asked), &apart)
            .unwrap_err();
        assert_eq!(
            failure,
            SplitFailure {
                code: "split_not_connected",
                detail: "dial: timed out".to_owned(),
            }
        );
        assert!(moved.is_node());
        assert!(!layout::moved_out(&moved.state).exists());
        assert_eq!(asked.borrow().len(), 2, "both asks came before the meeting");
        assert_eq!(
            *met.borrow(),
            ["g-core"],
            "met with the grant the core holds"
        );
        let first = moved.given().unwrap();

        moved.split(&core_answers("g-core", &asked)).unwrap();
        assert_eq!(moved.given().unwrap(), first);
        assert!(!moved.is_node());
    }

    /// An answer from a machine that is not the core's saves nothing, and a
    /// core that gave a new grant between the two asks is confirmed with by
    /// the grant it holds now.
    #[test]
    fn only_the_cores_machine_is_paired_and_by_the_grant_it_holds_now() {
        let moved = Moved::new();
        let other = |_: &str, _: &PeerKey, _: Option<&str>| {
            Ok(json!({"accepted": Accepted {
                key: core_key(),
                device: "another".to_owned(),
                name: "Other".to_owned(),
                grant: "g".to_owned(),
                candidates: Vec::new(),
            }}))
        };
        assert_eq!(moved.split(&other).unwrap_err().code, "split_refused");
        assert!(moved.host().is_none() && moved.given().is_none() && moved.is_node());

        let asked = std::cell::RefCell::new(Vec::new());
        let restarted = |request: &str, key: &PeerKey, given: Option<&str>| {
            core_answers(if given.is_some() { "g-new" } else { "g-old" }, &asked)(
                request, key, given,
            )
        };
        moved.split(&restarted).unwrap();
        assert_eq!(moved.host().unwrap().grant, "g-new");
    }

    /// A folder another paired machine already took stays that machine's.
    #[test]
    fn a_folder_another_machine_took_is_left_to_it() {
        let moved = Moved::new();
        let held = layout::HelperGuest {
            key: PeerKey::from_bytes([3; 32]).to_hex(),
            root: moved.home.path().join(".hide/host-helper"),
            panes: None,
        };
        private(
            &layout::device_node_guest(moved.home.path()),
            &serde_json::to_string(&held).unwrap(),
        );
        moved
            .split(&core_answers("g-core", &std::cell::RefCell::default()))
            .unwrap();
        assert_eq!(
            layout::HelperGuest::read(&layout::device_node_state(moved.home.path())),
            Some(held)
        );
    }
}
