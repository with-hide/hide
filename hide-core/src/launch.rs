//! The one way an agent starts in a pane, with the profile it runs with
//! (PRD agent-profiles D-26, D-27, D-32; docs/ARCHITECTURE.md, Starting an
//! agent with a profile).
//!
//! Every start names its agent and its profile ([`Request`]): a new start
//! picks the profile ([`pick`]); an agent that comes back (a resume, a wake,
//! a reopen, a fork, an archive's resume, a restore) names the one its
//! record keeps ([`LaunchProfile`]), and a record written before profiles
//! names none, which is the built-in default. [`resolve`] turns the request
//! into the [`Launch`] an engine start takes (`engine::AgentLaunch::launch`),
//! and nothing else makes one. The built-in default starts as every agent
//! always has, its line unchanged and nothing asked of its machine (D-19,
//! B29). Another profile is asked of its machine's node
//! (`Call::ProfileLaunch`), which answers its variables and program as it
//! checks them now, so an edit counts from the next start (D-11), a removed
//! profile is refused by name (D-27, B20), and a program that is not there
//! refuses the start before anything is typed (B22). Its start line runs
//! `hide agent exec` with the `hide` beside that node and names only the
//! start's attempt and the agent's own words: the executor asks for the
//! profile's variables and program just before it runs ([`exec_start`]), at
//! the version this start was checked with, so they reach the agent's
//! process alone, never the pane's shell or its history (D-26, B21).

use std::time::Duration;

use hide_agent_adapter::{AccountKey, LaunchTarget, ProfileId};
use hide_node_link::ErrorCode;
use hide_node_link::profiles::{ProfileLaunch, ProfileStart};
use serde::{Deserialize, Serialize};

use crate::model::ProfilesSnapshot;
use crate::node_access::{LinkError, NodeLink};

pub(crate) mod holds;

/// How long a start waits for its machine to answer its profile: a read of
/// one small file and a look for one program.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(10);
/// How long an executor's ask for its profile waits on the machine
/// ([`exec_start`]): inside the executor's own wait for its answer
/// (`hided::workspace_cli::AGENT_EXEC_WITHIN`, ten seconds).
const EXEC_TIMEOUT: Duration = Duration::from_secs(5);
/// The most an executor's start may carry
/// ([`hide_agent_hooks::hook_request::ExecStart`]): its answer has to fit
/// one hook answer, with room for the answer's own words.
const EXEC_START_LIMIT: usize = hide_agent_hooks::hook_request::HOOK_RESPONSE_LIMIT - 4096;

/// The profile an agent's record keeps of its start (D-27): its id, its name
/// then, which a refusal names once the profile is gone, and the account it
/// ran in. A record with none ran with the built-in default profile, as did
/// every agent recorded before profiles.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LaunchProfile {
    pub id: ProfileId,
    pub name: String,
    pub account: AccountKey,
}

impl LaunchProfile {
    /// The account an agent run with `profile` reads its sessions in: the
    /// default account for the built-in default (`None`).
    pub(crate) fn account_of(profile: Option<&Self>) -> AccountKey {
        profile.map_or_else(AccountKey::default, |profile| profile.account.clone())
    }
}

/// `name`, a profile of the `kind` agent, as the operator reads it named
/// after its agent (`Claude · work`): `hide agent list`, `show` and the start
/// line say it so; a row carries the name alone, for the mark beside its logo.
pub(crate) fn shown(kind: &str, name: &str) -> String {
    format!("{} · {name}", label(kind))
}

/// What a start asks for before its machine is asked: the agent and the
/// profile it runs with ([`LaunchTarget`]), and the name the start knew the
/// profile by, which a refusal names; none for the built-in default.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Request {
    pub(crate) target: LaunchTarget,
    name: Option<String>,
    /// The start's hold on its profile, taken under the runtime lock
    /// ([`holds`]); none for the built-in default and for a start that
    /// brings back an agent whose record already holds the profile.
    hold: Option<holds::Hold>,
    choice: hide_agent_adapter::LaunchChoice,
}

