//! Whether an agent's last turn left it waiting for the operator, read from
//! the structured records of its session file (PRD codex-plan-approval-hold
//! D-01..D-04).
//!
//! The tracker knows no agent: an agent's line parser turns its own records
//! into [`TurnMark`]s (`native` owns Claude/Codex records), and the
//! tracker folds them into one derived fact, [`Waiting`], that the doorbell
//! and the sidebar read without looking at the agent's kind. The caller keeps
//! the tracker beside its read checkpoint and hands it back on the next read,
//! so an incremental read continues the turn it was in.

pub(crate) mod content;
pub(crate) mod native;
pub(crate) mod wake;
pub use content::{UserTurnContent, UserTurnFact, UserTurnKind};

use hide_agent_adapter::work::{WorkKind, WorkRecord, WorkState};
use serde::{Deserialize, Serialize};

/// What the last turn leaves the operator to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Waiting {
    /// The agent waits for nothing the operator must answer.
    Nothing,
    /// The agent proposed a plan and waits for the operator to approve it.
    PlanApproval,
    /// A native question tool is waiting for the operator's answer.
    Question,
}

/// The mode a turn's start record names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnMode {
    Plan,
    /// A mode the parser knows runs no plan.
    Other,
    /// No mode, or one the parser does not know: a plan this turn proposes
    /// is not known to wait or not.
    Unknown,
}

/// One structured record about a turn, as an agent's parser reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TurnMark {
    /// A turn started in `mode`.
    Started {
        turn: Option<String>,
        mode: TurnMode,
    },
    /// The turn proposed a plan.
    Plan { turn: Option<String> },
    /// The provider's structured plan item or explicitly tagged plan body.
    PlanContent {
        turn: Option<String>,
        content: UserTurnContent,
    },
    /// Native question calls and their correlated results in one record.
    Tools(Vec<ToolTurnMark>),
    /// The turn finished.
    Completed { turn: Option<String> },
    /// The turn was interrupted.
    Aborted { turn: Option<String> },
    /// A person's message.
    Human,
    /// A person's message in a format whose native question calls are
    /// known but which supplies no separate task-start record (Claude).
    HumanTurn,
    /// A native interruption, without a turn identifier.
    Interrupted,
    /// A record too large to keep whose structure no bounded scan certifies
    /// (an unknown envelope, a record nested past the scan's depth): what the
    /// turn waits for is not known until the next turn starts or a person
    /// writes.
    Unreadable,
    /// Background work started or ended, or the process that ran it did.
    /// These follow their own offset (`wake_through`): a record that also
    /// carries a turn mark folds both.
    Wake(Vec<WakeMark>),
}

/// One record's word about work that outlives the turn that started it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WakeMark {
    /// A new process began, so nothing a process before it started is alive.
    Boot,
    Started {
        id: String,
        kind: Option<WorkKind>,
        /// Correlates the native description carried by the tool call.
        call: Option<String>,
        /// When the source says the task expires on its own.
        expires_at_unix_ms: Option<u64>,
    },
    Ended {
        id: String,
    },
    /// A call to a tool that can start background work. Its result may carry
    /// the start; one this reader cannot read (`StopOutcome::Unreadable`) may
    /// have started a device it cannot name.
    Call {
        call: String,
        name: Option<String>,
    },
    /// A call asked to stop the task `id`. It ends the device only if the
    /// result of the call `call` says it stopped it.
    Stop {
        call: String,
        id: String,
    },
    /// The result of the tool call `call`.
    Answered {
        call: String,
        outcome: StopOutcome,
    },
    /// A record about devices this reader could not follow: none is proven
    /// until the next process starts.
    Lost,
}

/// What a tool result says about the call it answers, as far as background
/// work is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopOutcome {
    Succeeded,
    /// The tool reported an error: a stop did not stop its task, and a call
    /// started nothing.
    Failed,
    /// The result is a record this reader could not keep, so whether the
    /// call started or stopped anything is not known.
    Unreadable,
}

/// Why no device is proven, until the next process starts. The two causes
/// call for different action by whoever reads the log: one raises
/// [`WAKE_DEVICE_LIMIT`], the other fixes a reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeLoss {
    /// More devices, or calls waiting for their result, were open at once than
    /// are tracked: the session is being read wrongly or abused.
    Capacity,
    /// A record that may have started or ended a device could not be
    /// followed (too large to keep, an id too long to keep, more marks than
    /// the bound, or a form this reader does not know).
    Lost,
}

/// A tracked device or pending call. More than this many at once is a
/// [`WakeLoss::Capacity`].
pub const WAKE_DEVICE_LIMIT: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct WakeDevice {
    #[serde(deserialize_with = "content::deserialize_id")]
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<WorkKind>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "content::deserialize_optional_id"
    )]
    name: Option<String>,
    #[serde(default)]
    state: WorkState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at_unix_ms: Option<u64>,
}

