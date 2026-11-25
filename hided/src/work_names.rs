//! What this machine's core shows its work on other machines by (names,
//! branches, marks and agents), read from the core as it changes, for the `core`
//! stream to tell each machine its own (PRD device-connect-iroh B10,
//! `hide_core::work_names`).
//!
//! The follower reads only while a machine this one opens is connected
//! (`wanted`): one snapshot read per change burst the core announces, the
//! names read again only when the devices or their sessions moved, and
//! published only when they read differently. While nothing is wanted it
//! reads nothing and keeps what it published; the read after a pause starts
//! from the whole snapshot.

use std::collections::BTreeMap;
use std::sync::Arc;

use hide_core::work_names::WorkName;
use serde_json::json;
use tokio::sync::{broadcast, watch};

use crate::core::CoreHandle;
use crate::ending::Ending;

// The core bounds what it reads by the bounds the peer link carries: a list
// past the link's would be refused whole by the machine it goes to, and one
// past the core's `guest_panes` would drop every row of that machine there.
const _: () = assert!(
    hide_core::work_names::MAX_NAMES == hide_peer::protocol::core::MAX_WORK_NAMES
        && hide_core::work_names::MAX_NAME_CHARS == hide_peer::protocol::core::MAX_WORK_NAME_CHARS
        && hide_core::work_names::MAX_BRANCH_BYTES
            == hide_peer::protocol::core::MAX_WORK_BRANCH_BYTES
        && hide_core::guest_panes::MAX_MARK_BYTES == hide_peer::protocol::core::MAX_WORK_MARK_BYTES
        && hide_agent_adapter::definition::MAX_ID_BYTES
            == hide_peer::protocol::core::MAX_WORK_AGENT_BYTES
);

/// Each machine's names by its connection key, as the core last read.
pub type Told = Arc<BTreeMap<String, Vec<WorkName>>>;

/// Starts the follower and answers what it publishes.
pub fn follow(
    core: Arc<CoreHandle>,
    wanted: watch::Receiver<bool>,
    shutdown: Ending,
) -> watch::Receiver<Told> {
    let (told, published) = watch::channel(Told::default());
    tokio::spawn(run(core, wanted, told, shutdown));
    published
}

async fn run(
    core: Arc<CoreHandle>,
    mut wanted: watch::Receiver<bool>,
    told: watch::Sender<Told>,
    shutdown: Ending,
) {
    let mut changes = core.notify.subscribe();
    let mut cursor = 0_u64;
    loop {
        if !*wanted.borrow_and_update() {
            cursor = 0;
            tokio::select! {
                () = shutdown.ended() => return,
                moved = wanted.changed() => if moved.is_err() { return },
            }
            continue;
        }
        let reading = Arc::clone(&core);
        let have = cursor;
        let reply = match tokio::task::spawn_blocking(move || reading.snapshot(have)).await {
            Ok(Ok(reply)) => reply,
            // The core's owner thread is gone: the daemon is ending.
            Ok(Err(_)) => return,
            Err(error) => {
                hide_core::diagnostic!(json!({
                    "component": "work_names",
                    "kind": "work_names.read_failed",
                    "reason": error.to_string(),
                }));
                return;
            }
        };
        cursor = reply.revision;
        let rest = &reply.current_rest;
        if rest.navigator.devices.revision > reply.have_revision
            || rest.status.remote.revision > reply.have_revision
        {
            let now = hide_core::work_names::told(
                &rest.navigator.devices.value,
                &rest.status.remote.value,
            );
            told.send_if_modified(|current| {
                if **current == now {
                    return false;
                }
                *current = Arc::new(now);
                true
            });
        }
        tokio::select! {
            () = shutdown.ended() => return,
            changed = changes.recv() => match changed {
                // What was missed is in the next read, which starts from the cursor.
                Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return,
            },
            moved = wanted.changed() => if moved.is_err() { return },
        }
    }
}
