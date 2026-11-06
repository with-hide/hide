//! A device's half of moving its Herdr session into the core's own engine
//! (PRD engine-bundle-4-herdr-removal D-10, B18). The device's node reads its
//! save and the server on its default socket and stops Hide's Herdr there
//! (`Call::HerdrSave`, `Call::HerdrStop`, the code this machine's daemon
//! runs), and a move is asked as this machine's is: committed, the save the
//! node read kept in the device's engine folder, and the core role started
//! again, whose start moves it before any link to the device is up
//! (`hided/src/herdr_transition.rs`), so one path takes every Herdr session
//! into an engine (D-7).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hide_node_link::herdr_save::{Found, Server as NodeServer};
use hide_node_link::protocol::Call;

use super::{Asked, KeptSave, Look, Machine, Seen, Server, Take, Taken};
use crate::node_access::{NodeLink, call_as};

/// How long a device has to say what its Herdr saved.
const READ_WITHIN: Duration = Duration::from_secs(15);
/// How long a device's stop may take: Herdr's own end and then the
/// system's, each within `hide_host::herdr_save::STOP_WITHIN`, and the call.
const STOP_WITHIN: Duration = Duration::from_secs(30);

/// A device whose Herdr session moves, on the link its engine opens on.
pub(crate) struct Device {
    pub(crate) link: Arc<dyn NodeLink>,
    /// Its engine's folder in the core's state folder.
    pub(crate) engine_dir: PathBuf,
    /// Its machine id, which its registrations name.
    pub(crate) machine: String,
    /// Whether Hide registered an agent on it (D-5).
    pub(crate) hide_used: bool,
    /// This machine's, whose restart starts the core role again.
    pub(crate) local: Arc<dyn Machine>,
}

fn shown(server: NodeServer) -> Server {
    match server {
        NodeServer::Stopped => Server::Stopped,
        NodeServer::Hide { .. } => Server::Hide,
        NodeServer::Other => Server::Other,
    }
}

fn refused(device: &Device, reason: &str) {
    crate::diagnostic!(serde_json::json!({
        "component": "herdr_import", "kind": "herdr_import.device_refused",
        "machine": device.machine, "reason": reason,
    }));
}

impl Device {
    fn found(&self) -> Result<Found, String> {
        call_as(self.link.as_ref(), Call::HerdrSave, READ_WITHIN)
            .map_err(|error| format!("the device did not say what its Herdr saved: {error}"))
    }

    /// Keeps the save the move takes and asks for it; `commit` marks it
    /// committed first, when the start is to give Hide's registrations to
    /// the engine (D-27). The move is made at the core's next start, before
    /// any link to the device is up, so its server cannot be asked again
    /// then: the save kept is one the node read with no Herdr running, and
    /// only the core role's restart lies between them.
    fn ask(&self, found: Found, resume: bool, commit: bool) -> Result<(), String> {
        if commit {
            super::commit(&self.engine_dir)?;
        }
        super::keep_save(
            &self.engine_dir,
            &KeptSave {
                machine: self.machine.clone(),
                save: found.save,
            },
        )?;
        super::ask(&self.engine_dir, Asked { resume })
    }

    /// What the device's save offers, and whether the core role starts
    /// again with it: Hide's own session that no server runs on the device
    /// moves without asking, once (B1, B18), before its engine opens.
    pub(crate) fn look_first(&self) -> (Seen, bool) {
        let found = match self.found() {
            Ok(found) => found,
            Err(reason) => {
                return (
                    Seen {
                        look: Look::Unreadable(reason),
                        ..Seen::default()
                    },
                    false,
                );
            }
        };
        let look = super::look(&self.engine_dir, &found.save).unwrap_or_else(|reason| {
            refused(self, &reason);
            Look::Unreadable(reason)
        });
        let server = found.server;
        if matches!(look, Look::Offer(_))
            && server == NodeServer::Stopped
            && self.hide_used
            && !super::committed(&self.engine_dir)
        {
            match self.ask(found, true, true) {
                Ok(()) => {
                    self.local.restart();
                    return (Seen::default(), true);
                }
                Err(reason) => refused(self, &reason),
            }
        }
        let outcome = super::outcome(&self.engine_dir).unwrap_or_else(|reason| {
            refused(self, &reason);
            None
        });
        (
            Seen {
                look,
                server: shown(server),
                outcome,
            },
            false,
        )
    }