/// A call whose result has not been read: one to a tool that can start
/// background work, or a `TaskStop`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct WakeCall {
    #[serde(deserialize_with = "content::deserialize_id")]
    call: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "content::deserialize_optional_id"
    )]
    name: Option<String>,
    /// The task a `TaskStop` asked to stop; none for a call that starts work.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "content::deserialize_optional_id"
    )]
    stop: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolTurnMark {
    Asked {
        call: String,
        content: Option<UserTurnContent>,
    },
    /// A question whose tool returns at once while the agent goes on (Codex
    /// `request_user_input_async`): its tool result answers nothing, and each
    /// of its `questions` waits for its own [`ToolTurnMark::Replied`].
    AskedAsync {
        call: String,
        questions: u32,
        content: Option<UserTurnContent>,
    },
    Answered {
        call: String,
    },
    /// The operator's reply to question number `question` of an async call.
    Replied {
        call: String,
        question: u32,
    },
}

/// A provider can ask any number of questions in a turn, one after another,
/// but no unbounded call history is retained: at most this many calls are
/// kept, and an answered call leaves to make room for a new one. A ninth call
/// while eight still wait makes the turn unknown until a person writes or a
/// new turn starts.
pub const QUESTION_CALL_LIMIT: usize = 8;
pub const NATIVE_ID_LIMIT_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Plan,
    Other,
    /// The turn's start was not read (it began before where the read
    /// started), so its mode is not known.
    Unseen,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum End {
    Completed,
    Aborted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Turn {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "content::deserialize_optional_id"
    )]
    id: Option<String>,
    mode: Mode,
    #[serde(default)]
    plan: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plan_content: Option<UserTurnContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    end: Option<End>,
    /// A person wrote after the turn ended.
    #[serde(default)]
    answered: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct QuestionCall {
    #[serde(deserialize_with = "content::deserialize_id")]
    call: String,
    content: Option<UserTurnContent>,
    answered: bool,
    /// For an async call, the numbers of its questions still waiting for a
    /// reply; its tool result is not an answer. `None` for a call its own
    /// result answers.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_unreplied"
    )]
    unreplied: Option<Vec<u32>>,
}

/// The last turn of a session as far as it has been read. Byte offsets
/// deduplicate reread records; native call IDs deduplicate only within the
/// current turn, whose bounded history resets on a new human/start record.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnTracker {
    /// The offset of the first record not folded yet. A read that starts
    /// earlier (a reader resuming at its anchor) folds nothing twice.
    through: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last: Option<Turn>,
    /// A person wrote while no turn record had been read, so the records
    /// read are not the ones this tracker knows (a format it does not
    /// read): what the session waits for is not known.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    unstructured: bool,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "deserialize_calls"
    )]
    questions: Vec<QuestionCall>,
    /// More questions waited at once than the tracker keeps, since the last
    /// turn start or person's message: what the turn waits for is not known.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    question_capacity: bool,
    /// An unreadable record was folded since the last turn start or
    /// person's message.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    unreadable: bool,
    /// A plan the session's current state says awaits approval, set anew
    /// on every read of a format that keeps that state outside its records
    /// (Grok's `plan_mode.json`); its content is `None` when withheld.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plan_hold: Option<PlanHold>,
    /// The offset of the first wake record not folded yet.
    #[serde(default, skip_serializing_if = "is_zero")]
    wake_through: u64,
    /// The read saw the record that begins the process now running, so the
    /// devices below are its own. A session read from a file with none proves
    /// no device.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    booted: bool,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "deserialize_work"
    )]
    wake: Vec<WakeDevice>,
    /// A bounded recent execution history, not an unbounded transcript copy.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "deserialize_ended_work"
    )]
    ended_work: Vec<WakeDevice>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    work_history_truncated: bool,
    /// Calls whose result has not been read: a device ends when its stop
    /// succeeded, not when it was asked for, and a result this reader could
    /// not keep (`StopOutcome::Unreadable`) of a call that can start work
    /// loses the devices.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "deserialize_work"
    )]
    wake_calls: Vec<WakeCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    wake_loss: Option<WakeLoss>,
    /// A new process started while devices were proven alive, so the work the
    /// agent was waiting for is gone. It holds until a turn or a device
    /// begins.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    wake_vanished: bool,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

fn deserialize_work<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Work<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Work<T> {
        type Value = Vec<T>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(
                formatter,
                "at most {WAKE_DEVICE_LIMIT} internal work entries"
            )
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            let mut values = Vec::new();
            while let Some(value) = sequence.next_element()? {
                if values.len() == WAKE_DEVICE_LIMIT {
                    return Err(serde::de::Error::custom("internal_work_capacity"));
                }
                values.push(value);
            }
            Ok(values)
        }
    }
    deserializer.deserialize_seq(Work(std::marker::PhantomData))
}

