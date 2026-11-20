//! The UDP port a state folder's endpoint binds, chosen once and kept in
//! `peers/udp-port` (PRD device-connect-iroh D-38): Windows lets
//! same-network machines reach the endpoint only through a firewall rule
//! for one port, so the port holds across restarts and updates. The port is
//! picked in the dynamic range and checked free when it is chosen; whoever
//! binds the endpoint binds this one.

use std::io::{self, Write};
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::path::Path;

use hide_platform::fs::private;

/// The dynamic range, which no registered service claims.
const FIRST: u16 = 49152;
/// How many free ports are tried before choosing gives up.
const TRIES: usize = 32;

/// The state folder's port: the one chosen before, or a free one chosen and
/// saved now. Two processes choosing at once keep the first saved.
pub fn of(state_dir: &Path) -> io::Result<u16> {
    let path = hide_kit::layout::peer_port(state_dir);
    if let Some(port) = read(&path)? {
        return Ok(port);
    }
    let port = choose()?;
    if let Some(folder) = path.parent() {
        private::create_dir_all(folder)?;
    }
    match private::create_new_file(&path) {
        Ok(mut file) => {
            file.write_all(port.to_string().as_bytes())?;
            file.sync_all()?;
            Ok(port)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            read(&path)?.ok_or_else(|| io::Error::other(format!("{} is empty", path.display())))
        }
        Err(error) => Err(error),
    }
}

/// A new free port for the state folder, saved in place of its port, which
/// another process holds: the endpoint binds the new one, and the firewall
/// rule, which names the old one, reads as missing until it is allowed
/// again. The old port is kept as the one replaced, unless one replaced
/// before is still kept: the rule in place is for that one.
pub fn replace(state_dir: &Path) -> io::Result<u16> {
    let path = hide_kit::layout::peer_port(state_dir);
    let old = read(&path)?;
    let port = choose()?;
    if let Some(folder) = path.parent() {
        private::create_dir_all(folder)?;
    }
    let replaced = hide_kit::layout::peer_port_replaced(state_dir);
    if let Some(old) = old
        && read(&replaced)?.is_none()
    {
        hide_platform::fs::atomic::write_file(
            &replaced,
            old.to_string().as_bytes(),
            hide_platform::fs::Access::Private,
        )?;
    }
    hide_platform::fs::atomic::write_file(
        &path,
        port.to_string().as_bytes(),
        hide_platform::fs::Access::Private,
    )?;
    Ok(port)
}

/// The port the state folder held before [`replace`], until [`retired`].
pub fn replaced(state_dir: &Path) -> io::Result<Option<u16>> {
    read(&hide_kit::layout::peer_port_replaced(state_dir))
}

/// Forgets the replaced port once the firewall rule for it is gone.
pub fn retired(state_dir: &Path) -> io::Result<()> {
    match std::fs::remove_file(hide_kit::layout::peer_port_replaced(state_dir)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// The port saved in `path`, when one is.
pub fn read(path: &Path) -> io::Result<Option<u16>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    match text.trim().parse::<u16>() {
        Ok(port) if port >= FIRST => Ok(Some(port)),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} holds no port of the dynamic range", path.display()),
        )),
    }
}

fn choose() -> io::Result<u16> {
    for _ in 0..TRIES {
        let mut bytes = [0u8; 2];
        getrandom::fill(&mut bytes)
            .map_err(|error| io::Error::other(format!("no random bytes: {error}")))?;
        let port = FIRST + u16::from_le_bytes(bytes) % (u16::MAX - FIRST + 1);
        if UdpSocket::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port))).is_ok() {
            return Ok(port);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AddrInUse,
        format!("no free UDP port in {TRIES} tries"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_folder_keeps_the_port_it_chose() {
        let state = tempfile::tempdir().unwrap();
        let port = of(state.path()).unwrap();
        assert!(port >= FIRST);
        assert_eq!(of(state.path()).unwrap(), port);
        let other = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(other.path().join("peers")).unwrap();
        std::fs::write(other.path().join("peers/udp-port"), "80").unwrap();
        assert_eq!(
            of(other.path()).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    /// A taken port is replaced by a new one, saved as the folder's; the
    /// first port replaced is kept until its rule is retired, through a
    /// second replacement too.
    #[test]
    fn a_replaced_port_is_kept_until_its_rule_is_retired() {
        let state = tempfile::tempdir().unwrap();
        let first = of(state.path()).unwrap();
        let second = replace(state.path()).unwrap();
        assert_eq!(of(state.path()).unwrap(), second);
        assert_eq!(replaced(state.path()).unwrap(), Some(first));
        let third = replace(state.path()).unwrap();
        assert_eq!(of(state.path()).unwrap(), third);
        assert_eq!(replaced(state.path()).unwrap(), Some(first));
        retired(state.path()).unwrap();
        assert_eq!(replaced(state.path()).unwrap(), None);
        retired(state.path()).unwrap();
    }
}