    fn take_or_fail(&self, take: Take) -> Result<Taken, String> {
        let found = self.found()?;
        let (resume, found) = match (take, found.server) {
            (Take::Move, NodeServer::Other) => {
                return Ok(Taken::Failed(
                    "the Herdr on the device's socket is not Hide's".to_owned(),
                ));
            }
            (Take::Move, NodeServer::Hide { pid, started }) => {
                super::commit(&self.engine_dir)?;
                if let Err(error) = call_as::<()>(
                    self.link.as_ref(),
                    Call::HerdrStop { pid, started },
                    STOP_WITHIN,
                ) {
                    crate::diagnostic!(serde_json::json!({
                        "component": "herdr_import", "kind": "herdr_import.device_not_stopped",
                        "machine": self.machine, "pid": pid, "message": error.to_string(),
                    }));
                    super::not_stopped(&self.engine_dir)?;
                    return Ok(Taken::NotStopped);
                }
                // The save Herdr wrote as it stopped is the one that moves;
                // a Herdr started again since keeps its conversations (D-8).
                let after = self.found()?;
                (after.server == NodeServer::Stopped, after)
            }
            // An import stops nothing: the conversations of a Herdr that
            // runs on wait until it no longer does (D-8, B15).
            (_, NodeServer::Stopped) => (true, found),
            (Take::Import, NodeServer::Hide { .. } | NodeServer::Other) => (false, found),
        };
        let commit = matches!(take, Take::Move) || self.hide_used;
        self.ask(found, resume, commit)?;
        self.local.restart();
        Ok(Taken::Restarting)
    }
}

impl Machine for Device {
    fn look(&self) -> Seen {
        self.look_first().0
    }

    /// Moves the device's save the way `take` says, as this machine's is
    /// moved (B18): Hide's Herdr on it is stopped first, by the node's code
    /// this machine's daemon runs, the same on every system.
    fn take(&self, take: Take) -> Taken {
        self.take_or_fail(take).unwrap_or_else(Taken::Failed)
    }

    fn dismiss(&self) {
        if let Err(reason) = super::dismiss(&self.engine_dir) {
            refused(self, &reason);
        }
    }

