//! Changes to a machine's agent profiles, from Settings › Agents and from
//! `hide agent profile` (PRD agent-profiles D-05, D-10, D-11, D-24, D-30,
//! D-33; docs/ARCHITECTURE.md, Changing a machine's profiles).
//!
//! Both ways in share one path, so they check and refuse alike: the runtime
//! plans a change under its lock ([`Change`], judged against the profiles
//! the machine last listed and the agents that run with them), the
//! machine's node makes it with the lock released ([`Prepared::run`]), and
//! the list the node answers goes back into the snapshot under the lock.
//! The node checks every field (`hide_host::profiles`); the core decides
//! only what the node cannot see: which agents run with a profile and which
//! projects start with it.

use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use hide_agent_adapter::{AccountRef, ProfileId};
use hide_node_link::ErrorCode;
use hide_node_link::profiles::{
    ListedProfile, ProfileApplied, ProfileField, ProfileList, ProfileOp,
};
use hide_node_link::protocol::Call;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::node_access::{LinkError, NodeLink, call_as};

/// A field the machine's node refused, with its words: the detail is
/// `<field>: <text>` (`ProfileField`).
pub const REFUSED: &str = "profile_refused";
/// Agents run with the profile, sleeping ones included (D-11, B27); the
/// detail names them.
pub const IN_USE: &str = "profile_in_use";
/// The machine lists no such profile of the agent (B16).
pub const GONE: &str = crate::launch::PROFILE_GONE;
/// The built-in profile is neither edited nor removed (D-10, B28).
pub const BUILT_IN: &str = "profile_builtin";
/// The machine's node predates profiles (D-04).
pub const UNSUPPORTED: &str = crate::launch::PROFILES_UNSUPPORTED;
/// The machine's profiles have not been read yet.
pub const UNREAD: &str = "profiles_unread";
/// The machine did not answer.
pub const UNANSWERED: &str = crate::launch::PROFILE_UNANSWERED;
/// The command itself is malformed: a variable without `=`, an account for
/// an agent that keeps none apart.
pub const INVALID: &str = "profile_invalid";
/// Another change of profiles is under way; the same command runs after it.
pub const BUSY: &str = "profile_busy";

/// The refusals whose detail says what to fix.
pub const DETAILED: [&str; 4] = [REFUSED, IN_USE, GONE, INVALID];

/// What a refusal of a profile command tells its caller to do.
pub fn next_action(code: &str) -> &'static str {
    match code {
        REFUSED | INVALID => "Fix what the reason names and run the command again",
        IN_USE => "End or archive those agents, then remove the profile again",
        GONE => "hide agent profile list shows the machine's profiles",
        BUILT_IN => "The built-in profile cannot be changed; add a profile of your own",
        UNSUPPORTED => "Update Hide on that machine to keep profiles there",
        UNREAD | UNANSWERED => "Check that the machine is connected, then retry",
        BUSY => "Run the same command again in a moment",
        "machine_unknown" | "machine_unavailable" => {
            "Name a connected device with --machine (hide device list shows them)"
        }
        _ => "Retry; the reason is in Hide's diagnostic log",
    }
}

/// The longest variable or argument a command carries, past the node's own
/// limits (D-29), so a malformed command is refused before it is sent.
const TEXT_LIMIT: usize = 33 * 1024;
/// The most variables one command names, the node's own limit.
const ENV_LIMIT: usize = 64;

/// `hide agent profile …` (D-33): the profile named by its agent and name
/// on the caller's machine, or on `--machine`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProfileCommand {
    List {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
    },
    Add {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine: Option<String>,
        kind: String,
        name: String,
        /// A folder, or omp's profile name, by the agent's `[account] by`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        program: Option<String>,
        /// `KEY=VALUE`, in order.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        env: Vec<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        default: bool,
    },
    Edit {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine: Option<String>,
        kind: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rename: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        program: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        no_program: bool,
        /// `KEY=VALUE`: set, replacing a variable of that name.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        env: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        unset_env: Vec<String>,
    },
    Default {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine: Option<String>,
        kind: String,
        name: String,
    },
    Remove {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine: Option<String>,
        kind: String,
        name: String,
    },
    Reinstall {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine: Option<String>,
        kind: String,
        name: String,
    },
}