fn deserialize_ended_work<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<WakeDevice>, D::Error> {
    let records: Vec<WakeDevice> = deserialize_work(deserializer)?;
    if records
        .iter()
        .any(|record| record.state == WorkState::Running)
    {
        return Err(serde::de::Error::custom("internal_work_history_running"));
    }
    Ok(records)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PlanHold {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content: Option<UserTurnContent>,
}

/// A restored checkpoint is peer or store data: an async call holds at most
/// [`QUESTION_CALL_LIMIT`] waiting questions, as the parser admits no more,
/// and the sequence is refused before it is retained past that.
fn deserialize_unreplied<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<u32>>, D::Error> {
    struct Waiting;
    impl<'de> serde::de::Visitor<'de> for Waiting {
        type Value = Option<Vec<u32>>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(formatter, "at most {QUESTION_CALL_LIMIT} waiting questions")
        }
        fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_some<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> Result<Self::Value, D::Error> {
            deserializer.deserialize_seq(self)
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            let mut waiting = Vec::new();
            while waiting.len() < QUESTION_CALL_LIMIT {
                let Some(question) = sequence.next_element()? else {
                    return Ok(Some(waiting));
                };
                waiting.push(question);
            }
            if sequence.next_element::<serde::de::IgnoredAny>()?.is_some() {
                return Err(serde::de::Error::custom("user_turn_capacity"));
            }
            Ok(Some(waiting))
        }
    }
    deserializer.deserialize_option(Waiting)
}

fn deserialize_calls<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<QuestionCall>, D::Error> {
    struct Calls;
    impl<'de> serde::de::Visitor<'de> for Calls {
        type Value = Vec<QuestionCall>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(formatter, "at most {QUESTION_CALL_LIMIT} question calls")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            let mut calls = Vec::new();
            while calls.len() < QUESTION_CALL_LIMIT {
                let Some(call) = sequence.next_element()? else {
                    return Ok(calls);
                };
                calls.push(call);
            }
            if sequence.next_element::<serde::de::IgnoredAny>()?.is_some() {
                return Err(serde::de::Error::custom("user_turn_capacity"));
            }
            Ok(calls)
        }
    }
    deserializer.deserialize_seq(Calls)
}

impl TurnTracker {
    /// Keep the native wait while omitting content an older peer cannot read.
    pub fn clear_user_turn_content(&mut self) {
        if let Some(last) = &mut self.last {
            last.plan_content = None;
        }
        if let Some(hold) = &mut self.plan_hold {
            hold.content = None;
        }
        for question in &mut self.questions {
            question.content = None;
        }
    }

    /// Existing records establish the kind of wait, never invented text.
    pub fn user_turn(&self) -> Option<UserTurnFact> {
        match self.waiting()? {
            Waiting::Nothing => None,
            Waiting::PlanApproval => Some(UserTurnFact {
                kind: UserTurnKind::PlanApproval,
                content: match &self.plan_hold {
                    Some(hold) => hold.content.clone(),
                    None => self.last.as_ref()?.plan_content.clone(),
                },
            }),
            Waiting::Question => {
                let pending = || self.questions.iter().filter(|question| !question.answered);
                let content = pending()
                    .all(|question| question.content.is_some())
                    .then(|| {
                        UserTurnContent::combine(
                            pending().filter_map(|question| question.content.as_ref()),
                        )
                    });
                Some(UserTurnFact {
                    kind: UserTurnKind::Question,
                    content,
                })
            }
        }
    }

    /// Replace the plan wait read from the session's current state: the
    /// state is the whole answer, so a read that finds none clears it.
    pub fn set_plan_hold(&mut self, hold: Option<UserTurnContent>) {
        self.plan_hold = hold.map(|content| PlanHold {
            content: Some(content),
        });
    }

    /// Only a read including the current process boundary proves absence.
    pub fn work_known(&self) -> bool {
        self.booted && self.wake_loss().is_none()
    }

    /// Native execution facts, scoped by the caller to this tracker's
    /// proven session and read generation. Expiry is derived at observation
    /// time and never converted into successful completion.
    pub fn work_records(&self, now_unix_ms: u64) -> Vec<WorkRecord> {
        if !self.work_known() {
            return Vec::new();
        }
        self.ended_work
            .iter()
            .chain(&self.wake)
            .map(|device| WorkRecord {
                id: device.id.clone(),
                kind: device.kind,
                name: device.name.clone(),
                state: if device.state == WorkState::Running
                    && device
                        .expires_at_unix_ms
                        .is_some_and(|at| at <= now_unix_ms)
                {
                    WorkState::Expired
                } else {
                    device.state
                },
                expires_at_unix_ms: device.expires_at_unix_ms,
            })
            .collect()
    }

    pub fn work_history_truncated(&self) -> bool {
        self.work_history_truncated
    }

    /// The devices the agent waited for died with the process that ran them,
    /// and nothing has begun since.
    pub fn wake_vanished(&self) -> bool {
        self.wake_vanished
    }

    /// Why no device is proven until the next process starts, if none is
    /// for that reason: the first cause since the process started.
    pub fn wake_loss(&self) -> Option<WakeLoss> {
        self.wake_loss
            .or_else(|| (self.tracked() > WAKE_DEVICE_LIMIT).then_some(WakeLoss::Capacity))
    }

    fn lose_wake(&mut self, cause: WakeLoss) {
        self.wake_loss.get_or_insert(cause);
        self.wake.clear();
        self.wake_calls.clear();
        self.ended_work.clear();
    }

    /// What is tracked now: the devices and the calls waiting for a result
    /// share [`WAKE_DEVICE_LIMIT`].
    fn tracked(&self) -> usize {
        self.wake.len() + self.wake_calls.len()
    }

