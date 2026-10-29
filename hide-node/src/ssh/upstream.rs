//! The connection a node keeps to its core on another machine (PRD
//! core-host-node-remote-core D-04, D-07), and a core move's to the machine
//! on its other side, through the system `ssh` (PRD system-ssh-device-link
//! B33): one owned `ssh` signed in once, whose start script installs a
//! build, runs the move's steps and copies its files, and then runs the
//! core machine's attach role in its own place, whose input and output
//! carry the node's link and its screens' streams (`hided`'s `link_mux`).
//!
//! The screen machine needs no SSH server of its own, only the alias, which
//! OpenSSH reads exactly as a terminal does, its known hosts included.

use std::io::{self, Read};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use hide_node_link::device::{Cancel, NoSignIn, SignIn, SshProblem, Wake};

use super::lock_recover;
use super::openssh;
use super::session::{AttachChannel, Ran, Session, SessionError, SshTarget};
use super::transfer::FileCopy;

/// A probe of the SSH greeting, bounded as a whole by `within`, each
/// resolved address tried in turn with an even share of the time left. The
/// name is resolved on a thread of its own the probe waits for only until its
/// deadline: the system's resolver has no bound, and hangs exactly when the
/// network is down, so it never holds the caller (the node's watch, which
/// the role's end joins). That thread ends when the resolver answers; while
/// one runs, a probe answers not reachable rather than start another.
fn probe<R>(within: Duration, resolving: &Arc<AtomicBool>, resolve: R) -> bool
where
    R: FnOnce() -> io::Result<Vec<SocketAddr>> + Send + 'static,
{
    let deadline = Instant::now() + within;
    if resolving.swap(true, Ordering::SeqCst) {
        return false;
    }
    let (found, answer) = mpsc::channel();
    let running = Arc::clone(resolving);
    let started = std::thread::Builder::new()
        .name("core-resolve".to_owned())
        .spawn(move || {
            let addresses = resolve();
            running.store(false, Ordering::SeqCst);
            let _ = found.send(addresses);
        });
    if started.is_err() {
        resolving.store(false, Ordering::SeqCst);
        return false;
    }
    let Ok(Ok(addresses)) = answer.recv_timeout(within) else {
        return false;
    };
    // Each address has an even share of what is left, so one that never
    // answers (a dead IPv6 address listed first) leaves the next its turn.
    let count = addresses.len();
    addresses.into_iter().enumerate().any(|(index, address)| {
        let left = deadline.saturating_duration_since(Instant::now());
        let share = left / (count - index) as u32;
        if share.is_zero() {
            return false;
        }
        let until = Instant::now() + share;
        let Ok(mut socket) = TcpStream::connect_timeout(&address, share) else {
            return false;
        };
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() || socket.set_read_timeout(Some(left)).is_err() {
            return false;
        }
        let mut greeting = [0_u8; 4];
        socket.read_exact(&mut greeting).is_ok() && &greeting == b"SSH-"
    })
}

/// The node's connection to its core's machine, and a core move's to the
/// machine on its other side: one signed-in [`Session`] at a time, kept for
/// the next step, so a core update and the attach that follows it, or a
/// move's steps and copies, sign in once (PRD system-ssh-device-link B33).
pub struct Upstream {
    target: SshTarget,
    /// The session the last step left, waiting for the next one.
    held: Mutex<Option<Session>>,
    /// Ends this connection's `ssh`, whichever step holds it, and every
    /// later dial: the owner's [`Self::close`].
    cancel: Cancel,
    /// Where the alias resolved at the last dial, for [`Self::reachable`].
    resolved: Mutex<Option<openssh::Resolved>>,
    /// Set while a resolution of the core machine's name runs: one at a
    /// time, however long the system's resolver takes.
    resolving: Arc<AtomicBool>,
    /// Where the next dial's questions go: the sign-in of the operator's
    /// own press, none for a dial nobody pressed for (PRD
    /// system-ssh-device-link D-06, B33). A dial takes it, so one press
    /// allows one sign-in and every later dial asks nobody.
    press: Mutex<Option<Arc<dyn SignIn>>>,
    /// What `ssh` stopped on at the last dial, none when it signed in.
    failed: Mutex<Option<SshProblem>>,
}

