//! Where the daemon's state folder is on an account, the rule every Hide
//! program follows to find it (PRD hide-home-layout D-04):
//! `hide_kit::layout` names it for the kit and the daemon, and a hook finds
//! the daemon it asks by it (`crate::route::state_dir`).

use std::path::{Path, PathBuf};

/// The folder under HOME that holds everything Hide owns.
pub const HIDE_HOME: &str = ".hide";

pub fn hide_home(home: &Path) -> PathBuf {
    home.join(HIDE_HOME)
}

/// The daemon's state folder when nothing relocates it (D-04).
pub fn default_state_dir(home: &Path) -> PathBuf {
    hide_home(home).join("state")
}

/// The environment variable that moves the state folder away from
/// [`default_state_dir`], named so the operator can be told which one to
/// change or unset.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum StateVariable {
    #[serde(rename = "HIDE_STATE_DIR")]
    HideStateDir,
    #[serde(rename = "XDG_STATE_HOME")]
    XdgStateHome,
}

/// `HIDE_STATE_DIR` wins; a set `XDG_STATE_HOME` keeps `$XDG_STATE_HOME/hide`
/// as before and is never moved; otherwise `~/.hide/state` (D-04). Empty
/// values count as unset, as the callers read them.
pub fn state_dir(
    home: &Path,
    hide_state_dir: Option<&str>,
    xdg_state_home: Option<&str>,
) -> PathBuf {
    state_dir_choice(home, hide_state_dir, xdg_state_home).0
}

/// [`state_dir`] with the variable that chose it, `None` for the default.
/// Judged by the folder, not by which variable is set: `hide connect` hands
/// the daemon it starts the folder it chose through `HIDE_STATE_DIR`, so that
/// variable naming the account's own folder chose nothing.
pub fn state_dir_choice(
    home: &Path,
    hide_state_dir: Option<&str>,
    xdg_state_home: Option<&str>,
) -> (PathBuf, Option<StateVariable>) {
    let (account, account_variable) = match xdg_state_home.filter(|value| !value.is_empty()) {
        Some(xdg) => (
            Path::new(xdg).join("hide"),
            Some(StateVariable::XdgStateHome),
        ),
        None => (default_state_dir(home), None),
    };
    match hide_state_dir.filter(|value| !value.is_empty()) {
        Some(dir) if Path::new(dir) != account => {
            (PathBuf::from(dir), Some(StateVariable::HideStateDir))
        }
        _ => (account, account_variable),
    }
}
