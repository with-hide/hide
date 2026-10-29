//! Remote host, transport, and projection boundaries for the IDE.
//!
//! Every other machine Hide reaches over SSH is reached with the system
//! `ssh`, one owned process per connection (PRD system-ssh-device-link D-01,
//! D-02, B33): a device's link (`device.rs`), and a node's connection to its
//! core on another machine and a core move's (`session.rs`, `upstream.rs`).
//! OpenSSH reads the account's configuration, keys, agent and known hosts
//! exactly as a terminal does; nothing here implements SSH.

use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::runtime::Runtime;

use hide_herdr_client::{ApiConnector, ApiError, ApiStream};

pub mod askpass;
mod device;
pub mod host;
pub mod hosts;
pub mod openssh;
pub mod profiles;
pub mod session;
pub mod transfer;
pub mod upstream;

pub use device::{Connector, SshDevice};
pub use host::{LinkStream, LinkWriter, OpenError, PaneEvents, PaneEventsSlot, RemoteHost};

use hide_node_link::device::{CapabilityReport, RemoteHostIdentity, RemoteStage, SnapshotCheck};

/// The remembered socket is one owned `String`, so a panic while it was held
/// cannot have left it half-written; the value is taken as it is.
fn lock_recover<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// One device client's Tokio runtime. Its clones travel with channels,
/// forwards and file routes into hided's own async tasks, so the last one can
/// be dropped inside another runtime, where the blocking shutdown of a plain
/// drop panics; there it shuts down in the background instead.
struct RemoteRuntime(Option<Runtime>);

impl std::ops::Deref for RemoteRuntime {
    type Target = Runtime;

    fn deref(&self) -> &Runtime {
        self.0
            .as_ref()
            .expect("a remote runtime is taken only when it drops")
    }
}

impl Drop for RemoteRuntime {
    fn drop(&mut self) {
        let Some(runtime) = self.0.take() else {
            return;
        };
        if tokio::runtime::Handle::try_current().is_ok() {
            runtime.shutdown_background();
        }
    }
}