    /// A call to wait for the result of. Nothing is waited for before the
    /// process began or after devices were lost: there is none to lose.
    fn open_call(&mut self, call: &str, stop: Option<&String>, name: Option<&String>) {
        if !self.booted
            || self.wake_loss.is_some()
            || self.wake_calls.iter().any(|open| open.call == call)
        {
            return;
        }
        if self.tracked() >= WAKE_DEVICE_LIMIT {
            self.lose_wake(WakeLoss::Capacity);
        } else {
            self.wake_calls.push(WakeCall {
                call: call.to_owned(),
                name: name.cloned(),
                stop: stop.cloned(),
            });
        }
    }

    fn end_work(&mut self, id: &str, state: WorkState) {
        if let Some(at) = self.wake.iter().position(|device| device.id == id) {
            let mut device = self.wake.remove(at);
            device.state = state;
            if self.ended_work.len() == WAKE_DEVICE_LIMIT {
                self.ended_work.remove(0);
                self.work_history_truncated = true;
            }
            self.ended_work.push(device);
        }
    }

    fn fold_wake(&mut self, offset: u64, marks: &[WakeMark]) {
        if offset < self.wake_through {
            return;
        }
        self.wake_through = offset + 1;
        // A restored checkpoint has individually bounded vectors; their
        // combined live table must obey the same limit as new records.
        if self.tracked() > WAKE_DEVICE_LIMIT {
            self.lose_wake(WakeLoss::Capacity);
        }
        for mark in marks {
            match mark {
                WakeMark::Boot => {
                    self.booted = true;
                    // Claude writes one record per start hook, so a start is several
                    // Boots in a row; the later ones find nothing left to lose.
                    self.wake_vanished |= !self.wake.is_empty();
                    self.wake.clear();
                    // The new process is a new observation scope. Neither
                    // live work nor history from its predecessor transfers.
                    self.ended_work.clear();
                    self.work_history_truncated = false;
                    // A call of the process before is never answered for this one.
                    self.wake_calls.clear();
                    self.wake_loss = None;
                }
                WakeMark::Started {
                    id,
                    kind,
                    call,
                    expires_at_unix_ms,
                } => {
                    if !self.booted || self.wake_loss.is_some() {
                        continue;
                    }
                    self.wake_vanished = false;
                    let name = call
                        .as_ref()
                        .and_then(|call| {
                            self.wake_calls
                                .iter()
                                .position(|open| open.call == *call && open.stop.is_none())
                        })
                        .and_then(|at| self.wake_calls.remove(at).name);
                    self.ended_work.retain(|device| device.id != *id);
                    if let Some(known) = self.wake.iter_mut().find(|device| device.id == *id) {
                        known.expires_at_unix_ms = *expires_at_unix_ms;
                        known.kind = *kind;
                        if name.is_some() {
                            known.name = name;
                        }
                    } else if self.tracked() >= WAKE_DEVICE_LIMIT {
                        self.lose_wake(WakeLoss::Capacity);
                    } else {
                        self.wake.push(WakeDevice {
                            id: id.clone(),
                            kind: *kind,
                            name,
                            state: WorkState::Running,
                            expires_at_unix_ms: *expires_at_unix_ms,
                        });
                    }
                }
                WakeMark::Ended { id } => self.end_work(id, WorkState::Ended),
                WakeMark::Call { call, name } => self.open_call(call, None, name.as_ref()),
                WakeMark::Stop { call, id } => self.open_call(call, Some(id), None),
                WakeMark::Answered { call, outcome } => {
                    let Some(at) = self.wake_calls.iter().position(|open| open.call == *call)
                    else {
                        continue;
                    };
                    let open = self.wake_calls.remove(at);
                    match (outcome, open.stop) {
                        (StopOutcome::Succeeded, Some(id)) => {
                            self.end_work(&id, WorkState::Stopped)
                        }
                        (StopOutcome::Unreadable, _) => self.lose_wake(WakeLoss::Lost),
                        (StopOutcome::Succeeded | StopOutcome::Failed, _) => {}
                    }
                }
                WakeMark::Lost => self.lose_wake(WakeLoss::Lost),
            }
        }
    }

    /// Folds the mark of a record its agent is still writing (OpenCode's
    /// message while its question waits). The record is not settled: the
    /// same offset folds again once it is complete, so its answer clears the
    /// wait its question opened.
    pub fn fold_unsettled(&mut self, offset: u64, mark: &TurnMark) {
        if offset < self.through {
            return;
        }
        self.fold(offset, mark);
        self.through = offset;
    }

