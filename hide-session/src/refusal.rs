//! Why a read of an agent's native records produced nothing, typed by what
//! it says about what an earlier read proved.

use std::fmt::{self, Display, Formatter};

/// Why a read produced nothing; its reason is a stable code, never a path or
/// record text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadRefusal {
    /// The read met the agent writing its database: the database was held
    /// when the read took its snapshot, or its files changed during the read.
    /// A read a moment later can answer, and nothing an earlier read proved
    /// is disproven by this one.
    InMotion(&'static str),
    /// The read refused.
    Refused(String),
}

impl ReadRefusal {
    pub fn reason(&self) -> &str {
        match self {
            Self::InMotion(reason) => reason,
            Self::Refused(reason) => reason,
        }
    }

    /// The refusal `error` carries, or a refusal with its message as reason.
    pub(crate) fn of(error: &anyhow::Error) -> Self {
        error
            .downcast_ref::<Self>()
            .cloned()
            .unwrap_or_else(|| Self::Refused(error.to_string()))
    }
}

impl Display for ReadRefusal {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.reason())
    }
}

impl std::error::Error for ReadRefusal {}

impl From<String> for ReadRefusal {
    fn from(reason: String) -> Self {
        Self::Refused(reason)
    }
}

impl From<&str> for ReadRefusal {
    fn from(reason: &str) -> Self {
        Self::Refused(reason.to_owned())
    }
}

impl From<ReadRefusal> for String {
    fn from(refusal: ReadRefusal) -> Self {
        refusal.reason().to_owned()
    }
}