impl ProfileCommand {
    /// Whether the command's words are within what any answer could use.
    pub fn valid(&self) -> bool {
        let text = |value: &str| value.len() <= TEXT_LIMIT && !value.contains('\0');
        let short = |value: &str| !value.is_empty() && value.len() <= 256 && text(value);
        let machine_ok = |machine: &Option<String>| machine.as_deref().is_none_or(short);
        match self {
            Self::List { machine, kind } => {
                machine_ok(machine) && kind.as_deref().is_none_or(short)
            }
            Self::Add {
                machine,
                kind,
                name,
                account,
                program,
                env,
                ..
            } => {
                machine_ok(machine)
                    && short(kind)
                    && text(name)
                    && account.as_deref().is_none_or(text)
                    && program.as_deref().is_none_or(text)
                    && env.len() <= ENV_LIMIT
                    && env.iter().all(|pair| text(pair))
            }
            Self::Edit {
                machine,
                kind,
                name,
                rename,
                program,
                env,
                unset_env,
                ..
            } => {
                machine_ok(machine)
                    && short(kind)
                    && text(name)
                    && rename.as_deref().is_none_or(text)
                    && program.as_deref().is_none_or(text)
                    && env.len() <= ENV_LIMIT
                    && unset_env.len() <= ENV_LIMIT
                    && env.iter().chain(unset_env).all(|pair| text(pair))
            }
            Self::Default {
                machine,
                kind,
                name,
            }
            | Self::Remove {
                machine,
                kind,
                name,
            }
            | Self::Reinstall {
                machine,
                kind,
                name,
            } => machine_ok(machine) && short(kind) && text(name),
        }
    }

    /// The machine the command names, if any.
    pub(crate) fn machine(&self) -> Option<&str> {
        match self {
            Self::List { machine, .. }
            | Self::Add { machine, .. }
            | Self::Edit { machine, .. }
            | Self::Default { machine, .. }
            | Self::Remove { machine, .. }
            | Self::Reinstall { machine, .. } => machine.as_deref(),
        }
    }
}

/// A change Settings › Agents asks for, by the profile's id on the machine
/// the page shows (`agent_profile_change`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ScreenChange {
    Add {
        kind: String,
        name: String,
        #[serde(default)]
        account: Option<String>,
        #[serde(default)]
        program: Option<String>,
        #[serde(default)]
        env: Vec<(String, String)>,
        #[serde(default)]
        default: bool,
    },
    /// The edit form's whole state: its name, program and variables.
    Edit {
        id: String,
        name: String,
        #[serde(default)]
        program: Option<String>,
        #[serde(default)]
        env: Vec<(String, String)>,
    },
    Default {
        kind: String,
        id: String,
    },
    /// What a removal would meet, asked before the confirmation opens: the
    /// agents that run with it, or the projects that start with it.
    RemoveCheck {
        id: String,
    },
    Remove {
        id: String,
    },
    Reinstall {
        id: String,
    },
    /// The profile's variables with their values, for the edit form.
    Values {
        id: String,
    },
}

/// How an edit changes a profile's variables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EnvEdit {
    /// The form's whole list.
    Replace(Vec<(String, String)>),
    /// The CLI's: each `set` replaces a variable of its name or joins the
    /// end, each `unset` leaves.
    Patch {
        set: Vec<(String, String)>,
        unset: Vec<String>,
    },
}

/// A change planned under the runtime lock, the profile named by its id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    List,
    Values {
        id: ProfileId,
    },
    Add {
        kind: String,
        name: String,
        account: Option<AccountRef>,
        program: Option<String>,
        env: Vec<(String, String)>,
        default: bool,
    },
    Edit {
        id: ProfileId,
        name: Option<String>,
        /// `Some(None)` takes the program away.
        program: Option<Option<String>>,
        env: EnvEdit,
    },
    Default {
        kind: String,
        id: ProfileId,
    },
    Remove {
        id: ProfileId,
        /// The projects that start with it, which go back to the agent's
        /// default once it is gone (D-31).
        projects: Vec<String>,
    },
    Reinstall {
        id: ProfileId,
    },
}

/// One agent that runs with a profile, as a refused removal lists it (B27).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct InUse {
    pub pane_id: String,
    pub title: String,
    pub place: String,
    pub asleep: bool,
}

