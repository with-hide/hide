//! A machine's Herdr save and the Herdr server on its default socket, as the
//! machine itself reads them, for moving its session into the core's own
//! engine (PRD engine-bundle-4-herdr-removal D-5, D-6, D-10). The core's
//! machine reads them in its daemon and a device's node answers them for
//! the core (`Call::HerdrSave`, `Call::HerdrStop`), so both machines are
//! read and stopped by this one code. Bundle 4 PR B deletes it with Herdr.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

pub use hide_node_link::herdr_save::{Fetched, Found, READ_CAP, Server};

/// How long Hide's Herdr has to end on each way of asking it (D-6).
pub const STOP_WITHIN: Duration = Duration::from_secs(10);

/// The account's default Herdr session and the socket beside it, in its
/// config folder (Herdr v0.9.5 `persist/io.rs`, `session_path`): the session
/// Hide's Herdr ran.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Place {
    pub session: PathBuf,
    pub socket: PathBuf,
}

impl Place {
    /// The place of the account whose home is `home`, with the rest of the
    /// variables this process has.
    pub fn of_account(home: &Path) -> Result<Self, String> {
        let variables = |name: &str| match name {
            "HOME" => Some(home.as_os_str().to_owned()),
            _ => std::env::var_os(name),
        };
        let config = hide_platform::host::herdr_config_dir_from(&variables)
            .map_err(|error| format!("Herdr's config folder is unknown: {error}"))?;
        Ok(Self {
            session: config.join("session.json"),
            socket: config.join("herdr.sock"),
        })
    }
}

/// Reads the save at `path`, at most [`READ_CAP`] bytes.
pub fn fetch(path: &Path) -> Fetched {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Fetched::Missing,
        Err(error) => {
            return Fetched::Unreadable {
                reason: format!("it could not be opened: {error}"),
            };
        }
    };
    // The last change tells a save Herdr wrote again (D-9), so a save whose
    // change the system will not say is not read as one never changed.
    let modified_ns = match file
        .metadata()
        .and_then(|metadata| metadata.modified())
        .map_err(|error| error.to_string())
        .and_then(|modified| {
            modified
                .duration_since(UNIX_EPOCH)
                .map_err(|_| "it is dated before 1970".to_owned())
        }) {
        Ok(since) => u64::try_from(since.as_nanos()).unwrap_or(u64::MAX),
        Err(error) => {
            return Fetched::Unreadable {
                reason: format!("its last change could not be read: {error}"),
            };
        }
    };
    let mut text = String::new();
    if let Err(error) =
        std::io::Read::read_to_string(&mut std::io::Read::take(file, READ_CAP + 1), &mut text)
    {
        return Fetched::Unreadable {
            reason: format!("it could not be read: {error}"),
        };
    }
    if text.len() as u64 > READ_CAP {
        return Fetched::Unreadable {
            reason: format!("it is larger than {READ_CAP} bytes"),
        };
    }
    Fetched::Text {
        len: text.len() as u64,
        text,
        modified_ns,
    }
}

/// Who runs the Herdr server on `place`'s socket, for the account whose
/// home is `home`.
pub fn server(place: &Place, home: &Path) -> Server {
    let Ok(stream) = hide_platform::ipc::LocalStream::connect(&place.socket) else {
        return Server::Stopped;
    };
    let Ok(pid) = stream.peer_pid() else {
        return Server::Other;
    };
    drop(stream);
    let pinned = hide_kit::device_herdr::herdr_dir(&hide_kit::layout::helper_root(home));
    let ours = match hide_platform::process::image_of(pid) {
        Ok(image) => {
            image.path.starts_with(&pinned)
                || image
                    .path
                    .parent()
                    .is_some_and(hide_kit::holds_package_runtime)
        }
        // Windows names no process's program file, only the path it started
        // from: the server is Hide's when it runs one of Hide's Herdr files
        // where they are now (D-5, D-11).
        Err(error) if error.kind() == std::io::ErrorKind::Unsupported => {
            hide_herdr_files(home).iter().any(|file| {
                hide_platform::process::running(file).is_ok_and(|pids| pids.contains(&pid))
            })
        }
        Err(_) => false,
    };
    match hide_platform::process::start_time(pid) {
        Ok(started) if ours => Server::Hide { pid, started },
        _ => Server::Other,
    }
}

