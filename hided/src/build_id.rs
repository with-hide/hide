//! Which build a daemon is (PRD labels-in-hided D-19).
//!
//! A build is the SHA-256 of the `hided` executable's bytes. The daemon
//! hashes its own file once at startup, before a later app update can
//! replace the file under the same path, and reports it on `/health`; `hide
//! connect` hashes the `hided` beside itself and replaces a daemon whose
//! build differs. Two binaries with the same version string but different
//! bytes (a dev rebuild, a package of a later commit) are different builds.

use std::io::Read;
use std::path::Path;

/// The build of the executable at `path`, as lowercase hex.
pub fn of_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("{} could not be opened: {error}", path.display()))?;
    let mut context = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buffer = vec![0; 1 << 20];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("{} could not be read: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        context.update(&buffer[..read]);
    }
    Ok(hex::encode(context.finish()))
}

/// The running daemon's build, as its node link's hello names it: the one
/// its environment pins, else the hash of its own executable, read once.
pub fn of_daemon(pinned: Option<&str>) -> Result<String, String> {
    static OWN: std::sync::OnceLock<Result<String, String>> = std::sync::OnceLock::new();
    match pinned {
        Some(build) => Ok(build.to_owned()),
        None => OWN.get_or_init(of_current_exe).clone(),
    }
}

/// This process's build.
pub fn of_current_exe() -> Result<String, String> {
    let exe = std::env::current_exe()
        .and_then(|path| path.canonicalize())
        .map_err(|error| format!("the daemon's executable could not be found: {error}"))?;
    of_file(&exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_is_the_hash_of_the_executable_bytes() {
        let dir = std::env::temp_dir().join(format!("hided-build-id-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = dir.join("a");
        let second = dir.join("b");
        std::fs::write(&first, b"abc").unwrap();
        std::fs::write(&second, b"abd").unwrap();
        assert_eq!(
            of_file(&first).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_ne!(of_file(&first).unwrap(), of_file(&second).unwrap());
        assert!(of_file(&dir.join("missing")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