/// What the screen's last profile request came to
/// (`status.agent_profile_answer`): the request id it carried and the
/// machine, then its outcome.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProfileAnswer {
    pub request_id: String,
    pub device_id: String,
    pub state: AnswerState,
    /// The refusal's code, one of this module's or a machine's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// The field a node's refusal is about.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<ProfileField>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_key: Option<String>,
    /// The node's words for a field it refused, for a node that sends no
    /// `reason`; the screen words every other code itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Why the node refused, which the screen words in its language.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<hide_node_link::profiles::ProfileReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub in_use: Vec<InUse>,
    /// The names of the projects that start with the profile.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<String>,
    /// The profile's variables with their values, for a `values` request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<(String, String)>>,
    /// The profile an `add` made.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub added: Option<String>,
}

/// Whether the request is still running (`contracts/snapshot-wire-enums.json`:
/// `profile_answer_state`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerState {
    Pending,
    Done,
    Refused,
}

impl ProfileAnswer {
    pub(crate) fn pending(request_id: &str, device_id: &str) -> Self {
        Self {
            request_id: request_id.to_owned(),
            device_id: device_id.to_owned(),
            state: AnswerState::Pending,
            code: None,
            field: None,
            env_key: None,
            text: None,
            reason: None,
            limit: None,
            in_use: Vec::new(),
            projects: Vec::new(),
            values: None,
            added: None,
        }
    }

    pub(crate) fn refused(mut self, refusal: &Refusal) -> Self {
        self.state = AnswerState::Refused;
        self.code = Some(refusal.code.to_owned());
        if let Some(field) = &refusal.field {
            self.field = Some(field.field);
            self.env_key.clone_from(&field.env_key);
            self.text = Some(field.text.clone());
            self.reason = field.reason;
            self.limit = field.limit;
        }
        self.in_use.clone_from(&refusal.in_use);
        self
    }
}

/// Why a profile command was refused, for the CLI and for the screen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Refusal {
    pub(crate) code: &'static str,
    /// Boxed: a refusal travels in every `Err`, and most carry none.
    pub(crate) field: Option<Box<hide_node_link::profiles::ProfileRefusal>>,
    pub(crate) in_use: Vec<InUse>,
    /// What the CLI's line names after the code.
    pub(crate) detail: Option<String>,
}

impl Refusal {
    pub(crate) fn code(code: &'static str) -> Self {
        Self {
            code,
            field: None,
            in_use: Vec::new(),
            detail: None,
        }
    }

    pub(crate) fn detailed(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            detail: Some(detail.into()),
            ..Self::code(code)
        }
    }

    fn of_node(refusal: hide_node_link::profiles::ProfileRefusal) -> Self {
        let field = serde_json::to_value(refusal.field)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default();
        Self {
            code: REFUSED,
            detail: Some(format!("{field}: {}", refusal.text)),
            field: Some(Box::new(refusal)),
            in_use: Vec::new(),
        }
    }

    pub(crate) fn in_use(agents: Vec<InUse>) -> Self {
        let names = agents
            .iter()
            .map(|agent| {
                let asleep = if agent.asleep { ", asleep" } else { "" };
                format!("{} ({}{asleep})", agent.title, agent.place)
            })
            .collect::<Vec<_>>()
            .join("; ");
        Self {
            code: IN_USE,
            field: None,
            detail: Some(names),
            in_use: agents,
        }
    }

    /// The refusal as a delivery refusal string, `code` and its detail.
    pub(crate) fn line(&self) -> String {
        match &self.detail {
            Some(detail) if DETAILED.contains(&self.code) => {
                crate::coordination::refusal(self.code, detail)
            }
            _ => self.code.to_owned(),
        }
    }
}

/// A list of profiles is a file read and the kit's judgment of each.
const LIST_TIMEOUT: Duration = Duration::from_secs(30);
/// A change installs or takes out Hide's pieces in the account it touches.
const APPLY_TIMEOUT: Duration = Duration::from_secs(90);

/// A change planned under the lock, to be run without it.
pub struct Prepared {
    pub(crate) machine: String,
    pub(crate) link: Arc<dyn NodeLink>,
    pub(crate) change: Change,
    /// The list's version the plan was made against.
    pub(crate) version: u64,
    /// The agent the command's listing is narrowed to.
    pub(crate) kind: Option<String>,
    /// The screen request it answers, if any.
    pub(crate) request: Option<String>,
    pub(crate) runtime: Weak<Mutex<crate::runtime::Runtime>>,
    pub(crate) notifier: crate::handle::ChangeNotifier,
    /// Set once the outcome reached the runtime; a run that ends without
    /// one gives the slot back on drop.
    pub(crate) settled: bool,
}