impl Request {
    /// A start of `kind` with the profile `profile`, by id with its name;
    /// `None`, or the built-in default's id, for the built-in default.
    pub(crate) fn new(kind: &str, profile: Option<(ProfileId, String)>) -> Self {
        let profile = profile.filter(|(id, _)| !id.is_default());
        Self {
            target: LaunchTarget {
                kind: kind.to_owned(),
                profile: profile
                    .as_ref()
                    .map_or_else(ProfileId::default_profile, |(id, _)| id.clone()),
            },
            name: profile.map(|(_, name)| name),
            hold: None,
            choice: Default::default(),
        }
    }

    pub(crate) fn with_choice(mut self, choice: &hide_agent_adapter::LaunchChoice) -> Self {
        self.choice = choice.clone();
        self
    }

    pub(crate) fn choice(&self) -> &hide_agent_adapter::LaunchChoice {
        &self.choice
    }

    /// The start's hold on its profile.
    pub(crate) fn hold(&self) -> Option<holds::Hold> {
        self.hold.clone()
    }

    /// This start, holding its profile with `hold`.
    pub(crate) fn held(mut self, hold: Option<holds::Hold>) -> Self {
        self.hold = hold;
        self
    }

    /// A start of `kind` with its built-in default profile.
    #[cfg(test)]
    pub(crate) fn default_of(kind: &str) -> Self {
        Self::new(kind, None)
    }

    /// A start of `kind` with the profile its record keeps, the built-in
    /// default when it keeps none.
    pub(crate) fn recorded(kind: &str, profile: Option<&LaunchProfile>) -> Self {
        Self::new(
            kind,
            profile.map(|profile| (profile.id.clone(), profile.name.clone())),
        )
    }

    pub(crate) fn kind(&self) -> &str {
        &self.target.kind
    }

    /// The profile by id with its name; `None` for the built-in default.
    pub(crate) fn profile(&self) -> Option<(ProfileId, String)> {
        Some((self.target.profile.clone(), self.name.clone()?))
    }
}

/// A start its machine answered for: the only thing an engine start takes
/// (`engine::AgentLaunch::launch`), made by [`resolve`] alone.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Launch {
    kind: String,
    profile: Option<(ProfileId, ProfileStart)>,
    hold: Option<holds::Hold>,
}

impl Launch {
    /// The agent, in the spelling the start was asked with.
    pub(crate) fn kind(&self) -> &str {
        &self.kind
    }

    pub(crate) fn profile_facts(&self) -> Option<(&AccountKey, u64)> {
        self.profile
            .as_ref()
            .map(|(_, start)| (&start.account, start.version))
    }

    pub(crate) fn finish(&self) -> holds::Finish {
        holds::Finish::new(self.hold.clone())
    }

    /// What the profile's machine answered for it, `None` for the built-in
    /// default: what a program the start runs before the agent's own (an
    /// OpenCode fork's) runs with too.
    pub(crate) fn start(&self) -> Option<&ProfileStart> {
        self.profile.as_ref().map(|(_, start)| start)
    }

    /// What the started agent's record keeps, `None` for the built-in
    /// default.
    pub(crate) fn record(&self) -> Option<LaunchProfile> {
        self.profile.as_ref().map(|(id, start)| LaunchProfile {
            id: id.clone(),
            name: start.name.clone(),
            account: start.account.clone(),
        })
    }

