//! A device's half of importing its Herdr session into the core's own engine
//! (PRD engine-bundle-4-herdr-removal D-10, B18). The device's node reads its
//! save and whether a server runs on its default socket (`Call::HerdrSave`,
//! the code this machine's daemon runs), and the import is asked as this
//! machine's is: the save the node read is kept in the device's engine
//! folder and the core role started again, whose start imports it before any
//! link to the device is up (`hided/src/herdr_transition.rs`).
//! An import stops no Herdr; the device's Herdr runs on untouched.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hide_node_link::herdr_save::{Found, Server as NodeServer};
use hide_node_link::protocol::Call;

use super::{Asked, KeptSave, Look, Machine, Seen, Server, Take, Taken};
use crate::node_access::{NodeLink, call_as};

/// How long a device has to say what its Herdr saved.
const READ_WITHIN: Duration = Duration::from_secs(15);

/// A device whose Herdr session moves, on the link its engine opens on.
pub(crate) struct Device {
    pub(crate) link: Arc<dyn NodeLink>,
    /// Its engine's folder in the core's state folder.
    pub(crate) engine_dir: PathBuf,
    /// Its machine id, which its registrations name.
    pub(crate) machine: String,
    /// This machine's, whose restart starts the core role again.
    pub(crate) local: Arc<dyn Machine>,
}

fn shown(server: NodeServer) -> Server {
    match server {
        NodeServer::Stopped => Server::Stopped,
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

    /// Keeps the save the import takes and asks for it. The import is made at
    /// the core's next start, before any link to the device is up.
    fn ask(&self, found: Found, resume: bool) -> Result<(), String> {
        super::keep_save(
            &self.engine_dir,
            &KeptSave {
                machine: self.machine.clone(),
                save: found.save,
            },
        )?;
        super::ask(&self.engine_dir, Asked { resume })
    }

    /// What the device's save offers.
    pub(crate) fn look_first(&self) -> Seen {
        let found = match self.found() {
            Ok(found) => found,
            Err(reason) => {
                return Seen {
                    look: Look::Unreadable(reason),
                    ..Seen::default()
                };
            }
        };
        let look = super::look(&self.engine_dir, &found.save).unwrap_or_else(|reason| {
            refused(self, &reason);
            Look::Unreadable(reason)
        });
        let server = found.server;
        let outcome = super::outcome(&self.engine_dir).unwrap_or_else(|reason| {
            refused(self, &reason);
            None
        });
        Seen {
            look,
            server: shown(server),
            outcome,
        }
    }

    fn take_or_fail(&self, take: Take) -> Result<Taken, String> {
        let found = self.found()?;
        let Take::Import = take;
        // An import stops nothing: the conversations of a Herdr that runs on
        // wait until it no longer does (D-8, B15).
        let resume = found.server == NodeServer::Stopped;
        self.ask(found, resume)?;
        self.local.restart();
        Ok(Taken::Restarting)
    }
}

impl Machine for Device {
    fn look(&self) -> Seen {
        self.look_first()
    }

    /// Imports the device's save the way `take` says, as this machine's is
    /// imported (B18).
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
    use crate::herdr_import::{Fetched, asked, kept_save};
    use crate::node_access::{LinkAnswer, LinkError};

    /// A device's node: the saves it answers in turn, the last one for good,
    /// and whether its Herdr ends when asked.
    struct Node {
        found: Mutex<Vec<Found>>,
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

    fn device(dir: &Path, found: Vec<Found>) -> (Device, Arc<Local>) {
        let node = Arc::new(Node {
            found: Mutex::new(found),
        });
        let local = Arc::new(Local::default());
        let device = Device {
            link: node as Arc<dyn NodeLink>,
            engine_dir: dir.join("engine/devices/d1"),
            machine: "m1".into(),
            local: Arc::clone(&local) as Arc<dyn Machine>,
        };
        (device, local)
    }

    // B5, B18: a device save no Hide agent used is offered and not imported
    // until asked.
    #[test]
    fn a_device_save_is_only_offered_until_asked() {
        let dir = tempfile::tempdir().unwrap();
        let (device, local) = device(dir.path(), vec![found(NodeServer::Stopped, 1)]);
        let seen = device.look_first();
        assert!(matches!(seen.look, Look::Offer(_)));
        assert_eq!(seen.server, Server::Stopped);
        assert_eq!(local.restarts.load(Ordering::SeqCst), 0);
        assert_eq!(kept_save(&device.engine_dir).unwrap(), None);
    }

    // B18, D-10: an asked import keeps the save the node read with the
    // device's machine id, and tells the core role to start again once; a
    // later look offers the same save and starts nothing.
    #[test]
    fn an_asked_device_import_keeps_its_save_with_the_machine_and_restarts_once() {
        let dir = tempfile::tempdir().unwrap();
        let (device, local) = device(dir.path(), vec![found(NodeServer::Stopped, 1)]);
        assert_eq!(device.take(Take::Import), Taken::Restarting);
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
        assert!(matches!(device.look_first().look, Look::Offer(_)));
        assert_eq!(local.restarts.load(Ordering::SeqCst), 1);
    }

    // D-8, B20: the operator's own Herdr on a device is never stopped: its
    // import leaves its conversations waiting.
    #[test]
    fn a_device_s_own_herdr_is_imported_and_never_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let (device, _) = device(dir.path(), vec![found(NodeServer::Other, 1)]);
        assert_eq!(device.take(Take::Import), Taken::Restarting);
        assert_eq!(
            asked(&device.engine_dir).unwrap(),
            Some(Asked { resume: false })
        );
    }
}