/// What a node answered for a change.
pub(crate) struct Outcome {
    pub(crate) list: Option<ProfileList>,
    pub(crate) result: Result<Done, Refusal>,
}

/// A change the node made.
#[derive(Default)]
pub(crate) struct Done {
    pub(crate) added: Option<ProfileId>,
    pub(crate) values: Option<Vec<(String, String)>>,
}

impl Prepared {
    /// Runs the change on the machine's node, with no lock held, and hands
    /// the list it answers back to the runtime. Returns what the CLI prints.
    pub fn run(mut self) -> Result<Value, String> {
        let outcome = self.call();
        let answer = self.answer(&outcome);
        let Some(runtime) = self.runtime.upgrade() else {
            self.settled = true;
            return Err(UNANSWERED.to_owned());
        };
        let changed = runtime
            .lock()
            .map(|mut runtime| runtime.settle_agent_profiles(&self, &outcome))
            .unwrap_or(false);
        self.settled = true;
        if changed {
            self.notifier.notify();
        }
        answer
    }

    fn call(&self) -> Outcome {
        let link = self.link.as_ref();
        match &self.change {
            Change::List => match list(link, false) {
                Ok(list) => Outcome {
                    list: Some(list),
                    result: Ok(Done::default()),
                },
                Err(refusal) => Outcome {
                    list: None,
                    result: Err(refusal),
                },
            },
            Change::Values { id } => match list(link, true) {
                Ok(mut list) => {
                    let values = list
                        .profiles
                        .iter_mut()
                        .find(|profile| profile.id == *id)
                        .and_then(|profile| profile.env.take());
                    for profile in &mut list.profiles {
                        profile.env = None;
                    }
                    let result = match values {
                        Some(values) => Ok(Done {
                            values: Some(values),
                            added: None,
                        }),
                        None if list.profiles.iter().any(|profile| profile.id == *id) => Ok(Done {
                            values: Some(Vec::new()),
                            added: None,
                        }),
                        None => Err(Refusal::detailed(GONE, id.0.clone())),
                    };
                    Outcome {
                        list: Some(list),
                        result,
                    }
                }
                Err(refusal) => Outcome {
                    list: None,
                    result: Err(refusal),
                },
            },
            Change::Reinstall { id } => {
                match call_as::<ProfileList>(
                    link,
                    Call::ProfileInstall {
                        profile: id.clone(),
                    },
                    APPLY_TIMEOUT,
                ) {
                    Ok(list) => Outcome {
                        list: Some(list),
                        result: Ok(Done::default()),
                    },
                    Err(LinkError::Refused(error)) if error.code == ErrorCode::NotFound => {
                        Outcome {
                            list: list(link, false).ok(),
                            result: Err(Refusal::detailed(GONE, id.0.clone())),
                        }
                    }
                    Err(error) => self.unanswered(link, &error),
                }
            }
            Change::Edit { id, .. } => {
                // An edit names the whole profile to the node, so it starts
                // from the variables the machine keeps now.
                let current = match list(link, true) {
                    Ok(current) => current,
                    Err(refusal) => {
                        return Outcome {
                            list: None,
                            result: Err(refusal),
                        };
                    }
                };
                let Some(profile) = current.profiles.iter().find(|profile| profile.id == *id)
                else {
                    return Outcome {
                        list: Some(scrubbed(current)),
                        result: Err(Refusal::detailed(GONE, id.0.clone())),
                    };
                };
                let op = edited(profile, &self.change);
                self.apply(op, current.version)
            }
            Change::Add {
                kind,
                name,
                account,
                program,
                env,
                default,
            } => self.apply(
                ProfileOp::Add {
                    kind: kind.clone(),
                    name: name.clone(),
                    account: account.clone(),
                    program: program.clone(),
                    env: env.clone(),
                    default: *default,
                },
                self.version,
            ),
            Change::Default { kind, id } => self.apply(
                ProfileOp::Default {
                    kind: kind.clone(),
                    id: id.clone(),
                },
                self.version,
            ),
            Change::Remove { id, .. } => {
                self.apply(ProfileOp::Remove { id: id.clone() }, self.version)
            }
        }
    }

