//! What a move step on the other machine answers (`hided core-move <step>`):
//! one line of JSON on its standard output, which `target::run` prints and
//! the driver's `Remote::step` reads. Both ends run one build (the move's
//! build check, then exact-build linking), so the line names the build that
//! wrote it and a reader takes only the answers its step gives: an answer
//! of another build, or one the step does not give, is refused and never
//! read as a default.

use serde::{Deserialize, Serialize};

use super::control::FailedCheck;
use super::handover::Handover;

/// One step's line.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StepLine {
    /// The build of the `hided` that answered (`build_id`).
    pub build: String,
    pub answer: StepAnswer,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum StepAnswer {
    /// `inspect`: what the machine says of itself.
    Inspected(Box<Inspection>),
    /// `check`: the move's checks of the account's logins that fail.
    Checked { failed: Vec<FailedCheck> },
    /// `verify`: the copy matches its manifest and this build loads it.
    Loadable,
    /// `verify`: the files the copy lacks or holds differently, and those
    /// it holds that the manifest does not name.
    Differs {
        differs: Vec<String>,
        extra: Vec<String>,
    },
    /// `place`: the copy is placed under a pending handover; `placed` names
    /// what this run placed, none when an earlier run did.
    Placed {
        placed: Vec<String>,
        handover: Handover,
    },
    /// `start`: the pending core started, and its lease ends `lease_left_ms`
    /// from the answer, as this machine counts it.
    Started { pid: u32, lease_left_ms: u64 },
    /// `status`: this move's handover there, none when it holds none.
    Status { handover: Option<Handover> },
    /// `status`, `abort`, `abandon`: another move's record holds the
    /// machine, so this move holds nothing there.
    OtherMove { intent: String },
    /// `abort`: no core of this move runs there and its copy is back in
    /// `move-incoming`.
    Aborted,
    /// `abort`: the move committed there.
    Active,
    /// `abort`: the pending core and its starter are gone, but the copy
    /// could not be taken back out of the state folder.
    StoppedNotReturned { reason: String },
    /// `finish`: the move's records there are gone.
    Finished,
    /// `abandon`: no core of the move the driver gave up on runs there,
    /// what one ran on is set aside under its intent, and its starter,
    /// handover and copy are gone.
    Abandoned,
    /// `release`: the core stopped for the move back and its copy is staged.
    Staged { files: usize },
    /// `release`, `resume`, `retire`: the core retired for this move back,
    /// which is its commit. `leftover` is what `retire` could not set aside
    /// or remove of the folder after the commit; the commit stands.
    Retired {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        leftover: Option<String>,
    },
    /// `resume`: the core runs on its folder; `leftover` names what of the
    /// move's staged copy could not be removed, which the next staging
    /// removes first.
    Running {
        pid: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        leftover: Option<String>,
    },
    /// The step cannot act yet (a lock another change holds, a core still
    /// starting or not yet confirmed stopped): asking again may answer.
    Busy { reason: String },
    /// The step refused; `file` names the file it refused on.
    Refused {
        file: Option<String>,
        reason: String,
    },
}

/// `inspect`'s answer.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Inspection {
    pub node: String,
    /// Its state folder, in the wire spelling.
    pub state_dir: String,
    /// The brain state files the folder holds.
    pub brain: Vec<String>,
    pub handover: Option<Handover>,
    /// The scope its core would own (once a Herdr socket); none when its engine names none.
    pub herdr_socket: Option<String>,
    /// Its Hide AI settings as stored there, `Ok(None)` when none are, and
    /// `Err` when they could not be read.
    pub ai: Result<Option<serde_json::Value>, String>,
    /// Each of the move's checks that fails there (`preflight`).
    pub failed: Vec<FailedCheck>,
    /// Why the move's copy cannot be sent to the state folder by its path:
    /// a link or a file on the way, named.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_refused: Option<String>,
}

impl StepAnswer {
    /// A refusal with no file.
    pub fn refused(reason: impl Into<String>) -> Self {
        Self::Refused {
            file: None,
            reason: reason.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every answer reads back as written, so the two ends of one build
    /// agree on the wire by construction.
    #[test]
    fn each_answer_reads_back_as_it_was_written() {
        let handover = Handover::new(
            "move-1",
            "a",
            "b",
            super::super::handover::HandoverState::pending_from_now(),
        );
        for answer in [
            StepAnswer::Loadable,
            StepAnswer::Differs {
                differs: vec!["labels.json".to_owned()],
                extra: Vec::new(),
            },
            StepAnswer::Placed {
                placed: vec!["node.json".to_owned()],
                handover: handover.clone(),
            },
            StepAnswer::Status {
                handover: Some(handover),
            },
            StepAnswer::Status { handover: None },
            StepAnswer::OtherMove {
                intent: "move-2".to_owned(),
            },
            StepAnswer::StoppedNotReturned {
                reason: "labels.json: busy".to_owned(),
            },
            StepAnswer::Abandoned,
            StepAnswer::Busy {
                reason: "held".to_owned(),
            },
            StepAnswer::Refused {
                file: Some("node.json".to_owned()),
                reason: "no".to_owned(),
            },
        ] {
            let line = StepLine {
                build: "b1".to_owned(),
                answer,
            };
            let text = serde_json::to_string(&line).unwrap();
            assert_eq!(serde_json::from_str::<StepLine>(&text).unwrap(), line);
        }
    }
}