    /// Folds the mark of the record at `offset`; a record before what was
    /// already folded is ignored.
    pub fn fold(&mut self, offset: u64, mark: &TurnMark) {
        if let TurnMark::Wake(marks) = mark {
            self.fold_wake(offset, marks);
            return;
        }
        if offset < self.through {
            return;
        }
        self.through = offset + 1;
        match mark {
            TurnMark::Started { turn, mode } => {
                self.wake_vanished = false;
                // A provider may repeat a native start at another byte
                // offset. It must not reopen an answered plan or question.
                if turn.is_some()
                    && self
                        .last
                        .as_ref()
                        .is_some_and(|last| last.id == *turn && last.mode != Mode::Unseen)
                {
                    return;
                }
                // An async question outlives the turn that asked it: only its
                // reply, a person's message or an abort ends it.
                self.questions
                    .retain(|question| question.unreplied.is_some() && !question.answered);
                self.question_capacity = false;
                self.unreadable = false;
                self.last = Some(Turn {
                    id: turn.clone(),
                    mode: match mode {
                        TurnMode::Plan => Mode::Plan,
                        TurnMode::Other => Mode::Other,
                        TurnMode::Unknown => Mode::Unseen,
                    },
                    plan: false,
                    plan_content: None,
                    end: None,
                    answered: false,
                });
            }
            TurnMark::Plan { turn } => self.current(turn).plan = true,
            TurnMark::PlanContent { turn, content } => {
                let current = self.current(turn);
                current.plan = true;
                current.plan_content = Some(content.clone());
            }
            TurnMark::Tools(marks) => {
                for mark in marks {
                    match mark {
                        ToolTurnMark::Asked { call, content } => {
                            if !self.ask(call, content, None) {
                                break;
                            }
                        }
                        ToolTurnMark::AskedAsync {
                            call,
                            questions,
                            content,
                        } => {
                            if !self.ask(call, content, Some((0..*questions).collect())) {
                                break;
                            }
                        }
                        ToolTurnMark::Answered { call } => {
                            if let Some(question) = self
                                .questions
                                .iter_mut()
                                .find(|known| known.call == *call && known.unreplied.is_none())
                            {
                                question.answered = true;
                                question.content = None;
                            }
                        }
                        ToolTurnMark::Replied { call, question } => {
                            if let Some(known) = self
                                .questions
                                .iter_mut()
                                .find(|known| known.call == *call && known.unreplied.is_some())
                            {
                                let unreplied = known.unreplied.as_mut().expect("checked above");
                                unreplied.retain(|waiting| waiting != question);
                                if unreplied.is_empty() {
                                    known.answered = true;
                                    known.content = None;
                                }
                            }
                        }
                    }
                }
            }
            TurnMark::Completed { turn } => {
                let current = self.current(turn);
                if current.end != Some(End::Completed) {
                    current.end = Some(End::Completed);
                    current.answered = false;
                }
            }
            TurnMark::Aborted { turn } => {
                if self
                    .last
                    .as_ref()
                    .is_none_or(|last| turn.is_none() || last.id.is_none() || last.id == *turn)
                {
                    self.clear_questions();
                }
                self.current(turn).end = Some(End::Aborted);
            }
            TurnMark::Human => {
                self.wake_vanished = false;
                self.questions.clear();
                self.question_capacity = false;
                self.unreadable = false;
                match self.last.as_mut() {
                    Some(turn) if turn.end.is_some() => turn.answered = true,
                    Some(_) => {}
                    None => self.unstructured = true,
                }
            }
            TurnMark::HumanTurn => {
                self.wake_vanished = false;
                self.questions.clear();
                self.question_capacity = false;
                self.unreadable = false;
                self.unstructured = false;
                self.last = Some(Turn {
                    id: None,
                    mode: Mode::Other,
                    plan: false,
                    plan_content: None,
                    end: None,
                    answered: false,
                });
            }
            TurnMark::Interrupted => {
                self.clear_questions();
                if let Some(turn) = &mut self.last {
                    turn.end = Some(End::Aborted);
                }
            }
            TurnMark::Unreadable => self.unreadable = true,
            TurnMark::Wake(_) => unreachable!("wake marks are folded before the turn offset guard"),
        }
    }

    /// Opens a question call. The oldest answered call leaves to make room,
    /// so only calls that wait count against the bound; `false` when the
    /// bound is crossed anyway, which makes the turn unknown instead of
    /// dropping a waiting question.
    fn ask(
        &mut self,
        call: &str,
        content: &Option<UserTurnContent>,
        unreplied: Option<Vec<u32>>,
    ) -> bool {
        if self.questions.iter().any(|known| known.call == call) {
            return true;
        }
        if self.questions.len() == QUESTION_CALL_LIMIT
            && let Some(oldest) = self.questions.iter().position(|known| known.answered)
        {
            self.questions.remove(oldest);
        }
        if self.questions.len() == QUESTION_CALL_LIMIT || call.len() > NATIVE_ID_LIMIT_BYTES {
            self.question_capacity = true;
            self.questions.clear();
            return false;
        }
        self.questions.push(QuestionCall {
            call: call.to_owned(),
            content: content.clone(),
            answered: false,
            unreplied,
        });
        true
    }

    fn clear_questions(&mut self) {
        for question in &mut self.questions {
            question.answered = true;
            question.content = None;
        }
    }

    /// What the last turn leaves the operator to do; `None` when the records
    /// read do not settle it (a running turn whose mode was not plain, a
    /// finished plan whose turn's mode was not read or not known, a
    /// person's messages with no turn record at all, or an unreadable record
    /// since the turn started).
    pub fn waiting(&self) -> Option<Waiting> {
        if self.question_capacity || self.unreadable {
            return None;
        }
        if self.questions.iter().any(|question| !question.answered) {
            return Some(Waiting::Question);
        }
        if self.plan_hold.is_some() {
            return Some(Waiting::PlanApproval);
        }
        let Some(turn) = &self.last else {
            return (!self.unstructured).then_some(Waiting::Nothing);
        };
        // A turn that proposed no plan cannot end on the plan menu. One that
        // did waits when it ran in plan mode; in any other mode the records
        // are not the ones this rule was written for, so it is not known.
        match (turn.end, turn.mode) {
            (Some(_), _) if turn.answered => Some(Waiting::Nothing),
            (Some(End::Aborted), Mode::Plan | Mode::Other) => Some(Waiting::Nothing),
            (_, Mode::Other) | (Some(_), Mode::Unseen) if !turn.plan => Some(Waiting::Nothing),
            (Some(End::Completed), Mode::Plan) => Some(if turn.plan {
                Waiting::PlanApproval
            } else {
                Waiting::Nothing
            }),
            _ => None,
        }
    }

