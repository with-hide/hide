//! One bounded side-channel input waiting on the node's existing holder.
use hide_node_link::terminal::{InputPermitError, InputPermitReply};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub(super) const LIMIT: usize = 64;
pub(super) const MAX_AGE: Duration = Duration::from_secs(35);

struct Pending {
    expires: Instant,
    reply: Mutex<Option<InputPermitReply>>,
    count: Arc<AtomicUsize>,
}

impl Drop for Pending {
    fn drop(&mut self) {
        if let Some(reply) = self
            .reply
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            reply(Err(InputPermitError::Discarded));
        }
        self.count.fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(Clone)]
pub(super) struct Permit(Arc<Pending>);

impl std::fmt::Debug for Permit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InputPermit")
    }
}
impl PartialEq for Permit {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for Permit {}

impl Permit {
    pub(super) fn take(count: &Arc<AtomicUsize>, reply: InputPermitReply) -> Option<Self> {
        if count
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                (n < LIMIT).then_some(n + 1)
            })
            .is_err()
        {
            reply(Err(InputPermitError::Limit));
            return None;
        }
        Some(Self(Arc::new(Pending {
            expires: Instant::now() + MAX_AGE,
            reply: Mutex::new(Some(reply)),
            count: Arc::clone(count),
        })))
    }
    pub(super) fn deadline(&self) -> Instant {
        self.0.expires
    }
    pub(super) fn finish(&self, result: Result<(), InputPermitError>) {
        let reply = self
            .0
            .reply
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(reply) = reply {
            reply(result);
        }
    }
}
