//! A core started on a move's copy waits for the move to commit (PRD
//! core-host-node-move amendment 3): while its handover is pending it takes
//! only the link that carries the move's intent, and every worker that acts
//! outside the machine holds until that link is taken. A core with no
//! pending handover is open from the start.
//!
//! The pending lease ends at the deadline the handover records, and the
//! link's commit and the lease's end are decided under one lock: whichever
//! comes first wins, and the other is refused.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;
use tokio::sync::watch;

use super::handover::{self, Activation, HandoverState};

/// Where a core's move stands.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Stand {
    Open,
    /// Waiting for `intent`'s link until `lease_until_unix_ms`.
    Pending {
        intent: String,
        lease_until_unix_ms: u64,
    },
    /// The lease ended first: the core gives its copy back and takes no
    /// link.
    Expired,
}

pub struct MoveGate {
    state_dir: PathBuf,
    stand: std::sync::Mutex<Stand>,
    open: watch::Sender<bool>,
}

impl MoveGate {
    /// The gate for a core starting on `state_dir`.
    pub fn for_start(state_dir: &Path) -> Result<Arc<Self>, String> {
        let stand = match handover::read(state_dir)? {
            Some(record) => match record.state {
                HandoverState::Pending {
                    lease_until_unix_ms,
                } => Stand::Pending {
                    intent: record.intent,
                    lease_until_unix_ms,
                },
                HandoverState::Active => Stand::Open,
                HandoverState::StoppedFor | HandoverState::Retired => {
                    return Err(format!(
                        "this state folder's core left it in move {}; it starts no core",
                        record.intent
                    ));
                }
            },
            None => Stand::Open,
        };
        let open = stand == Stand::Open;
        Ok(Arc::new(Self {
            state_dir: state_dir.to_path_buf(),
            stand: std::sync::Mutex::new(stand),
            open: watch::Sender::new(open),
        }))
    }

    fn stand(&self) -> std::sync::MutexGuard<'_, Stand> {
        self.stand
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn is_open(&self) -> bool {
        *self.open.borrow()
    }

    /// Hears the gate open; it never closes again.
    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.open.subscribe()
    }

    /// When the pending lease ends, on this machine's clock; `None` once
    /// the gate is open or the lease has ended.
    pub fn lease_until_unix_ms(&self) -> Option<u64> {
        match &*self.stand() {
            Stand::Pending {
                lease_until_unix_ms,
                ..
            } => Some(*lease_until_unix_ms),
            Stand::Open | Stand::Expired => None,
        }
    }

    /// Ends the pending lease once its deadline passed and no link took the
    /// move first; answers the move's intent only then, and a later link is
    /// refused.
    pub fn expire(&self, now_unix_ms: u64) -> Option<String> {
        let mut stand = self.stand();
        let Stand::Pending {
            intent,
            lease_until_unix_ms,
        } = &*stand
        else {
            return None;
        };
        if now_unix_ms < *lease_until_unix_ms {
            return None;
        }
        let intent = intent.clone();
        *stand = Stand::Expired;
        Some(intent)
    }

    /// Whether a node's link that names `move_intent` may be taken now: a
    /// pending core takes only its own move's link from the node the move
    /// came from, and taking it is the commit, recorded before the node is
    /// told.
    pub fn admit(&self, node: &str, move_intent: Option<&str>) -> Result<(), &'static str> {
        let mut stand = self.stand();
        let intent = match &*stand {
            Stand::Open => return Ok(()),
            Stand::Expired => return Err("move_unavailable"),
            Stand::Pending { intent, .. } => intent.clone(),
        };
        if move_intent != Some(intent.as_str()) {
            return Err(if move_intent.is_none() {
                "core_pending"
            } else {
                "move_unknown"
            });
        }
        match handover::activate(&self.state_dir, &intent, node) {
            Ok(Activation::Activated | Activation::AlreadyActive) => {
                *stand = Stand::Open;
                drop(stand);
                hide_core::diagnostic!(json!({
                    "component": "core_move",
                    "kind": "move.committed",
                    "intent": intent,
                    "node": node,
                }));
                self.open.send_replace(true);
                Ok(())
            }
            Ok(Activation::NotThisMove) => Err("move_unknown"),
            Err(message) => {
                hide_core::diagnostic!(json!({
                    "component": "core_move",
                    "kind": "move.activation_failed",
                    "intent": intent,
                    "message": message,
                }));
                Err("move_unavailable")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_move::handover::Handover;

    fn pending(lease_until_unix_ms: u64) -> (tempfile::TempDir, Arc<MoveGate>) {
        let dir = tempfile::tempdir().unwrap();
        handover::hold(dir.path())
            .unwrap()
            .write(&Handover::new(
                "i1",
                "m",
                "c",
                HandoverState::Pending {
                    lease_until_unix_ms,
                },
            ))
            .unwrap();
        let gate = MoveGate::for_start(dir.path()).unwrap();
        (dir, gate)
    }

    /// The commit and the lease's end are one decision: a link taken first
    /// leaves the lease nothing to end, and a lease ended first refuses the
    /// link; before its deadline the lease does not end.
    #[test]
    fn a_link_and_the_lease_s_end_never_both_win() {
        let (_dir, gate) = pending(1_000);
        assert_eq!(gate.expire(999), None);
        assert_eq!(gate.admit("m", Some("i1")), Ok(()));
        assert!(gate.is_open());
        assert_eq!(gate.expire(5_000), None);

        let (dir, gate) = pending(1_000);
        assert_eq!(gate.expire(1_000), Some("i1".to_owned()));
        assert_eq!(gate.admit("m", Some("i1")), Err("move_unavailable"));
        assert!(!gate.is_open());
        assert!(
            handover::read(dir.path())
                .unwrap()
                .unwrap()
                .state
                .is_pending()
        );
    }

    #[test]
    fn only_the_node_the_move_came_from_commits_it() {
        let (dir, gate) = pending(u64::MAX);
        assert_eq!(gate.admit("other", Some("i1")), Err("move_unknown"));
        assert_eq!(gate.admit("m", None), Err("core_pending"));
        assert!(
            handover::read(dir.path())
                .unwrap()
                .unwrap()
                .state
                .is_pending()
        );
        assert_eq!(gate.admit("m", Some("i1")), Ok(()));
    }
}
