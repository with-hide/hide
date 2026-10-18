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
}

impl KitRefusal {
    pub(crate) fn new(cause: RefusalCause, path: Option<&Path>, detail: impl Into<String>) -> Self {
        Self {
            cause,
            path: path.map(|path| path.display().to_string()),
            variable: None,
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
}

/// A step that is no preflight, such as a retirement step or a record save,
/// keeps reporting the kit's own sentence as a part's reason.
impl From<KitRefusal> for String {
    fn from(refusal: KitRefusal) -> Self {
        refusal.detail
    }
}