    /// The turn a mark names: the last one when the ids agree or the mark
    /// names none, otherwise a turn whose start was not read. That turn keeps
    /// a plan the last one left unanswered, so a record naming another turn
    /// never clears a wait it did not start a turn after.
    fn current(&mut self, id: &Option<String>) -> &mut Turn {
        let same = match (&self.last, id) {
            (Some(turn), Some(id)) => turn.id.as_ref().is_none_or(|known| known == id),
            (Some(_), None) => true,
            (None, _) => false,
        };
        if !same {
            let pending = self
                .last
                .as_ref()
                .is_some_and(|turn| turn.plan && !turn.answered);
            self.last = Some(Turn {
                id: id.clone(),
                mode: Mode::Unseen,
                plan: pending,
                plan_content: None,
                end: None,
                answered: false,
            });
        }
        self.last.as_mut().expect("a turn was just made")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: &str) -> Option<String> {
        Some(value.to_owned())
    }

    fn folded(marks: &[TurnMark]) -> TurnTracker {
        let mut tracker = TurnTracker::default();
        for (offset, mark) in marks.iter().enumerate() {
            tracker.fold(offset as u64 * 10, mark);
        }
        tracker
    }

    #[test]
    fn a_finished_plan_turn_waits_until_a_person_writes_or_a_turn_starts() {
        let plan = [
            TurnMark::Started {
                turn: id("t1"),
                mode: TurnMode::Plan,
            },
            TurnMark::Human,
            TurnMark::Plan { turn: id("t1") },
            TurnMark::Completed { turn: id("t1") },
        ];
        assert_eq!(folded(&plan).waiting(), Some(Waiting::PlanApproval));

        let mut answered = plan.to_vec();
        answered.push(TurnMark::Human);
        assert_eq!(folded(&answered).waiting(), Some(Waiting::Nothing));

        let mut next = plan.to_vec();
        next.push(TurnMark::Started {
            turn: id("t2"),
            mode: TurnMode::Other,
        });
        assert_eq!(folded(&next).waiting(), Some(Waiting::Nothing));
    }

    #[test]
    fn an_unreadable_record_is_unknown_until_a_turn_starts_or_a_person_writes() {
        let done = [
            TurnMark::Started {
                turn: id("t1"),
                mode: TurnMode::Other,
            },
            TurnMark::Completed { turn: id("t1") },
            TurnMark::Unreadable,
        ];
        assert_eq!(folded(&done).waiting(), None);
        for settles in [
            TurnMark::Started {
                turn: id("t2"),
                mode: TurnMode::Other,
            },
            TurnMark::Human,
            TurnMark::HumanTurn,
        ] {
            let mut next = done.to_vec();
            next.push(settles.clone());
            assert_eq!(
                folded(&next).waiting(),
                Some(Waiting::Nothing),
                "{settles:?}"
            );
        }
    }

    #[test]
    fn an_unsettled_question_waits_and_its_completed_record_answers_it() {
        let asked = TurnMark::Tools(vec![ToolTurnMark::Asked {
            call: "call-1".into(),
            content: None,
        }]);
        let mut tracker = TurnTracker::default();
        tracker.fold(0, &TurnMark::HumanTurn);
        tracker.fold_unsettled(1, &asked);
        // A second read of the same unfinished record changes nothing.
        tracker.fold_unsettled(1, &asked);
        assert_eq!(tracker.waiting(), Some(Waiting::Question));
        tracker.fold(
            1,
            &TurnMark::Tools(vec![ToolTurnMark::Answered {
                call: "call-1".into(),
            }]),
        );
        assert_eq!(tracker.waiting(), Some(Waiting::Nothing));
        // A settled record is never folded again as unsettled.
        tracker.fold_unsettled(1, &asked);
        assert_eq!(tracker.waiting(), Some(Waiting::Nothing));
    }

    #[test]
    fn a_plan_whose_turn_start_was_not_read_is_unknown() {
        let mut tracker = TurnTracker::default();
        tracker.fold(0, &TurnMark::Plan { turn: id("t1") });
        tracker.fold(10, &TurnMark::Completed { turn: id("t1") });
        assert_eq!(tracker.waiting(), None);
    }

    #[test]
    fn a_mark_already_folded_is_not_folded_again() {
        let mut tracker = folded(&[
            TurnMark::Started {
                turn: id("t1"),
                mode: TurnMode::Plan,
            },
            TurnMark::Plan { turn: id("t1") },
            TurnMark::Completed { turn: id("t1") },
        ]);
        // A reader resuming at an earlier anchor sees the person's message
        // that opened the turn again; it does not answer the plan.
        tracker.fold(5, &TurnMark::Human);
        assert_eq!(tracker.waiting(), Some(Waiting::PlanApproval));
    }