impl Upstream {
    /// Dials nothing yet: the first step does.
    pub fn new(target: SshTarget) -> Self {
        Self {
            target,
            held: Mutex::new(None),
            cancel: Cancel::default(),
            resolved: Mutex::new(None),
            resolving: Arc::default(),
            press: Mutex::new(None),
            failed: Mutex::new(None),
        }
    }

    /// Where the next dial may ask the operator: `press`, the sign-in of a
    /// step their own press started, or none. The dial takes it, so the
    /// dials after it ask nobody until the next press; a session already
    /// signed in asks nothing more.
    pub fn may_ask(&self, press: Option<Arc<dyn SignIn>>) {
        *lock_recover(&self.press) = press;
    }

    /// The alias this connection dials.
    pub fn alias(&self) -> &str {
        self.target.alias()
    }

    /// What `ssh` stopped on at the last dial, for a screen to say what the
    /// operator does about it; none when that dial signed in.
    pub fn ssh_problem(&self) -> Option<SshProblem> {
        *lock_recover(&self.failed)
    }

    /// Runs `step` on the session the last step left, or on a new one, and
    /// keeps the session for the next step unless the step lost it.
    fn with_session<T>(
        &self,
        step: impl FnOnce(&mut Session) -> Result<T, SessionError>,
    ) -> Result<T, SessionError> {
        let held = lock_recover(&self.held).take();
        let mut session = match held {
            Some(session) => session,
            None => self.open()?,
        };
        let done = step(&mut session);
        if !session.broken() && !self.cancel.is_cancelled() {
            *lock_recover(&self.held) = Some(session);
        }
        done
    }

    fn open(&self) -> Result<Session, SessionError> {
        let opened = self.dial();
        *lock_recover(&self.failed) = match &opened {
            Err(SessionError::Ssh(failure)) => Some(failure.problem),
            _ => None,
        };
        opened
    }

    fn dial(&self) -> Result<Session, SessionError> {
        let (asks, wake): (Arc<dyn SignIn>, _) = match lock_recover(&self.press).take() {
            Some(press) => (press, Wake::Allowed),
            None => (Arc::new(NoSignIn), Wake::NotAllowed),
        };
        let resolved = self.target.resolve().map_err(SessionError::Ssh)?;
        *lock_recover(&self.resolved) = Some(resolved.clone());
        Session::open(&self.target, &resolved, &self.cancel, asks, wake)
    }

    /// Runs `program attach [--state-dir <dir>]` on the machine in place of
    /// the start script, on the session a core update left or a new one.
    pub fn attach(
        &self,
        program: &str,
        state_dir: Option<&str>,
    ) -> Result<AttachChannel, SessionError> {
        let session = match lock_recover(&self.held).take() {
            Some(session) => session,
            None => self.open()?,
        };
        session.attach(program, state_dir)
    }

    /// Runs `argv` on the machine with no input; see [`Session::run`].
    pub fn run(&self, argv: &[String], cap: usize, within: Duration) -> Result<Ran, SessionError> {
        self.with_session(|session| session.run(argv, cap, within))
    }

    /// Copies `files` to the machine, one after another, each whole
    /// ([`Session::upload`]). The first failure stops the copy; the files
    /// already placed stay.
    pub fn upload(
        &self,
        files: &[FileCopy],
        sent: &(dyn Fn(u64) + Sync),
    ) -> Result<(), SessionError> {
        self.with_session(|session| files.iter().try_for_each(|file| session.upload(file, sent)))
    }

    /// Copies `files` from the machine into their places below `into`, one
    /// after another, each whole ([`Session::download`]). The first failure
    /// stops the copy; the files already placed stay.
    pub fn download(
        &self,
        into: &Path,
        files: &[FileCopy],
        received: &(dyn Fn(u64) + Sync),
    ) -> Result<(), SessionError> {
        self.with_session(|session| {
            files
                .iter()
                .try_for_each(|file| session.download(into, file, received))
        })
    }