    /// `file`, what the node read the agent's own program to start from
    /// (`engine::StartProgram`, npm's launcher and the script it runs on
    /// Windows), unless the profile runs its own program in the agent's
    /// place: that program then takes the agent's arguments alone, never the
    /// words the agent's program started with (D-26).
    pub(crate) fn agent_file<'a, T>(&self, file: Option<&'a T>) -> Option<&'a T> {
        let wraps = self
            .profile
            .as_ref()
            .is_some_and(|(_, start)| start.program.is_some());
        file.filter(|_| !wraps)
    }

    /// `argv`, the agent's own start, as the pane's shell runs it: itself
    /// for the built-in default, with no attempt; else the `hide` beside the
    /// pane's node, `agent exec`, a new attempt
    /// (`hide_agent_hooks::hook_request::AGENT_EXEC_ATTEMPT`) first, and
    /// `argv` after `--` (`hided::agent_exec`). The profile's variables and
    /// program are not on it: the attempt keeps which profile, at which
    /// version, this start was checked with, and the executor asks for them
    /// ([`exec_start`]).
    pub(crate) fn line(&self, argv: Vec<String>) -> Result<StartLine, String> {
        let Some((profile, start)) = &self.profile else {
            return Ok(StartLine {
                words: argv,
                attempt: None,
            });
        };
        let id = hide_agent_hooks::hook_request::fresh_attempt()
            .map_err(|why| format!("the start's attempt could not be named: {why}"))?;
        let mut line = vec![
            start.exec.clone(),
            "agent".to_owned(),
            "exec".to_owned(),
            hide_agent_hooks::hook_request::AGENT_EXEC_ATTEMPT.to_owned(),
            id.clone(),
            "--".to_owned(),
        ];
        line.extend(argv);
        Ok(StartLine {
            words: line,
            attempt: Some(ExecAttempt {
                id,
                kind: self.kind.clone(),
                profile: profile.clone(),
                name: start.name.clone(),
                version: start.version,
            }),
        })
    }

    /// A start of `kind` with the built-in default profile, as a test's
    /// engine takes it.
    #[cfg(test)]
    pub(crate) fn default_of(kind: &str) -> Self {
        Self {
            kind: kind.to_owned(),
            profile: None,
            hold: None,
        }
    }

    /// A start of `kind` with the profile `id` as its machine answered it,
    /// for a test that types its line.
    #[cfg(test)]
    pub(crate) fn answered(kind: &str, id: &str, start: ProfileStart) -> Self {
        Self {
            kind: kind.to_owned(),
            profile: Some((ProfileId(id.to_owned()), start)),
            hold: None,
        }
    }
}

/// A start's line as the pane's shell runs it, and the attempt a profile's
/// start runs under (`engine::own::attempt`); none for the built-in
/// default, which waits on nothing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StartLine {
    pub(crate) words: Vec<String>,
    pub(crate) attempt: Option<ExecAttempt>,
}

/// The attempt a profile's start runs under, and what its executor's ask
/// is answered from ([`exec_start`]): the profile of the agent `kind`, by
/// the name a refusal gives it, at the version its machine answered when
/// the start was checked.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExecAttempt {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) profile: ProfileId,
    pub(crate) name: String,
    pub(crate) version: u64,
}

/// Why a start with a profile was refused before anything was typed: a
/// stable `code`, and the sentence for the operator, which names the agent,
/// the profile and, when it is the cause, its program.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Refusal {
    pub(crate) code: &'static str,
    pub(crate) detail: String,
}

impl From<Refusal> for String {
    fn from(refusal: Refusal) -> Self {
        format!("{}: {}", refusal.code, refusal.detail)
    }
}

impl From<Refusal> for crate::agent_start::StartError {
    fn from(refusal: Refusal) -> Self {
        Self::NotStarted {
            reason: refusal.code.into(),
            detail: refusal.detail,
        }
    }
}

/// The profile is gone from its machine (B20).
pub(crate) const PROFILE_GONE: &str = "profile_gone";
/// The machine's node predates profiles: only the built-in default starts
/// there (B22).
pub(crate) const PROFILES_UNSUPPORTED: &str = "profiles_unsupported";
/// The machine refused the profile now: being removed, another agent's, an
/// account or variable that no longer resolves, or a program it cannot find.
pub(crate) const PROFILE_REFUSED: &str = "profile_refused";
/// The machine did not answer for the profile.
pub(crate) const PROFILE_UNANSWERED: &str = "profile_unanswered";
/// As many launch records are kept, or held by starts under way, as the core
/// keeps (`runtime::agent_choice::PANE_LAUNCH_LIMIT`): a start with a profile
/// is refused before it runs rather than run with no record of its account.
pub(crate) const LAUNCH_RECORDS_FULL: &str = "launch_records_full";
/// A profile's start was typed and nothing says whether its program runs
/// (`engine::own::attempt`): neither that it ran nor that it did not, and
/// nothing starts again on it.
pub(crate) const PROFILE_UNCONFIRMED: &str = "profile_unconfirmed";