    fn restart(&self) {
        self.local.restart();
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::herdr_import::{Fetched, Outcome, asked, committed, kept_save, outcome};
    use crate::node_access::{LinkAnswer, LinkError};

    /// A device's node: the saves it answers in turn, the last one for good,
    /// and whether its Herdr ends when asked.
    struct Node {
        found: Mutex<Vec<Found>>,
        stops: AtomicUsize,
        stop_fails: bool,
    }

    impl NodeLink for Node {
        fn call(&self, call: Call, _: Duration) -> Result<LinkAnswer, LinkError> {
            match call {
                Call::HerdrSave => {
                    let mut found = self.found.lock().unwrap();
                    let next = if found.len() > 1 {
                        found.remove(0)
                    } else {
                        found[0].clone()
                    };
                    Ok(serde_json::to_value(next).unwrap().into())
                }
                Call::HerdrStop { .. } => {
                    self.stops.fetch_add(1, Ordering::SeqCst);
                    if self.stop_fails {
                        Err(LinkError::Unknown("it would not end".into()))
                    } else {
                        Ok(serde_json::Value::Null.into())
                    }
                }
                other => panic!("the move asks the node only for its save: {other:?}"),
            }
        }
    }

    #[derive(Default)]
    struct Local {
        restarts: AtomicUsize,
    }

    impl Machine for Local {
        fn look(&self) -> Seen {
            Seen::default()
        }

        fn take(&self, _: Take) -> Taken {
            Taken::Failed("not this machine's".into())
        }

        fn dismiss(&self) {}

        fn restart(&self) {
            self.restarts.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn save(modified_ns: u64) -> Fetched {
        let text = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/herdr-session/herdr-0.9.5.json"),
        )
        .unwrap();
        Fetched::Text {
            len: text.len() as u64,
            text,
            modified_ns,
        }
    }

    fn found(server: NodeServer, modified_ns: u64) -> Found {
        Found {
            save: save(modified_ns),
            server,
            socket: "/home/me/.config/herdr/herdr.sock".into(),
        }
    }

    fn device(
        dir: &Path,
        found: Vec<Found>,
        stop_fails: bool,
        hide_used: bool,
    ) -> (Device, Arc<Node>, Arc<Local>) {
        let node = Arc::new(Node {
            found: Mutex::new(found),
            stops: AtomicUsize::new(0),
            stop_fails,
        });
        let local = Arc::new(Local::default());
        let device = Device {
            link: Arc::clone(&node) as Arc<dyn NodeLink>,
            engine_dir: dir.join("engine/devices/d1"),
            machine: "m1".into(),
            hide_used,
            local: Arc::clone(&local) as Arc<dyn Machine>,
        };
        (device, node, local)
    }

    // B1, B18: Hide's own session that no Herdr runs on the device any more
    // moves without asking, once: the save the node read is kept with the
    // device's machine, and the core role starts again with it.
    #[test]
    fn a_device_whose_hide_herdr_no_longer_runs_moves_without_asking_once() {
        let dir = tempfile::tempdir().unwrap();
        let (device, _, local) =
            device(dir.path(), vec![found(NodeServer::Stopped, 1)], false, true);
        assert!(device.look_first().1);
        assert_eq!(local.restarts.load(Ordering::SeqCst), 1);
        assert_eq!(
            kept_save(&device.engine_dir).unwrap(),
            Some(KeptSave {
                machine: "m1".into(),
                save: save(1),
            })
        );
        assert_eq!(
            asked(&device.engine_dir).unwrap(),
            Some(Asked { resume: true })
        );
        assert!(committed(&device.engine_dir));
        // Committed, a later look offers it and starts nothing again.
        let (seen, restarting) = device.look_first();
        assert!(!restarting && matches!(seen.look, Look::Offer(_)));
        assert_eq!(local.restarts.load(Ordering::SeqCst), 1);
    }

    // B5, B18: a device save no Hide agent used is offered and not moved.
    #[test]
    fn a_device_save_hide_never_used_is_only_offered() {
        let dir = tempfile::tempdir().unwrap();
        let (device, _, local) = device(
            dir.path(),
            vec![found(NodeServer::Stopped, 1)],
            false,
            false,
        );
        let (seen, restarting) = device.look_first();
        assert!(!restarting && matches!(seen.look, Look::Offer(_)));
        assert_eq!(seen.server, Server::Stopped);
        assert_eq!(local.restarts.load(Ordering::SeqCst), 0);
        assert_eq!(kept_save(&device.engine_dir).unwrap(), None);
    }

    // D-6, B18: the move stops the device's Hide Herdr through its node and
    // moves the save that Herdr wrote as it stopped.
    #[test]
    fn a_device_move_stops_its_hide_herdr_and_moves_the_save_it_wrote_as_it_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let running = NodeServer::Hide { pid: 7, started: 9 };
        let (device, node, local) = device(
            dir.path(),
            vec![found(running, 1), found(NodeServer::Stopped, 2)],
            false,
            true,
        );
        assert_eq!(device.take(Take::Move), Taken::Restarting);
        assert_eq!(node.stops.load(Ordering::SeqCst), 1);
        assert_eq!(
            kept_save(&device.engine_dir).unwrap().unwrap().save,
            save(2)
        );
        assert_eq!(
            asked(&device.engine_dir).unwrap(),
            Some(Asked { resume: true })
        );
        assert_eq!(local.restarts.load(Ordering::SeqCst), 1);
    }

    // B4, B18: a device Herdr that would not stop moves nothing and says so.
    #[test]
    fn a_device_herdr_that_would_not_stop_moves_nothing_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let running = NodeServer::Hide { pid: 7, started: 9 };
        let (device, _, local) = device(dir.path(), vec![found(running, 1)], true, true);
        assert_eq!(device.take(Take::Move), Taken::NotStopped);
        assert_eq!(
            outcome(&device.engine_dir).unwrap(),
            Some(Outcome::NotStopped)
        );
        assert_eq!(kept_save(&device.engine_dir).unwrap(), None);
        assert_eq!(asked(&device.engine_dir).unwrap(), None);
        assert_eq!(local.restarts.load(Ordering::SeqCst), 0);
    }

    // D-8, B20: the operator's own Herdr on a device is never stopped: a
    // move of it is refused, and its import leaves its conversations
    // waiting.
    #[test]
    fn a_device_s_own_herdr_is_imported_and_never_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let (device, node, _) = device(dir.path(), vec![found(NodeServer::Other, 1)], false, false);
        assert!(matches!(device.take(Take::Move), Taken::Failed(_)));
        assert_eq!(kept_save(&device.engine_dir).unwrap(), None);
        assert_eq!(device.take(Take::Import), Taken::Restarting);
        assert_eq!(node.stops.load(Ordering::SeqCst), 0);
        assert_eq!(
            asked(&device.engine_dir).unwrap(),
            Some(Asked { resume: false })
        );
        // A read-only import of a save Hide never used commits nothing (D-27).
        assert!(!committed(&device.engine_dir));
    }

    // D-8: a Herdr started on the device again between the stop and the
    // save read after it keeps its conversations: they wait, not open twice.
    #[test]
    fn a_device_herdr_started_again_after_the_stop_keeps_its_conversations() {
        let dir = tempfile::tempdir().unwrap();
        let running = NodeServer::Hide { pid: 7, started: 9 };
        let again = NodeServer::Hide {
            pid: 8,
            started: 10,
        };
        let (device, _, _) = device(
            dir.path(),
            vec![found(running, 1), found(again, 2)],
            false,
            true,
        );
        assert_eq!(device.take(Take::Move), Taken::Restarting);
        assert_eq!(
            asked(&device.engine_dir).unwrap(),
            Some(Asked { resume: false })
        );
    }
}