    /// Puts this build's programs in their version folder under
    /// `helper_root` on the machine, sending only what it lacks: a core
    /// update (PRD core-host-node-move B10). Answers the path of the build's
    /// `hided` there.
    pub fn install_build(
        &self,
        packages: &super::host::HelperPackages,
        helper_root: &str,
    ) -> Result<String, SessionError> {
        self.with_session(|session| session.install_build(packages, helper_root))
    }

    /// Whether the core machine's SSH server answers now: its name resolved,
    /// a connection to its port, and the server's `SSH-` greeting read, all
    /// within `within`, at the address the last dial resolved. A server
    /// that takes the connection and closes it, a port nothing answers, a
    /// name the resolver has not answered in time, and an alias that goes
    /// through a proxy or a jump host, whose server this machine does not
    /// reach itself, are not reachable.
    pub fn reachable(&self, within: Duration) -> bool {
        let Some(resolved) = lock_recover(&self.resolved).clone() else {
            return false;
        };
        if resolved.proxied() {
            return false;
        }
        let (name, port) = (resolved.hostname, resolved.port);
        probe(within, &self.resolving, move || {
            (name.as_str(), port)
                .to_socket_addrs()
                .map(|found| found.collect())
        })
    }

    /// Whether [`Self::close`] ended this connection: a closed one dials
    /// nothing again, and its owner makes a new one.
    pub fn closed(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Ends the connection, whichever step holds it, and every later dial.
    pub fn close(&self) {
        self.cancel.cancel();
        lock_recover(&self.held).take();
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::net::{Ipv4Addr, TcpListener};

    use super::*;

    /// An address that takes the connection and never greets (a dead
    /// address listed first) leaves the next address its own share of the
    /// deadline, so a live one behind it still answers.
    #[test]
    fn a_silent_first_address_leaves_the_next_its_share_of_the_probe() {
        let silent = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let live = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addresses = vec![silent.local_addr().unwrap(), live.local_addr().unwrap()];
        let greeting = std::thread::spawn(move || {
            let (mut stream, _) = live.accept().unwrap();
            stream.write_all(b"SSH-2.0-fixture\r\n").unwrap();
        });
        let resolving = Arc::new(AtomicBool::new(false));
        let started = Instant::now();
        assert!(probe(Duration::from_millis(600), &resolving, move || Ok(
            addresses
        )));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        greeting.join().unwrap();
        drop(silent);
    }

    /// A resolver that does not answer holds the probe only until its
    /// deadline, and no second resolution starts while it runs; once it has
    /// answered, a probe reads the server's greeting again.
    #[test]
    fn a_resolver_that_hangs_holds_the_probe_only_until_its_deadline() {
        let resolving = Arc::new(AtomicBool::new(false));
        let (release, released) = mpsc::channel::<()>();
        let within = Duration::from_millis(200);
        let started = Instant::now();
        let reached = probe(within, &resolving, move || {
            let _ = released.recv_timeout(Duration::from_secs(30));
            Ok(Vec::new())
        });
        assert!(!reached);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );

        let asked = Arc::new(AtomicBool::new(false));
        let second = Arc::clone(&asked);
        assert!(!probe(within, &resolving, move || {
            second.store(true, Ordering::SeqCst);
            Ok(Vec::new())
        }));
        assert!(!asked.load(Ordering::SeqCst), "a second resolution started");

        drop(release);
        let deadline = Instant::now() + Duration::from_secs(10);
        while resolving.load(Ordering::SeqCst) {
            assert!(Instant::now() < deadline, "the resolution never ended");
            std::thread::yield_now();
        }
        let server = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = server.local_addr().unwrap();
        let greeting = std::thread::spawn(move || {
            let (mut stream, _) = server.accept().unwrap();
            stream.write_all(b"SSH-2.0-fixture\r\n").unwrap();
        });
        assert!(probe(Duration::from_secs(5), &resolving, move || Ok(vec![
            address
        ])));
        greeting.join().unwrap();
    }
}