/// Whether an engine's refusal `code` is what came of a profile's typed
/// start ([`PROFILE_REFUSED`] from its executor, [`PROFILE_UNCONFIRMED`]):
/// a caller that would otherwise start again in another way (a reopen's new
/// conversation) does not.
pub(crate) fn ends_a_start(code: &str) -> bool {
    code == PROFILE_REFUSED || code == PROFILE_UNCONFIRMED
}

/// Resolves `request` on the machine `node` runs, with the runtime lock
/// released: the built-in default at once, another profile as its machine
/// answers for it now.
pub(crate) fn resolve(node: Option<&dyn NodeLink>, request: &Request) -> Result<Launch, Refusal> {
    let kind = request.kind();
    let Some((id, name)) = request.profile() else {
        return Ok(Launch {
            kind: kind.to_owned(),
            profile: None,
            hold: None,
        });
    };
    let start = profile_start(node, kind, &id, &name, LAUNCH_TIMEOUT)?;
    Ok(Launch {
        kind: kind.to_owned(),
        profile: Some((id, start)),
        hold: request.hold.clone(),
    })
}

/// How the profile `id` of `kind`, known as `name`, starts on `node` now
/// (`Call::ProfileLaunch`), or why it does not: the one reading of the
/// machine's answer, which a start asks when it is planned ([`resolve`])
/// and its executor asks again just before it runs ([`exec_start`]).
fn profile_start(
    node: Option<&dyn NodeLink>,
    kind: &str,
    id: &ProfileId,
    name: &str,
    timeout: Duration,
) -> Result<ProfileStart, Refusal> {
    let agent = label(kind);
    let refused = |code, detail: String| Refusal { code, detail };
    let node = node.ok_or_else(|| {
        refused(
            PROFILE_UNANSWERED,
            format!(
                "{agent} with the profile {name} was not started: the machine that runs its pane cannot be reached"
            ),
        )
    })?;
    let answer = crate::node_access::call_as::<ProfileLaunch>(
        node,
        hide_node_link::protocol::Call::ProfileLaunch {
            kind: kind.to_owned(),
            profile: id.clone(),
        },
        timeout,
    );
    match answer {
        Ok(ProfileLaunch::Ready(start)) => Ok(start),
        Ok(ProfileLaunch::Gone) => Err(refused(
            PROFILE_GONE,
            format!(
                "{agent}'s profile {name} is gone, so it was not started; start a new agent with another profile (hide agent spawn --kind {kind} --profile <name>)"
            ),
        )),
        Ok(ProfileLaunch::Refused { text }) => Err(refused(
            PROFILE_REFUSED,
            format!("{agent} was not started: {text}"),
        )),
        // A node that knows the call and keeps no profiles, or one on an
        // older protocol that cannot read it.
        Err(LinkError::Refused(error))
            if error.code == ErrorCode::Unsupported
                || (error.code == ErrorCode::InvalidRequest
                    && node.predates_current_protocol()) =>
        {
            crate::diagnostic!(serde_json::json!({
                "component": "launch",
                "kind": "profile.unsupported",
                "agent": kind,
                "profile": id.0,
                "reason": error.message,
            }));
            Err(refused(
                PROFILES_UNSUPPORTED,
                format!(
                    "this machine does not support profiles, so {agent} starts there only with its default profile"
                ),
            ))
        }
        Err(error) => {
            crate::diagnostic!(serde_json::json!({
                "component": "launch",
                "kind": "profile.unanswered",
                "agent": kind,
                "profile": id.0,
                "reason": error.to_string(),
            }));
            Err(refused(
                PROFILE_UNANSWERED,
                format!(
                    "{agent} with the profile {name} was not started: its machine did not answer for the profile"
                ),
            ))
        }
    }
}

