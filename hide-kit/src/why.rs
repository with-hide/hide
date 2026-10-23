//! Why a kit part or an agent's piece is in its state, as the screen words
//! it: a cause the app turns into the operator's language, with the one value
//! its sentence names (a path, a name, a version). The kit's own English
//! sentence stays beside it in `reason`, for the log and the `hide` CLI; no
//! screen draws it (docs/LOCALIZATION.md, Translation boundary).

use serde::{Deserialize, Serialize};

/// What the operator meets in a part's state, one per thing they can do.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartCause {
    /// Hide installed it and it has since gone; Reinstall puts it back.
    TakenOut,
    /// An older version of Hide's file is there.
    OlderVersion,
    /// Hide's entries were edited after Hide wrote them; Reinstall puts
    /// Hide's back.
    Edited,
    /// The hook runs a helper that is gone (`value`: the helper's path).
    HelperGone,
    /// The hook's captured state folder is not this build's.
    RouteChanged,
    /// It points at another Hide build (`value`: what it points at).
    OtherBuild,
    /// The hook's command is not one Hide wrote.
    NotHides,
    /// This build has no file for the part to point at (`value`: the path
    /// it looked for).
    NoHelper,
    /// Hide could not read what is there (`value`: the path, when one).
    Unreadable,
    /// Another program's file holds the place; Hide left it (`value`: the
    /// path).
    OtherProgram,
    /// A skill or plugin of the same name that Hide did not write is there;
    /// Hide left it (`value`: the names).
    Foreign,
    /// The agent is not set up on this machine.
    AgentAbsent,
    /// The agent has not made its settings folder yet; the part goes in once
    /// it has (`value`: the folder).
    NotCreatedYet,
    /// None of the agent's programs is found here (`value`: the program
    /// names).
    ProgramMissing,
    /// This system or this version of the agent takes no such part.
    Unsupported,
    /// The agent is older than the hook needs (`value`: the version needed).
    UpdateAgent,
    /// Hide AI's tool-free folder for agy is missing or not trusted.
    AiFolder,
    /// Hide's file is still there after the agent was switched off.
    StillInPlace,
    /// The pass could not install or change it; the log says why.
    NotChanged,
    /// Codex has not trusted Hide's hook and asks the operator to review it.
    CodexUntrusted,
    /// Installed, but Hide could not save that it installed it, so the next
    /// pass installs it again.
    RecordUnsaved,
    /// Herdr is not found on this machine.
    HerdrAbsent,
    /// This Herdr has no integration for the agent; a newer Herdr has one.
    UpdateHerdr,
    /// Retiring an earlier Hide's coordination has not finished; the next
    /// pass carries it on.
    RetirementPending,
    /// A cause a newer helper named that this build does not know; the
    /// screen says only the state.
    #[serde(other)]
    Other,
}

/// The cause and its value, as a report carries them.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PartWhy {
    pub cause: PartCause,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// A part's why while a pass judges it: what the screen words, and the
/// sentence the log and the CLI keep.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Reason {
    pub why: PartWhy,
    pub text: String,
}

impl Reason {
    pub(crate) fn new(cause: PartCause, text: impl Into<String>) -> Self {
        Self {
            why: PartWhy { cause, value: None },
            text: text.into(),
        }
    }

    pub(crate) fn with(
        cause: PartCause,
        value: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            why: PartWhy {
                cause,
                value: Some(value.into()),
            },
            text: text.into(),
        }
    }

    /// A pass that could not install, change or remove the part, with the
    /// failure's own words for the log.
    pub(crate) fn not_changed(text: impl Into<String>) -> Self {
        Self::new(PartCause::NotChanged, text)
    }

    pub(crate) fn taken_out() -> Self {
        Self::new(
            PartCause::TakenOut,
            "taken out after Hide installed it; Reinstall puts it back",
        )
    }

    /// The report's two fields: the sentence and the cause.
    pub(crate) fn split(reason: Option<Self>) -> (Option<String>, Option<PartWhy>) {
        match reason {
            Some(reason) => (Some(reason.text), Some(reason.why)),
            None => (None, None),
        }
    }
}