    /// One change of the list, made again once on a list read anew when the
    /// list moved since the plan (D-30): the change names its profile by
    /// id, so it means the same on the newer list.
    fn apply(&self, op: ProfileOp, version: u64) -> Outcome {
        let link = self.link.as_ref();
        let mut expected = version;
        for attempt in 0..2 {
            let call = Call::ProfileApply {
                change: op.clone(),
                expected_version: expected,
            };
            match call_as::<ProfileApplied>(link, call, APPLY_TIMEOUT) {
                Ok(applied) => {
                    return Outcome {
                        result: match applied.refused {
                            Some(refused) => Err(Refusal::of_node(refused)),
                            None => Ok(Done {
                                added: applied.added,
                                values: None,
                            }),
                        },
                        list: Some(applied.list),
                    };
                }
                Err(LinkError::Refused(error))
                    if error.code == ErrorCode::Conflict && attempt == 0 =>
                {
                    match list(link, false) {
                        Ok(list) => expected = list.version,
                        Err(refusal) => {
                            return Outcome {
                                list: None,
                                result: Err(refusal),
                            };
                        }
                    }
                }
                Err(error) => return self.unanswered(link, &error),
            }
        }
        // A list that moved twice while one change ran: read as it is now.
        Outcome {
            list: list(link, false).ok(),
            result: Err(Refusal::code(BUSY)),
        }
    }

    /// A node that did not answer: the list is read again, so the screen
    /// shows what the machine holds now (D-30), and why goes to the log.
    fn unanswered(&self, link: &dyn NodeLink, error: &LinkError) -> Outcome {
        crate::diagnostic!(json!({
            "component": "agent_profiles",
            "kind": "profiles.unanswered",
            "device_id": self.machine,
            "reason": error.to_string(),
        }));
        Outcome {
            list: list(link, false).ok(),
            result: Err(match error {
                LinkError::Refused(refusal) if refusal.code == ErrorCode::Unsupported => {
                    Refusal::code(UNSUPPORTED)
                }
                _ => Refusal::code(UNANSWERED),
            }),
        }
    }

    /// What the CLI prints for the outcome.
    fn answer(&self, outcome: &Outcome) -> Result<Value, String> {
        let done = outcome.result.as_ref().map_err(Refusal::line)?;
        let list = outcome.list.as_ref();
        let listed = |id: &ProfileId| {
            list.and_then(|list| list.profiles.iter().find(|profile| profile.id == *id))
                .map(|profile| {
                    listed(
                        profile,
                        list.and_then(|list| list.defaults.get(&profile.kind)) == Some(&profile.id),
                    )
                })
        };
        Ok(match &self.change {
            Change::List => {
                let Some(list) = list else {
                    return Err(UNANSWERED.to_owned());
                };
                json!({
                    "machine": self.machine,
                    "profiles": listing(list, self.kind.as_deref()),
                })
            }
            Change::Values { id } => json!({"id": id.0, "env": done.values}),
            Change::Add { .. } => {
                json!({"machine": self.machine, "profile": done.added.as_ref().and_then(listed)})
            }
            Change::Edit { id, .. } | Change::Default { id, .. } | Change::Reinstall { id } => {
                json!({"machine": self.machine, "profile": listed(id)})
            }
            Change::Remove { id, projects } => json!({
                "machine": self.machine,
                "removed": id.0,
                "projects_back_to_default": projects,
            }),
        })
    }
}

impl Drop for Prepared {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        if let Some(runtime) = self.runtime.upgrade()
            && let Ok(mut runtime) = runtime.lock()
        {
            runtime.abandon_agent_profiles(self.request.as_deref());
        }
        self.notifier.notify();
    }
}

fn list(link: &dyn NodeLink, values: bool) -> Result<ProfileList, Refusal> {
    match call_as::<ProfileList>(link, Call::ProfileList { values }, LIST_TIMEOUT) {
        Ok(list) => Ok(list),
        Err(LinkError::Refused(refusal)) if refusal.code == ErrorCode::Unsupported => {
            Err(Refusal::code(UNSUPPORTED))
        }
        Err(error) => {
            crate::diagnostic!(json!({
                "component": "agent_profiles",
                "kind": "profiles.unanswered",
                "reason": error.to_string(),
            }));
            Err(Refusal::code(UNANSWERED))
        }
    }
}

/// A list with no variable's value left in it.
fn scrubbed(mut list: ProfileList) -> ProfileList {
    for profile in &mut list.profiles {
        profile.env = None;
    }
    list
}

