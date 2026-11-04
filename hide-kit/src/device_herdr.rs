//! Where an earlier build put the Herdr copy a device ran (PRD
//! device-herdr-update): `<root>/herdr/<id>/herdr`, beside the build
//! folders, with `<root>/herdr/current` leading to the one in use and the
//! node's record of failed handoffs beside them. No build brings one now
//! (PRD engine-bundle-4-herdr-removal D-10): the helper takes each copy off
//! once no process runs it (`hide_host::kit`), and a move of that Herdr's
//! session finds the server it runs by these names (`hide_host::herdr_save`).

use std::path::{Path, PathBuf};

/// The folder under a helper root that holds the Herdr copies.
pub const HERDR_DIR: &str = "herdr";
/// The program's name in a copy's folder.
pub const HERDR_PROGRAM: &str = "herdr";
/// The record of the device's failed handoffs, in [`HERDR_DIR`].
pub const HANDOFF_RECORD: &str = "handoff.json";

/// The folder holding the copies under `root`.
pub fn herdr_dir(root: &Path) -> PathBuf {
    root.join(HERDR_DIR)
}
