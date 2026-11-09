//! Opening a file or folder to everyone, as an operator or another program
//! might, the one test helper for it on every system: a mode on Unix, an
//! access entry for Everyone on Windows. Shared by the crates whose tests
//! check what Hide does with a place others can reach (`#[path]`).

use std::path::Path;
#[cfg(windows)]
use std::process::{Command, Stdio};

/// Gives everyone read access (`write` false) or write access to `path`,
/// the way an operator or another program might.
pub fn widen(path: &Path, write: bool) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let directory = std::fs::metadata(path).unwrap().is_dir();
        let mode = match (write, directory) {
            (true, true) => 0o777,
            (true, false) => 0o666,
            (false, true) => 0o755,
            (false, false) => 0o644,
        };
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }
    #[cfg(windows)]
    {
        let grant = if write {
            "Everyone:(M)"
        } else {
            "Everyone:(R)"
        };
        let status = Command::new("icacls")
            .arg(path)
            .args(["/grant", grant])
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }
}