/// The node's edit for `change`, made from the profile as it is now: what
/// the change leaves alone is sent as it was.
fn edited(profile: &ListedProfile, change: &Change) -> ProfileOp {
    let Change::Edit {
        id,
        name,
        program,
        env,
    } = change
    else {
        unreachable!("an edit is planned from an edit");
    };
    let current = profile.env.clone().unwrap_or_default();
    let env = match env {
        EnvEdit::Replace(env) => env.clone(),
        EnvEdit::Patch { set, unset } => {
            let mut env: Vec<(String, String)> = current
                .into_iter()
                .filter(|(key, _)| !unset.contains(key))
                .collect();
            for (key, value) in set {
                match env.iter_mut().find(|(existing, _)| existing == key) {
                    Some(pair) => pair.1.clone_from(value),
                    None => env.push((key.clone(), value.clone())),
                }
            }
            env
        }
    };
    ProfileOp::Edit {
        id: id.clone(),
        name: name.clone().unwrap_or_else(|| profile.name.clone()),
        program: program.clone().unwrap_or_else(|| profile.program.clone()),
        env,
    }
}

/// The machine's profiles as `hide agent profile list` prints them, each
/// agent's built-in profile first.
fn listing(list: &ProfileList, kind: Option<&str>) -> Vec<Value> {
    let agents: Vec<&str> = match kind {
        Some(kind) => vec![kind],
        None => crate::agent_profiles::agents().collect(),
    };
    let mut rows = Vec::new();
    for agent in agents {
        let builtin = hide_agent_adapter::Profile::builtin_default(agent);
        rows.push(json!({
            "kind": agent,
            "name": builtin.name,
            "id": builtin.id.0,
            "built_in": true,
            "default": !list.defaults.contains_key(agent),
        }));
        for profile in list.profiles.iter().filter(|profile| profile.kind == agent) {
            rows.push(listed(
                profile,
                list.defaults.get(agent) == Some(&profile.id),
            ));
        }
    }
    rows
}

/// One profile as the CLI prints it: no variable's value, and its pieces'
/// state as the kit last judged them.
fn listed(profile: &ListedProfile, default: bool) -> Value {
    let account = profile.account.as_ref().map(|account| match account {
        AccountRef::Folder(text) | AccountRef::Name(text) => text.clone(),
    });
    json!({
        "kind": profile.kind,
        "name": profile.name,
        "id": profile.id.0,
        "default": default,
        "account": account,
        "program": profile.program,
        "env_keys": profile.env_keys,
        "removing": profile.removing,
        "install": profile.place,
    })
}

/// The agents a machine keeps profiles for, in the start panel's order:
/// every agent with an account rule, Cursor aside (D-25).
pub(crate) fn agents() -> impl Iterator<Item = &'static str> {
    hide_agent_adapter::ADAPTERS
        .iter()
        .filter(|adapter| adapter.start.is_some())
        .filter(|adapter| hide_agent_adapter::definition::builtin_account(adapter.id).is_some())
        .map(|adapter| adapter.id)
}

/// `KEY=VALUE` as a pair; refused without `=` or with an empty key.
pub(crate) fn pair(text: &str) -> Result<(String, String), Refusal> {
    match text.split_once('=') {
        Some((key, value)) if !key.is_empty() => Ok((key.to_owned(), value.to_owned())),
        _ => Err(Refusal::detailed(
            INVALID,
            format!(
                "--env takes KEY=VALUE, not {}",
                text.chars().take(64).collect::<String>()
            ),
        )),
    }
}

/// The account `text` names for the agent `kind`, by its `[account] by`:
/// a folder, an omp profile name, or none for an agent that keeps no
/// accounts apart (D-14).
pub(crate) fn account(kind: &str, text: Option<String>) -> Result<Option<AccountRef>, Refusal> {
    use hide_agent_adapter::definition::By;
    let Some(text) = text.filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    match hide_agent_adapter::definition::builtin_account(kind).map(|rule| rule.by) {
        Some(By::Folder) => Ok(Some(AccountRef::Folder(text))),
        Some(By::Name) => Ok(Some(AccountRef::Name(text))),
        _ => Err(Refusal::detailed(
            INVALID,
            format!("{kind} keeps no accounts apart, so its profile takes no --account"),
        )),
    }
}

#[cfg(test)]
mod tests;