/// How the start `attempt` runs, asked by its executor just before it runs
/// (`hide agent exec`, `AgentExecEvent::Profile`), with the runtime lock
/// and the owner thread released: the profile as `node`, its machine, has it
/// now, only at the version the start was checked with (D-26). A profile
/// that is gone, being removed, changed since, or that its machine does not
/// answer for now is refused with why, which the executor reports as its
/// check's refusal and runs nothing on. The machine is asked as a start
/// asks it ([`profile_start`]); the hold the start took when it was planned
/// stays until what came of the start ends it, so a removal through this
/// core is refused all along, and this ask refuses one made elsewhere.
pub(crate) fn exec_start(
    node: Option<&dyn NodeLink>,
    attempt: &ExecAttempt,
) -> hide_agent_hooks::hook_request::ExecProfile {
    use hide_agent_hooks::hook_request::{ExecProfile, ExecStart};
    let (agent, name) = (label(&attempt.kind), &attempt.name);
    let refused = |reason: &str, failure: String| {
        crate::diagnostic!(serde_json::json!({
            "component": "launch",
            "kind": "exec_start.refused",
            "attempt": attempt.id,
            "agent": attempt.kind,
            "profile": attempt.profile.0,
            "reason": reason,
        }));
        ExecProfile::Refused { failure }
    };
    let start = match profile_start(node, &attempt.kind, &attempt.profile, name, EXEC_TIMEOUT) {
        Ok(start) if start.version == attempt.version => start,
        Ok(_) => {
            return refused(
                "profile_changed",
                format!(
                    "{agent} was not started: its profile {name} changed while it was starting; start it again"
                ),
            );
        }
        Err(refusal) => return refused(refusal.code, refusal.detail),
    };
    let (program, leading) = start.program.map_or((None, Vec::new()), |program| {
        (Some(program.file), program.leading)
    });
    let ready = ExecStart {
        label: shown(&attempt.kind, &start.name),
        env: start.env,
        program,
        leading,
    };
    let size = serde_json::to_vec(&ready).map_or(usize::MAX, |bytes| bytes.len());
    if size > EXEC_START_LIMIT {
        return refused(
            "start_too_large",
            format!(
                "{agent} was not started: its profile {name} carries more variables than a start takes ({EXEC_START_LIMIT} bytes)"
            ),
        );
    }
    ExecProfile::Ready(ready)
}

/// Two accounts' stores on the session's machine hold the session a resume
/// names, so no one account can resume it.
pub(crate) const SESSION_ACCOUNT_AMBIGUOUS: &str = "session_account_ambiguous";
/// The session's machine did not say which of its accounts holds the
/// session a resume names.
pub(crate) const SESSION_ACCOUNT_UNANSWERED: &str = "session_account_unanswered";
/// A resume names its session by id alone while the session's machine keeps
/// another account for the agent besides the default.
pub(crate) const SESSION_ACCOUNT_UNPROVEN: &str = "session_account_unproven";

