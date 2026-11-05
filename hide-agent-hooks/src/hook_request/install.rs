//! Which Hide installation answers for a pane: the rule a pane's `hide`
//! (`hided`'s `workspace_cli::bootstrap`) and its agent hooks
//! ([`super::send`]) both follow, kept here once (with-hide/hide-private#497
//! and #575; docs/agent-hooks.md, More than one Hide on an account).
//!
//! A pane names the installation that started it
//! ([`crate::route::INSTALL_VARIABLE`]) and every request from it carries
//! that name. A daemon or device node of another installation refuses the
//! request with [`OTHER_INSTALL`] before it proves or runs anything. An
//! earlier build ignores the name it does not know and binds the pane by its
//! folder, so the answer names the installation that gave it too, and the
//! caller takes an answer only when that name is the pane's own
//! ([`confirm`]).

use serde_json::Value;

use crate::route::APP_INSTALL;

/// An answer that does not name the installation the request named: a
/// daemon or node from before answers carried one, which ignores the
/// request's `install` and binds the pane by its folder, or one of another
/// installation. What it answered is never used: a reference expires
/// unclaimed with its issuer, a hook's result is dropped.
pub const INSTALL_UNCONFIRMED: &str = "install_unconfirmed";

/// The refusal of a daemon or node of another installation than the one the
/// pane names (`hided`'s `pane_auth`, `hide_host::panes`), given before it
/// proves the caller or runs anything: no answer about the pane.
pub const OTHER_INSTALL: &str = "other_install";

/// Whether a pane naming `install` asks the device bridges before this
/// account's daemon. A pane another installation's node started, a device
/// helper's, is that node's, and this account's own daemon would bind it by
/// its folder alone; a pane the desktop app started, or one that names no
/// installation, asks the daemon first.
pub fn asks_bridges_first(install: Option<&str>) -> bool {
    install.is_some_and(|install| install != APP_INSTALL)
}

/// Whether the `ok` answer `answer` counts for a pane naming `install`: it
/// names that same installation. A pane that names none (a Herdr pane, a
/// plain terminal) takes any answer, naming one or not.
pub fn confirm(answer: &Value, install: Option<&str>) -> Result<(), &'static str> {
    match install {
        Some(install) if answer["install"].as_str() != Some(install) => Err(INSTALL_UNCONFIRMED),
        _ => Ok(()),
    }
}
