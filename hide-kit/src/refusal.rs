//! Why a kit pass stopped on a machine before changing anything there: the
//! retirement preflight refused, or the account's kit folder or lock could
//! not be had. It is one answer for the whole pass, never a copy per part,
//! and it says what the operator can act on; the kit's own words are for the
//! log (engineering rule 10).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::layout::StateVariable;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct KitRefusal {
    /// What stopped the pass, which decides what the operator can do.
    pub cause: RefusalCause,
    /// The folder or file at fault, as that machine spells it; `None` when
    /// no single one is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The variable that chose Hide's state folder, when `path` is that
    /// folder or one above it: the operator can choose another folder or
    /// unset it to return to `~/.hide/state`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variable: Option<StateVariable>,
    /// With `variable`, on the machine whose app runs this kit: the command
    /// that stops Hide's background service there. That service reads the
    /// variable only when it starts and outlives the app, so a changed
    /// variable reaches the kit only once it has stopped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<String>,
    /// The kit's own words, for the log only.
    #[serde(default)]
    pub detail: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalCause {
    /// `path` is a link, belongs to another account, or another account can
    /// change it or a folder above it, so Hide neither trusts what it reads
    /// there nor writes anything.
    Unsafe,
    /// `path` holds work an earlier Hide's coordination left open, or a
    /// record Hide cannot read, so ending that coordination could lose it.
    Record,
    /// Another Hide was changing this account's kit and did not finish in
    /// time.
    Busy,
    /// More checkouts are registered than one pass inspects, so it cannot
    /// tell whether ending the old coordination would lose work in one.
    Projects,
    /// A cause a newer helper named that this build does not know; the
    /// screen says only that the pass stopped.
    #[serde(other)]
    Other,
}

impl KitRefusal {
    pub(crate) fn new(cause: RefusalCause, path: Option<&Path>, detail: impl Into<String>) -> Self {
        Self {
            cause,
            path: path.map(|path| path.display().to_string()),
            variable: None,
            stop: None,
            detail: detail.into(),
        }
    }

    pub(crate) fn unsafe_path(path: &Path, detail: impl Into<String>) -> Self {
        Self::new(RefusalCause::Unsafe, Some(path), detail)
    }

    pub(crate) fn record(path: &Path, detail: impl Into<String>) -> Self {
        Self::new(RefusalCause::Record, Some(path), detail)
    }

    /// A refusal met while inspecting the state folder `variable` chose; only
    /// an unsafe folder is the variable's to fix.
    pub(crate) fn chosen_by(mut self, variable: Option<StateVariable>) -> Self {
        if self.cause == RefusalCause::Unsafe {
            self.variable = variable;
        }
        self
    }

    /// Names, for a refusal of the state folder `state` a variable chose, the
    /// command that stops the background service of the app whose `hide` is
    /// in `kit_dir`: the next launch starts one that reads the variable
    /// again. It names that folder itself, so it stops the right service
    /// whether or not the terminal still has the old value.
    pub fn name_stop(&mut self, kit_dir: &Path, state: &Path) {
        if self.variable.is_none() {
            return;
        }
        let state = state.display().to_string();
        let hide = kit_dir.join(format!("hide{}", std::env::consts::EXE_SUFFIX));
        let hide = hide.display().to_string();
        let quote = hide_agent_hooks::install::quote_for_shell;
        self.stop = Some(if cfg!(windows) {
            format!(
                "$env:HIDE_STATE_DIR = {}; & {} stop; Remove-Item Env:HIDE_STATE_DIR",
                quote(&state),
                quote(&hide)
            )
        } else {
            format!("HIDE_STATE_DIR={} {} stop", quote(&state), quote(&hide))
        });
    }
}

/// A step that is no preflight, such as a retirement step or a record save,
/// keeps reporting the kit's own sentence as a part's reason.
impl From<KitRefusal> for String {
    fn from(refusal: KitRefusal) -> Self {
        refusal.detail
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn the_stop_command_names_the_folder_and_the_app_s_own_hide() {
        let mut refusal = KitRefusal::unsafe_path(Path::new("/srv/team"), "words")
            .chosen_by(Some(StateVariable::HideStateDir));
        refusal.name_stop(
            Path::new("/opt/hide/resources"),
            Path::new("/srv/team/o'hide"),
        );
        assert_eq!(
            refusal.stop.as_deref(),
            Some("HIDE_STATE_DIR='/srv/team/o'\\''hide' '/opt/hide/resources/hide' stop")
        );

        let mut unchosen = KitRefusal::unsafe_path(Path::new("/srv/team"), "words");
        unchosen.name_stop(Path::new("/opt/hide/resources"), Path::new("/srv/team"));
        assert_eq!(
            unchosen.stop, None,
            "only a variable is cleared by a restart"
        );
    }

    #[cfg(windows)]
    #[test]
    fn the_stop_command_names_the_folder_and_the_app_s_own_hide() {
        let mut refusal = KitRefusal::unsafe_path(Path::new(r"D:\"), "words")
            .chosen_by(Some(StateVariable::HideStateDir));
        refusal.name_stop(
            Path::new(r"C:\Users\me\.hide\runtime\b"),
            Path::new(r"D:\o'hide"),
        );
        assert_eq!(
            refusal.stop.as_deref(),
            Some(
                r"$env:HIDE_STATE_DIR = 'D:\o''hide'; & 'C:\Users\me\.hide\runtime\b\hide.exe' stop; Remove-Item Env:HIDE_STATE_DIR"
            )
        );
    }
}