/// The account `node`, the machine that holds the `kind` session
/// `reference` names, registered and holds it in
/// (`Call::SessionAccount`, PRD agent-profiles B19), with the runtime lock
/// released: what a resume of a session line or record, which names no
/// account, runs in. A node the core admits on the older protocol keeps no
/// profile, so its sessions are the default account's; any other node that
/// does not answer starts nothing, never the default account in its place.
pub(crate) fn session_account(
    node: &dyn NodeLink,
    kind: &str,
    reference: &crate::sidebar::SessionAgentSessionPayload,
) -> Result<AccountKey, Refusal> {
    let refused = |code, detail: String| Refusal { code, detail };
    let agent = hide_session::Agent::from_kind(kind).ok_or_else(|| {
        refused(
            SESSION_ACCOUNT_UNANSWERED,
            format!("{} keeps no session Hide reads", label(kind)),
        )
    })?;
    let answer = crate::node_access::call_as::<AccountKey>(
        node,
        hide_node_link::protocol::Call::SessionAccount {
            request: hide_session::session_account::SessionAccountRequest {
                agent,
                reference_kind: reference.kind.clone(),
                reference_value: reference.value.clone(),
            },
        },
        LAUNCH_TIMEOUT,
    );
    match answer {
        Ok(account) => Ok(account),
        Err(LinkError::Refused(error))
            if matches!(
                error.code,
                ErrorCode::InvalidRequest | ErrorCode::Unsupported
            ) && node.predates_current_protocol() =>
        {
            Ok(AccountKey::Default)
        }
        Err(error) => {
            let reason = match &error {
                LinkError::Refused(refusal) => Some(refusal.message.as_str()),
                _ => None,
            };
            crate::diagnostic!(serde_json::json!({
                "component": "launch",
                "kind": "session_account.unanswered",
                "agent": kind,
                "reason": error.to_string(),
            }));
            Err(
                if reason == Some(hide_session::session_account::AMBIGUOUS) {
                    refused(
                        SESSION_ACCOUNT_AMBIGUOUS,
                        format!(
                            "{} was not resumed: two of its accounts on this machine hold the session",
                            label(kind)
                        ),
                    )
                } else if reason == Some(hide_session::session_account::UNPROVEN) {
                    refused(
                        SESSION_ACCOUNT_UNPROVEN,
                        format!(
                            "{} was not resumed: it keeps more than one account on this machine, and the session names no file",
                            label(kind)
                        ),
                    )
                } else {
                    refused(
                        SESSION_ACCOUNT_UNANSWERED,
                        format!(
                            "{} was not resumed: its machine did not say which account holds the session",
                            label(kind)
                        ),
                    )
                },
            )
        }
    }
}

/// What a new start's parent says of the profile the start runs with
/// (D-32): a parent that is the same agent on the same machine hands its
/// child the profile it runs with, the built-in default included.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Parent {
    /// No parent hands one down: the start has none, or it is another
    /// agent, or on another machine.
    None,
    /// The parent is the same agent, and Hide has no record of its start
    /// that describes it, so its account is not known: the start picks as
    /// one with no parent does, never assuming the built-in default.
    Unknown,
    /// The parent runs with the built-in default profile.
    Default,
    /// The parent runs with this profile; whether its machine still has it
    /// is the machine's to answer when the child starts ([`resolve`]).
    Profile(LaunchProfile),
}

impl Parent {
    /// What an agent whose start Hide recorded hands down: the profile its
    /// record keeps, or the built-in default when it keeps none.
    pub(crate) fn recorded(profile: Option<LaunchProfile>) -> Self {
        profile.map_or(Self::Default, Self::Profile)
    }
}

/// The profile a new start of `kind` runs with when no record names one
/// (D-32): the one the start chose, then its parent's ([`Parent`]), then the
/// project's default (`project`, which Project settings choose), then the
/// agent's default on the machine (`profiles`). Whether a start may take a
/// profile now is judged once (`ProfilesSnapshot::startable`), and how the
/// start came to it decides what one it may not take does: a choice is the
/// caller's to refuse ([`chosen_id`]); a parent's profile is asked of its
/// machine, which refuses one gone or being removed by name; a project's or
/// machine default that is gone or being removed falls through to the next,
/// as the start panel shows it preselected.
pub(crate) fn pick(
    kind: &str,
    chosen: Option<(ProfileId, String)>,
    parent: &Parent,
    project: Option<&ProfileId>,
    profiles: Option<&ProfilesSnapshot>,
) -> Request {
    let startable = |id: &ProfileId| {
        profiles?
            .startable(kind, &id.0)
            .map(|profile| (id.clone(), profile.name.clone()))
    };
    if let Some(chosen) = chosen {
        return Request::new(kind, Some(chosen));
    }
    match parent {
        Parent::Default => return Request::new(kind, None),
        Parent::Profile(profile) => {
            let named = listed(kind, &profile.id, profiles)
                .unwrap_or_else(|| (profile.id.clone(), profile.name.clone()));
            return Request::new(kind, Some(named));
        }
        Parent::None | Parent::Unknown => {}
    }
    // A project that names the built-in default starts it (D-31): only no
    // choice at all falls through to the agent's default.
    if project.is_some_and(ProfileId::is_default) {
        return Request::new(kind, None);
    }
    let profile = project.and_then(startable).or_else(|| {
        let agent = hide_agent_adapter::adapter(kind)?.id;
        let id = profiles?.defaults.get(agent)?;
        startable(&ProfileId(id.clone()))
    });
    Request::new(kind, profile)
}

