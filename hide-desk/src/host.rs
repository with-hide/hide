//! One Desk process: the adapter, started tied to this process so it and
//! everything it starts end with hided even when hided is killed, its
//! standard streams the ACP connection (engineering practice: resident
//! processes).
//!
//! Its owner (the core's Desk worker) holds the [`Host`]; dropping it ends
//! the whole tree. The tree is measured after each turn against
//! [`MAX_DESCENDANTS`] and [`MAX_RSS_BYTES`]; crossing either is the owner's
//! failure to report, and the owner ends the host.

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

use hide_platform::process::{self, OwnedChild, TreeMeasure};

use crate::adapter::Launch;
use crate::rpc::{Connection, Sink};

/// Processes one Desk process may run under it: the adapter's agent and
/// the commands a turn runs. A turn that leaves more is a leak.
pub const MAX_DESCENDANTS: usize = 48;
/// Memory the whole tree may hold.
pub const MAX_RSS_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// The adapter's error output kept for a failure's diagnostic.
const STDERR_TAIL_BYTES: usize = 16 * 1024;

/// Variables a Desk process keeps from hided besides the login ones: its
/// shell, its language and the operator's network route. A pane's
/// identity (`HERDR_*`), Hide's own (`HIDE_*`) and every other variable
/// stay out, so no pane hook reads the Desk process as a pane session.
const KEPT_VARIABLES: &[&str] = &[
    "SHELL",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "HTTPS_PROXY",
    "HTTP_PROXY",
    "NO_PROXY",
    "ALL_PROXY",
    "https_proxy",
    "http_proxy",
    "no_proxy",
    "all_proxy",
    "NODE_EXTRA_CA_CERTS",
];

fn kept(name: &OsStr) -> bool {
    let name = name.to_string_lossy();
    process::LOGIN_CHILD_VARIABLES
        .iter()
        .chain(KEPT_VARIABLES)
        .any(|kept| {
            if cfg!(windows) {
                kept.eq_ignore_ascii_case(&name)
            } else {
                *kept == name
            }
        })
}

/// What [`Host::end`] and a failed start leave behind for the diagnostic.
#[derive(Clone, Debug, Default)]
pub struct Tail(Arc<Mutex<Vec<u8>>>);

impl Tail {
    pub fn last_line(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8_lossy(&bytes)
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or_default()
            .chars()
            .take(400)
            .collect()
    }
}

pub struct Host {
    child: OwnedChild,
    connection: Arc<Connection>,
    reader: Option<JoinHandle<()>>,
    stderr: Option<JoinHandle<()>>,
    tail: Tail,
    pid: u32,
    started: u64,
}

impl Host {
    /// Starts the adapter `launch` names. `sink` receives everything the
    /// agent sends that no call waits for.
    pub fn start(launch: &Launch, sink: Sink) -> std::io::Result<Self> {
        std::fs::create_dir_all(&launch.cwd)?;
        for (path, body, executable) in &launch.files {
            write_file(path, body, *executable)?;
        }
        let mut command = Command::new(&launch.program);
        command
            .args(&launch.args)
            .current_dir(&launch.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // `env_clear` is not carried by the tied start, so each variable
        // that may not reach the child is removed by name.
        for (name, _) in std::env::vars_os() {
            if !kept(&name) {
                command.env_remove(name);
            }
        }
        for (name, value) in &launch.env {
            command.env(name, value);
        }
        let mut child = OwnedChild::spawn_tied(&mut command)?;
        let pid = child.id();
        let started = process::start_time(pid)?;
        let stdin = child
            .take_stdin()
            .ok_or_else(|| std::io::Error::other("no stdin"))?;
        let stdout = child
            .take_stdout()
            .ok_or_else(|| std::io::Error::other("no stdout"))?;
        let stderr = child
            .take_stderr()
            .ok_or_else(|| std::io::Error::other("no stderr"))?;
        let tail = Tail::default();
        let stderr = {
            let tail = tail.clone();
            std::thread::Builder::new()
                .name("hide-desk-stderr".into())
                .spawn(move || drain(stderr, &tail))?
        };
        let (connection, reader) = Connection::start(stdout, stdin, sink)?;
        Ok(Self {
            child,
            connection,
            reader: Some(reader),
            stderr: Some(stderr),
            tail,
            pid,
            started,
        })
    }

    pub fn connection(&self) -> &Arc<Connection> {
        &self.connection
    }

    /// The process the capability is bound to: it runs as long as the tree.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn started(&self) -> u64 {
        self.started
    }

    /// Whether the process has exited.
    pub fn exited(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
    }

    pub fn measure(&self) -> std::io::Result<TreeMeasure> {
        process::measure_tree(self.pid)
    }

    /// The last line the adapter wrote to its error output.
    pub fn stderr_tail(&self) -> String {
        self.tail.last_line()
    }

    /// Ends the tree and waits for the reader threads. An end the system
    /// could not confirm is the error; the child's handle still ends what
    /// it can when it drops.
    pub fn end(mut self) -> std::io::Result<String> {
        self.finish()?;
        Ok(self.tail.last_line())
    }

    fn finish(&mut self) -> std::io::Result<()> {
        let ended = self.child.kill_tree().map(|_| ());
        let _ = self.child.wait();
        for thread in [self.reader.take(), self.stderr.take()]
            .into_iter()
            .flatten()
        {
            let _ = thread.join();
        }
        ended
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if self.reader.is_some() {
            // Nobody is left to tell; the child's own drop still ends what
            // the kill could not.
            let _ = self.finish();
        }
    }
}

fn drain(mut stderr: impl Read, tail: &Tail) {
    let mut buffer = [0u8; 4096];
    loop {
        match stderr.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                let mut kept = tail.0.lock().unwrap_or_else(PoisonError::into_inner);
                kept.extend_from_slice(&buffer[..n]);
                let excess = kept.len().saturating_sub(STDERR_TAIL_BYTES);
                kept.drain(..excess);
            }
        }
    }
}

fn write_file(path: &std::path::Path, body: &str, executable: bool) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let access = if executable {
        hide_platform::fs::Access::PrivateExecutable
    } else {
        hide_platform::fs::Access::Private
    };
    hide_platform::fs::atomic::write_file(path, body.as_bytes(), access).map(|_| ())
}

/// A variable kept by name, for a test to check what the child sees.
#[doc(hidden)]
pub fn keeps(name: &str) -> bool {
    kept(&OsString::from(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pane_s_identity_and_hide_s_own_variables_do_not_reach_the_desk_process() {
        for name in [
            "HERDR_PANE_ID",
            "HERDR_SOCKET_PATH",
            "HIDE_CAP_REF",
            "CLAUDECODE",
        ] {
            assert!(!keeps(name), "{name}");
        }
        for name in ["HOME", "PATH", "SHELL", "LANG", "HTTPS_PROXY"] {
            if cfg!(windows) && ["HOME", "SHELL"].contains(&name) {
                continue;
            }
            assert!(keeps(name), "{name}");
        }
    }
}
