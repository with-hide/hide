//! Moving the core to another machine and back (PRD core-host-node-move).
//!
//! The machine giving the core drives a forward move: it stops its core,
//! copies its brain state into a staging folder, changes the copy's owner
//! (`node_migration::reown`), sends it over SSH, and has the other machine
//! verify, place and start it (`target`); its own hided then links to that
//! core as a node, and the first link that carries the move's intent is
//! the commit point (`handover`). Every step before it is undone by
//! restarting the old core from its untouched folder. A move back is its
//! own sequence, driven by the node (`back`).

pub mod abandoned;
pub mod answer;
pub mod back;
pub mod control;
pub mod driver;
pub mod gate;
pub mod handover;
pub mod journal;
pub mod preflight;
pub mod screen;
pub mod target;

use std::io::Read;
use std::path::Path;

pub use crate::core_install::checked_intent;

/// A fresh move id.
pub fn new_intent() -> String {
    crate::core_install::new_intent("move")
}

/// Reads a private record of at most `cap` bytes; a record another account
/// can read or one past the cap is refused.
pub(crate) fn read_private(path: &Path, cap: u64) -> std::io::Result<Vec<u8>> {
    let file = hide_platform::fs::private::open_own_file(path, false)?;
    if !hide_platform::fs::private::is_private(path)? {
        return Err(std::io::Error::other(
            "the record is readable by other accounts",
        ));
    }
    let mut bytes = Vec::new();
    file.take(cap + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > cap {
        return Err(std::io::Error::other("the record is too large"));
    }
    Ok(bytes)
}
