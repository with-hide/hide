//! A machine's Herdr save and whether a Herdr server runs on its default
//! socket, as the machine itself reads them, for the read-only import of an
//! earlier Hide's Herdr session. The core's machine reads them in its daemon
//! and a device's node answers them for the core (`Call::HerdrSave`).

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub use hide_node_link::herdr_save::{Fetched, Found, NEWEST_FORMAT, READ_CAP, Server};

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
///
/// The save is opened without waiting and only as a regular file
/// (`hide_platform::fs::open_regular`, a link to it followed), so a pipe or
/// a device at the path is unreadable at once rather than holding the
/// reader.
pub fn fetch(path: &Path) -> Fetched {
    let file =
        match std::fs::canonicalize(path).and_then(|real| hide_platform::fs::open_regular(&real)) {
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

/// Whether a Herdr server runs on `place`'s socket: `Stopped` when nothing
/// answers there, else `Other`. Nothing is ever stopped from here.
pub fn server(place: &Place) -> Server {
    match hide_platform::ipc::LocalStream::connect(&place.socket) {
        Ok(_) => Server::Other,
        Err(_) => Server::Stopped,
    }
}

/// What this machine's node answers of the account whose home is `home`
/// (`Call::HerdrSave`).
pub fn found(home: &Path) -> Result<Found, String> {
    let place = Place::of_account(home)?;
    Ok(Found {
        save: fetch(&place.session),
        server: server(&place),
        socket: hide_platform::path::to_wire(&place.socket)
            .map_err(|error| format!("{}: {error}", place.socket.display()))?,
    })
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
        assert_eq!(server(&place), Server::Stopped);
    }
}