/// The most folders read for Hide's Herdr files: the kit keeps two builds.
const FOLDERS_CAP: usize = 64;

/// Hide's Herdr program files where they are now, for the account whose
/// home is `home`: each build's runtime copy's, which a Windows package runs
/// (`hide_kit::runtime`), and each pinned copy a device's helper holds.
fn hide_herdr_files(home: &Path) -> Vec<hide_platform::fs::identity::FileId> {
    let program = format!(
        "{}{}",
        hide_kit::device_herdr::HERDR_PROGRAM,
        std::env::consts::EXE_SUFFIX
    );
    let folders = |root: PathBuf| {
        std::fs::read_dir(root)
            .into_iter()
            .flatten()
            .flatten()
            .take(FOLDERS_CAP)
            .map(|entry| entry.path())
    };
    folders(hide_kit::layout::runtime_root(home))
        .filter(|dir| hide_kit::holds_package_runtime(dir))
        .chain(folders(hide_kit::device_herdr::herdr_dir(
            &hide_kit::layout::helper_root(home),
        )))
        .filter_map(|dir| hide_platform::fs::identity::file_id(&dir.join(&program)).ok())
        .collect()
}

/// What this machine's node answers of the account whose home is `home`
/// (`Call::HerdrSave`).
pub fn found(home: &Path) -> Result<Found, String> {
    let place = Place::of_account(home)?;
    Ok(Found {
        save: fetch(&place.session),
        server: server(&place, home),
        socket: hide_platform::path::to_wire(&place.socket)
            .map_err(|error| format!("{}: {error}", place.socket.display()))?,
    })
}

/// Stops the Herdr server `pid`, which started at `started`, on `socket`:
/// by its own `server.stop`, which saves its session as its quit does, else
/// by the system's polite end (D-6); either way it is confirmed gone by its
/// pid and start, so a pid a new process took is never read as it. The
/// socket call needs no program file, so a server whose file an update
/// deleted stops the same way (B42). The reason a polite ask was refused is
/// handed to `refused` for the caller's log.
pub fn stop(socket: &Path, pid: u32, started: u64, refused: &dyn Fn(&str)) -> Result<(), String> {
    stop_within(socket, pid, started, refused, STOP_WITHIN)
}

fn stop_within(
    socket: &Path,
    pid: u32,
    started: u64,
    refused: &dyn Fn(&str),
    within: Duration,
) -> Result<(), String> {
    let same = || hide_platform::process::start_time(pid).is_ok_and(|now| now == started);
    let asked = hide_herdr_client::request_with_timeout(
        socket,
        "server.stop",
        serde_json::json!({}),
        within,
    );
    // A server that ends as it answers may close the call first, so its end
    // is waited for whatever the call came to.
    if wait_gone(&same, within) {
        return Ok(());
    }
    if let Err(error) = asked {
        refused(&format!("server.stop: {error}"));
    }
    // One that ended between the look and the end is gone, not refused.
    if same()
        && let Err(error) = hide_platform::process::terminate(pid)
        && same()
    {
        return Err(format!("Herdr's server {pid} could not be ended: {error}"));
    }
    if wait_gone(&same, within) {
        Ok(())
    } else {
        Err(format!("Herdr's server {pid} did not end"))
    }
}

