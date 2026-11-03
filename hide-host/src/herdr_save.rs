//! A machine's Herdr save and the Herdr server on its default socket, as the
//! machine itself reads them, for moving its session into the core's own
//! engine (PRD engine-bundle-4-herdr-removal D-5, D-6, D-10). The core's
//! machine reads them in its daemon and a device's node answers them for
//! the core (`Call::HerdrSave`, `Call::HerdrStop`), so both machines are
//! read and stopped by this one code. Bundle 4 PR B deletes it with Herdr.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

pub use hide_node_link::herdr_save::{Fetched, Found, Server};

/// The largest save read: an operator's session of 25 workspaces and 83
/// panes took 51 KB on 2026-11-04, so this is far past any session and
/// bounds what an unreadable file can cost.
pub const READ_CAP: u64 = 4 * 1024 * 1024;

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
    let modified_ns = file
        .metadata()
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| {
            u64::try_from(since.as_nanos()).unwrap_or(u64::MAX)
        });
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
    let ours = hide_platform::process::image_of(pid).is_ok_and(|image| {
        image.path.starts_with(&pinned)
            || image
                .path
                .parent()
                .is_some_and(hide_kit::holds_package_runtime)
    });
    match hide_platform::process::start_time(pid) {
        Ok(started) if ours => Server::Hide { pid, started },
        _ => Server::Other,
    }
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
    let same = || hide_platform::process::start_time(pid).is_ok_and(|now| now == started);
    let asked = hide_herdr_client::request_with_timeout(
        socket,
        "server.stop",
        serde_json::json!({}),
        STOP_WITHIN,
    );
    if wait_gone(&same) {
        return Ok(());
    }
    if let Err(error) = asked {
        refused(&format!("server.stop: {error}"));
    }
    if same() {
        hide_platform::process::terminate(pid)
            .map_err(|error| format!("Herdr's server {pid} could not be ended: {error}"))?;
    }
    if wait_gone(&same) {
        Ok(())
    } else {
        Err(format!("Herdr's server {pid} did not end"))
    }
}

#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn wait_gone(same: &dyn Fn() -> bool) -> bool {
    // No event tells another process's exit, so its end is polled.
    let deadline = Instant::now() + STOP_WITHIN;
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