/// The profile `name` of `kind` on a machine whose profiles are
/// `profiles`, as a start that names it by name (`hide agent spawn
/// --profile`) takes it: the built-in default by its name, which the start
/// then runs with whatever its parent's or the machine's default is; refused
/// when the machine lists no such profile of that agent, or none at all
/// (B16).
pub(crate) fn chosen(
    kind: &str,
    name: &str,
    profiles: Option<&ProfilesSnapshot>,
) -> Result<(ProfileId, String), String> {
    let agent = hide_agent_adapter::adapter(kind).map_or(kind, |adapter| adapter.id);
    let builtin = hide_agent_adapter::Profile::builtin_default(agent);
    if name.eq_ignore_ascii_case(&builtin.name) {
        return Ok((builtin.id, builtin.name));
    }
    let Some(profiles) = profiles else {
        return Err(format!(
            "{PROFILE_GONE}: this machine lists no profiles, so {} has no profile {name}",
            label(kind)
        ));
    };
    profiles
        .profiles
        .iter()
        .find(|profile| profile.kind == agent && profile.name.eq_ignore_ascii_case(name))
        .map(|profile| (ProfileId(profile.id.clone()), profile.name.clone()))
        .ok_or_else(|| {
            format!(
                "{PROFILE_GONE}: {} has no profile {name} on this machine; hide agent profile list shows its profiles",
                label(kind)
            )
        })
}

/// The profile `id` of `kind` as a start that names it by id (the start
/// panel's `▾`) takes it: the built-in default, or one a start may take now
/// (`ProfilesSnapshot::startable`); refused otherwise with why, the
/// caller's to word under [`PROFILE_GONE`] (B16).
pub(crate) fn chosen_id(
    kind: &str,
    id: &str,
    profiles: Option<&ProfilesSnapshot>,
) -> Result<(ProfileId, String), String> {
    let agent = hide_agent_adapter::adapter(kind).map_or(kind, |adapter| adapter.id);
    let builtin = hide_agent_adapter::Profile::builtin_default(agent);
    if builtin.id.0 == id {
        return Ok((builtin.id, builtin.name));
    }
    profiles
        .and_then(|profiles| profiles.startable(kind, id))
        .map(|profile| (ProfileId(profile.id.clone()), profile.name.clone()))
        .ok_or_else(|| {
            format!(
                "{} has no such profile on this machine; choose another",
                label(kind)
            )
        })
}

/// The profile `id` of `kind` as `profiles` lists it, with its name, in
/// whatever state: what a parent's profile is named by before its machine
/// is asked for it.
fn listed(
    kind: &str,
    id: &ProfileId,
    profiles: Option<&ProfilesSnapshot>,
) -> Option<(ProfileId, String)> {
    let agent = hide_agent_adapter::adapter(kind).map_or(kind, |adapter| adapter.id);
    profiles?
        .profiles
        .iter()
        .find(|profile| profile.kind == agent && profile.id == id.0)
        .map(|profile| (id.clone(), profile.name.clone()))
}

/// The agent's name as the operator reads it beside a profile's
/// (`Claude · work`), the start picker's.
fn label(kind: &str) -> &str {
    hide_agent_adapter::adapter(kind).map_or(kind, |adapter| adapter.picker_label)
}

#[cfg(test)]
mod tests;