#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn wait_gone(same: &dyn Fn() -> bool, within: Duration) -> bool {
    // No event tells another process's exit, so its end is polled.
    let deadline = Instant::now() + within;
    while same() {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_save_is_read_whole_with_its_stamp_and_a_missing_or_oversized_one_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        assert_eq!(fetch(&path), Fetched::Missing);
        std::fs::write(&path, "{\"version\":3}").unwrap();
        let Fetched::Text {
            text,
            len,
            modified_ns,
        } = fetch(&path)
        else {
            panic!("the save is read");
        };
        assert_eq!((text.as_str(), len), ("{\"version\":3}", 13));
        assert!(modified_ns > 0);
        std::fs::write(&path, vec![b' '; READ_CAP as usize + 1]).unwrap();
        assert!(matches!(fetch(&path), Fetched::Unreadable { .. }));
    }

    /// Selects [`a_stand_in_herdr_role`] in a child of this test binary.
    const STAND_IN: &str = "HIDE_TEST_HERDR_STAND_IN";

    /// A stand-in for Herdr's server: a child of this test binary that runs
    /// until it is ended, reaped as it ends, so a gone one is not read as
    /// running; it is ended and reaped on drop whatever the test came to.
    struct StandIn {
        pid: u32,
        started: u64,
        input: Option<std::process::ChildStdin>,
        reaper: Option<std::thread::JoinHandle<()>>,
    }

    /// Where a stand-in listens as Herdr's server, when it does.
    const STAND_IN_SOCKET: &str = "HIDE_TEST_HERDR_STAND_IN_SOCKET";

    /// The line a stand-in prints once it waits; the test harness prints the
    /// test's name on the same line before it, so it is printed after a
    /// line break.
    const STAND_IN_WAITING: &str = "hide-test-herdr-stand-in-waiting";

    impl StandIn {
        fn start() -> Self {
            Self::start_from(&std::env::current_exe().unwrap(), None)
        }

        /// A stand-in run from `program`, a copy of this test binary,
        /// listening on `socket` when given.
        fn start_from(program: &Path, socket: Option<&Path>) -> Self {
            let mut command = std::process::Command::new(program);
            if let Some(socket) = socket {
                command.env(STAND_IN_SOCKET, socket);
            }
            let mut child = command
                .args([
                    "--exact",
                    "herdr_save::tests::a_stand_in_herdr_role",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(STAND_IN, "1")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            // It says it runs once it waits on its input, on a line of its
            // own after what the test harness prints; one that has not said
            // so in time is ended, so a test never waits on it unbounded.
            let output = child.stdout.take().unwrap();
            let (said, heard) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let waiting = std::io::BufRead::lines(std::io::BufReader::new(output))
                    .map_while(Result::ok)
                    .any(|line| line == STAND_IN_WAITING);
                let _ = said.send(waiting);
            });
            let waiting = heard.recv_timeout(Duration::from_secs(10));
            if waiting != Ok(true) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("the stand-in did not say it waits: {waiting:?}");
            }
            let pid = child.id();
            Self {
                pid,
                started: hide_platform::process::start_time(pid).unwrap(),
                input: child.stdin.take(),
                reaper: Some(std::thread::spawn(move || {
                    let _ = child.wait();
                })),
            }
        }

        fn running(&self) -> bool {
            hide_platform::process::start_time(self.pid).is_ok_and(|now| now == self.started)
        }
    }

    impl Drop for StandIn {
        fn drop(&mut self) {
            if self.running() {
                let _ = hide_platform::process::terminate(self.pid);
            }
            drop(self.input.take());
            if let Some(reaper) = self.reaper.take() {
                let _ = reaper.join();
            }
        }
    }

    /// The stand-in's own role: it waits on its input until it is ended.
    #[test]
    fn a_stand_in_herdr_role() {
        if std::env::var_os(STAND_IN).is_none() {
            return;
        }
        let _listening = std::env::var_os(STAND_IN_SOCKET)
            .map(|socket| hide_platform::ipc::LocalListener::bind(Path::new(&socket)).unwrap());
        println!("\n{STAND_IN_WAITING}");
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
    }

    // D-6, B3: a server that ends on its socket's `server.stop` is confirmed
    // gone by its pid and start, and the system is not asked to end it.
    #[test]
    fn a_herdr_that_ends_on_server_stop_is_confirmed_gone() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("herdr.sock");
        let herdr = StandIn::start();
        let listener = hide_platform::ipc::LocalListener::bind(&socket).unwrap();
        let closer = listener.closer();
        let pid = herdr.pid;
        // The server reads the call and ends, as Herdr's quit does; a call
        // that never comes ends its wait once the stop returns.
        let server = std::thread::spawn(move || {
            let mut stream = listener.accept().ok()?;
            let mut call = String::new();
            std::io::BufRead::read_line(&mut std::io::BufReader::new(&mut stream), &mut call)
                .ok()?;
            hide_platform::process::terminate(pid).ok()?;
            Some(call)
        });
        let refusals = std::cell::RefCell::new(Vec::new());
        let refused = |reason: &str| refusals.borrow_mut().push(reason.to_owned());
        stop_within(
            &socket,
            herdr.pid,
            herdr.started,
            &refused,
            Duration::from_secs(5),
        )
        .unwrap();
        closer.close();
        assert!(server.join().unwrap().unwrap().contains("server.stop"));
        assert!(!herdr.running());
        assert!(refusals.borrow().is_empty(), "{:?}", refusals.borrow());
    }

    // D-6, B42: a server whose socket does not answer (its program file
    // deleted by an update, or the call refused) is ended by the system, and
    // why the ask failed goes to the caller's log.
    #[test]
    fn a_herdr_whose_socket_does_not_answer_is_ended_by_the_system() {
        let dir = tempfile::tempdir().unwrap();
        let herdr = StandIn::start();
        let refusals = std::cell::RefCell::new(Vec::new());
        let refused = |reason: &str| refusals.borrow_mut().push(reason.to_owned());
        stop_within(
            &dir.path().join("herdr.sock"),
            herdr.pid,
            herdr.started,
            &refused,
            Duration::from_millis(300),
        )
        .unwrap();
        assert!(!herdr.running());
        assert_eq!(refusals.borrow().len(), 1);
        assert!(refusals.borrow()[0].starts_with("server.stop: "));
    }

    // D-6: a pid a new process took is never ended: it is not the server
    // that was found, so it reads as gone.
    #[test]
    fn a_pid_another_process_took_is_never_ended() {
        let dir = tempfile::tempdir().unwrap();
        let other = StandIn::start();
        stop_within(
            &dir.path().join("herdr.sock"),
            other.pid,
            other.started + 1,
            &|_| {},
            Duration::from_millis(300),
        )
        .unwrap();
        assert!(other.running());
    }

    // D-5, D-11: a Herdr server running from a Hide build's runtime copy is
    // Hide's on every system, Windows included, where the system names no
    // process's program file.
    #[test]
    fn a_herdr_running_from_a_hide_runtime_copy_is_hide_s() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let build = "1.0.0-abcdef12";
        let copy = hide_kit::layout::runtime_dir(&home, build);
        std::fs::create_dir_all(&copy).unwrap();
        let exe = |name: &str| copy.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        for program in ["hided", "hide", "hide-agent-hooks"] {
            std::fs::write(exe(program), "").unwrap();
        }
        std::fs::copy(std::env::current_exe().unwrap(), exe("herdr")).unwrap();
        std::fs::write(copy.join(hide_kit::runtime::MARKER), build).unwrap();
        let place = Place {
            session: dir.path().join("session.json"),
            socket: dir.path().join("herdr.sock"),
        };
        let herdr = StandIn::start_from(&exe("herdr"), Some(&place.socket));
        assert_eq!(
            server(&place, &home),
            Server::Hide {
                pid: herdr.pid,
                started: herdr.started,
            }
        );
        // One not run from a Hide file is someone else's.
        let other = StandIn::start_from(
            &std::env::current_exe().unwrap(),
            Some(&dir.path().join("other.sock")),
        );
        let elsewhere = Place {
            socket: dir.path().join("other.sock"),
            ..place.clone()
        };
        assert_eq!(server(&elsewhere, &home), Server::Other);
        drop(other);
    }

    #[test]
    fn no_server_on_the_socket_is_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let place = Place {
            session: dir.path().join("session.json"),
            socket: dir.path().join("herdr.sock"),
        };
        assert_eq!(server(&place, dir.path()), Server::Stopped);
    }
}