    #[test]
    fn records_the_tracker_does_not_know_never_read_as_nothing_waiting() {
        // A start whose mode is missing or unknown, then a proposed plan.
        let unknown_mode = folded(&[
            TurnMark::Started {
                turn: id("t1"),
                mode: TurnMode::Unknown,
            },
            TurnMark::Plan { turn: id("t1") },
            TurnMark::Completed { turn: id("t1") },
        ]);
        assert_eq!(unknown_mode.waiting(), None);
        // A plan proposed in a turn whose mode says it runs none.
        let plan_in_other = folded(&[
            TurnMark::Started {
                turn: id("t1"),
                mode: TurnMode::Other,
            },
            TurnMark::Plan { turn: id("t1") },
        ]);
        assert_eq!(plan_in_other.waiting(), None);
        let mut ended = plan_in_other.clone();
        ended.fold(100, &TurnMark::Completed { turn: id("t1") });
        assert_eq!(ended.waiting(), None);
        // A person's messages with no turn record read at all.
        assert_eq!(folded(&[TurnMark::Human]).waiting(), None);
        assert_eq!(folded(&[]).waiting(), Some(Waiting::Nothing));
        // A record naming another turn, with no start read, after a wait.
        for stray in [
            TurnMark::Completed { turn: id("t2") },
            TurnMark::Aborted { turn: id("t2") },
        ] {
            let mut tracker = folded(&[
                TurnMark::Started {
                    turn: id("t1"),
                    mode: TurnMode::Plan,
                },
                TurnMark::Plan { turn: id("t1") },
                TurnMark::Completed { turn: id("t1") },
            ]);
            tracker.fold(100, &stray);
            assert_eq!(tracker.waiting(), None, "{stray:?}");
        }
    }
    fn wake(marks: &[WakeMark]) -> TurnTracker {
        let mut tracker = TurnTracker::default();
        tracker.fold(0, &TurnMark::Wake(marks.to_vec()));
        tracker
    }

    fn started(id: &str) -> WakeMark {
        WakeMark::Started {
            id: id.into(),
            kind: None,
            call: None,
            expires_at_unix_ms: None,
        }
    }

    fn answered(call: &str, outcome: StopOutcome) -> WakeMark {
        WakeMark::Answered {
            call: call.into(),
            outcome,
        }
    }

    #[test]
    fn a_result_nobody_can_read_loses_the_devices_only_of_a_call_that_can_start_work() {
        let call = |call: &str| WakeMark::Call {
            call: call.into(),
            name: None,
        };
        let lost = |marks: &[WakeMark]| {
            let mut all = vec![WakeMark::Boot, started("bg1")];
            all.extend_from_slice(marks);
            wake(&all).wake_loss()
        };
        let unreadable = |call: &str| answered(call, StopOutcome::Unreadable);
        assert_eq!(lost(&[call("c1"), unreadable("c1")]), Some(WakeLoss::Lost));
        // A result of a call this reader never marked answers nothing it
        // tracks, and a readable result of one that did loses nothing.
        assert_eq!(lost(&[unreadable("c2")]), None);
        assert_eq!(
            lost(&[call("c1"), answered("c1", StopOutcome::Succeeded)]),
            None
        );
        assert_eq!(
            lost(&[call("c1"), answered("c1", StopOutcome::Failed)]),
            None
        );
        // Answered once: the same call is not open for a second result.
        assert_eq!(
            lost(&[call("c1"), unreadable("c1"), unreadable("c1")]),
            Some(WakeLoss::Lost)
        );
        // A call before the process began, or after devices were lost, is not
        // waited for: there is none to lose.
        let mut early = vec![call("c0")];
        early.extend([WakeMark::Boot, unreadable("c0")]);
        assert_eq!(wake(&early).wake_loss(), None);
    }

    #[test]
    fn open_calls_are_kept_across_a_save_and_bounded() {
        let mut tracker = wake(&[
            WakeMark::Boot,
            started("bg1"),
            WakeMark::Call {
                call: "c1".into(),
                name: None,
            },
            WakeMark::Stop {
                call: "c2".into(),
                id: "bg1".into(),
            },
        ]);
        let saved = serde_json::to_string(&tracker).unwrap();
        let mut restored: TurnTracker = serde_json::from_str(&saved).unwrap();
        assert_eq!(restored, tracker);
        restored.fold(
            10,
            &TurnMark::Wake(vec![answered("c2", StopOutcome::Succeeded)]),
        );
        assert_eq!(
            restored.work_records(0)[0].state,
            WorkState::Stopped,
            "the stop it saved"
        );
        restored.fold(
            20,
            &TurnMark::Wake(vec![answered("c1", StopOutcome::Unreadable)]),
        );
        assert_eq!(restored.wake_loss(), Some(WakeLoss::Lost));

        let calls: Vec<_> = (0..=WAKE_DEVICE_LIMIT)
            .map(|index| WakeMark::Call {
                call: format!("c{index}"),
                name: None,
            })
            .collect();
        tracker.fold(10, &TurnMark::Wake(calls));
        assert_eq!(tracker.wake_loss(), Some(WakeLoss::Capacity));
    }

