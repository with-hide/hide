//! A device's kit target, as `hided node serve` builds it on the device.
//!
//! The core uploads each build's parts into `<root>/<version>/` beside the
//! helper, and the helper points `<root>/current` at its own version before
//! it applies the kit, so the hooks, the `hide` link name
//! a path that survives the next build (B16). The `herdr` CLI is the pinned
//! copy the build brought (`device_herdr`) when it brought one, and the
//! device's own otherwise; the operator's own `herdr` is never changed.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::KitTarget;

/// The name of the link under the helper root that leads to the running
/// build's folder.
pub const CURRENT: &str = "current";

/// The kit target for a device whose helper root is `root`. `cli_dir` and
/// `herdr_socket` come from the device's consent and registration.
pub fn device_target(
    root: &Path,
    home: &Path,
    cli_dir: &Path,
    herdr_socket: &Path,
    stop: Arc<AtomicBool>,
) -> KitTarget {
    let login_shell = crate::agents::login_shell();
    let pinned = crate::device_herdr::pinned_herdr(root);
    let herdr_bin = if pinned.is_file() {
        Some(pinned)
    } else {
        crate::agents::find(home, login_shell.as_deref(), &stop, "herdr")
            .map(|herdr| herdr.path().to_path_buf())
    };
    let relocated = std::env::var_os("HCOORD_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    KitTarget {
        home: home.to_path_buf(),
        kit_dir: root.join(CURRENT),
        cli_dir: cli_dir.to_path_buf(),
        // The old layout's root too, so its `hide` link is re-pointed rather
        // than left as the operator's (D-13).
        owned_roots: vec![root.to_path_buf(), crate::layout::legacy_helper_root(home)],
        herdr_socket: herdr_socket.to_path_buf(),
        herdr_bin,
        codex: crate::agents::find(home, login_shell.as_deref(), &stop, "codex"),
        login_shell,
        legacy_coordination_home: relocated,
        user_agents: crate::login_agents(home),
        retirement_projects: Vec::new(),
        legacy: crate::legacy::device(home, root),
        stop,
    }
}