    #[test]
    fn internal_work_distinguishes_unknown_empty_loss_expiry_and_bounded_history() {
        let mut tracker = TurnTracker::default();
        assert!(
            !tracker.work_known(),
            "no process boundary is not a confirmed zero"
        );
        tracker.fold(0, &TurnMark::Wake(vec![WakeMark::Boot]));
        assert!(tracker.work_known());
        assert!(tracker.work_records(0).is_empty());
        tracker.fold(
            1,
            &TurnMark::Wake(vec![WakeMark::Started {
                id: "monitor".into(),
                kind: Some(WorkKind::Background),
                call: None,
                expires_at_unix_ms: Some(10),
            }]),
        );
        assert_eq!(tracker.work_records(9)[0].state, WorkState::Running);
        assert_eq!(tracker.work_records(10)[0].state, WorkState::Expired);
        tracker.fold(2, &TurnMark::Wake(vec![WakeMark::Boot]));
        for offset in 0..=WAKE_DEVICE_LIMIT {
            let id = format!("work-{offset}");
            tracker.fold(
                offset as u64 + 3,
                &TurnMark::Wake(vec![started(&id), WakeMark::Ended { id }]),
            );
        }
        let records = tracker.work_records(0);
        assert_eq!(records.len(), WAKE_DEVICE_LIMIT);
        assert_eq!(records[0].id, "work-1");
        assert!(tracker.work_history_truncated());
        tracker.fold(100, &TurnMark::Wake(vec![WakeMark::Lost]));
        assert!(!tracker.work_known());
        assert!(tracker.work_records(0).is_empty());
        tracker.fold(101, &TurnMark::Wake(vec![WakeMark::Boot]));
        assert!(tracker.work_known());
        assert!(!tracker.work_history_truncated());
        assert!(
            tracker.work_records(0).is_empty(),
            "another process inherits no execution history"
        );
    }

    #[test]
    fn restored_internal_work_obeys_the_live_table_and_per_vector_caps() {
        let tracker = wake(&[WakeMark::Boot, started("bg1")]);
        let mut saved = serde_json::to_value(tracker).unwrap();
        saved["wake"] = serde_json::json!(
            (0..WAKE_DEVICE_LIMIT)
                .map(|n| serde_json::json!({"id":format!("bg{n}")}))
                .collect::<Vec<_>>()
        );
        saved["wake_calls"] = serde_json::json!([{"call":"overflow"}]);
        let mut restored: TurnTracker = serde_json::from_value(saved.clone()).unwrap();
        assert_eq!(restored.wake_loss(), Some(WakeLoss::Capacity));
        assert!(!restored.work_known());
        restored.fold(
            10,
            &TurnMark::Wake(vec![WakeMark::Ended { id: "bg1".into() }]),
        );
        assert_eq!(
            restored.wake_loss(),
            Some(WakeLoss::Capacity),
            "later records cannot revive an incomplete table"
        );
        saved["wake"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"id":"one-too-many"}));
        assert!(
            serde_json::from_value::<TurnTracker>(saved)
                .unwrap_err()
                .to_string()
                .contains("internal_work_capacity")
        );
        assert!(
            serde_json::from_value::<TurnTracker>(serde_json::json!({
                "booted": true, "ended_work": [{"id":"not-proven-live"}]
            }))
            .unwrap_err()
            .to_string()
            .contains("internal_work_history_running")
        );
    }

    /// Devices and the calls waiting for a result share one limit, so what is
    /// kept stays under it whichever of the two fills it.
    #[test]
    fn devices_and_open_calls_share_one_limit() {
        let calls = |count: usize| {
            (0..count).map(|index| WakeMark::Call {
                call: format!("c{index}"),
                name: None,
            })
        };
        let devices = |count: usize| (0..count).map(|index| started(&format!("bg{index}")));
        let loss = |marks: Vec<WakeMark>| {
            let mut all = vec![WakeMark::Boot];
            all.extend(marks);
            wake(&all).wake_loss()
        };
        let half = WAKE_DEVICE_LIMIT / 2;
        // Exactly the limit between them is kept, one more is not, in either order.
        assert_eq!(
            loss(
                calls(half)
                    .chain(devices(WAKE_DEVICE_LIMIT - half))
                    .collect()
            ),
            None
        );
        assert_eq!(
            loss(
                devices(WAKE_DEVICE_LIMIT - half)
                    .chain(calls(half))
                    .collect()
            ),
            None
        );
        assert_eq!(
            loss(calls(20).chain(devices(13)).collect()),
            Some(WakeLoss::Capacity)
        );
        assert_eq!(
            loss(devices(13).chain(calls(20)).collect()),
            Some(WakeLoss::Capacity)
        );
        // A result frees its call's place before the start it carries takes one.
        let mut all: Vec<_> = devices(WAKE_DEVICE_LIMIT - 1).collect();
        all.push(WakeMark::Call {
            call: "c".into(),
            name: None,
        });
        all.push(answered("c", StopOutcome::Succeeded));
        all.push(started("last"));
        assert_eq!(loss(all), None);
    }
}
