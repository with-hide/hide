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
//! `hide agent exec` with the `hide` beside that node, so the variables reach
//! the agent's process alone and never the pane's shell (D-26, B21).

use std::time::Duration;

use hide_agent_adapter::{AccountKey, LaunchTarget, ProfileId};
use hide_node_link::ErrorCode;
use hide_node_link::profiles::{ProfileLaunch, ProfileStart};
use serde::{Deserialize, Serialize};

use crate::model::ProfilesSnapshot;
use crate::node_access::{LinkError, NodeLink};

/// How long a start waits for its machine to answer its profile: a read of
/// one small file and a look for one program.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(10);

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

    /// The profile as the operator reads it, named after its `kind` agent
    /// (`Claude · work`): a row, `hide agent list` and the start line all
    /// say it so.
    pub(crate) fn shown(&self, kind: &str) -> String {
        shown(kind, &self.name)
    }
}

/// `name`, a profile of the `kind` agent, as the operator reads it.
fn shown(kind: &str, name: &str) -> String {
    format!("{} · {name}", label(kind))
}

/// What a start asks for before its machine is asked: the agent and the
/// profile it runs with ([`LaunchTarget`]), and the name the start knew the
/// profile by, which a refusal names; none for the built-in default.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Request {
    pub(crate) target: LaunchTarget,
    name: Option<String>,
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
        }
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
}

impl Launch {
    /// The agent, in the spelling the start was asked with.
    pub(crate) fn kind(&self) -> &str {
        &self.kind
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
    /// (`hide_agent_hooks::hook_request::AGENT_EXEC_ATTEMPT`) first, the
    /// profile's variables and program, and `argv` after `--`
    /// (`hided::agent_exec`).
    pub(crate) fn line(&self, argv: Vec<String>) -> Result<StartLine, String> {
        let Some((_, start)) = &self.profile else {
            return Ok(StartLine {
                words: argv,
                attempt: None,
            });
        };
        let attempt = hide_agent_hooks::hook_request::fresh_request_id()
            .map_err(|why| format!("the start's attempt could not be named: {why}"))?;
        let mut line = vec![
            start.exec.clone(),
            "agent".to_owned(),
            "exec".to_owned(),
            hide_agent_hooks::hook_request::AGENT_EXEC_ATTEMPT.to_owned(),
            attempt.clone(),
            "--profile".to_owned(),
            shown(&self.kind, &start.name),
        ];
        for (name, value) in &start.env {
            line.push("--env".to_owned());
            line.push(format!("{name}={value}"));
        }
        if let Some(program) = &start.program {
            line.push("--program".to_owned());
            line.push(program.file.clone());
            for word in &program.leading {
                line.push("--leading".to_owned());
                line.push(word.clone());
            }
        }
        line.push("--".to_owned());
        line.extend(argv);
        Ok(StartLine {
            words: line,
            attempt: Some(attempt),
        })
    }

    /// A start of `kind` with the built-in default profile, as a test's
    /// engine takes it.
    #[cfg(test)]
    pub(crate) fn default_of(kind: &str) -> Self {
        Self {
            kind: kind.to_owned(),
            profile: None,
        }
    }

    /// A start of `kind` with the profile `id` as its machine answered it,
    /// for a test that types its line.
    #[cfg(test)]
    pub(crate) fn answered(kind: &str, id: &str, start: ProfileStart) -> Self {
        Self {
            kind: kind.to_owned(),
            profile: Some((ProfileId(id.to_owned()), start)),
        }
    }
}

/// A start's line as the pane's shell runs it, and the attempt a profile's
/// start runs under (`engine::own::attempt`); none for the built-in
/// default, which waits on nothing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StartLine {
    pub(crate) words: Vec<String>,
    pub(crate) attempt: Option<String>,
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
        });
    };
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
        LAUNCH_TIMEOUT,
    );
    let start = match answer {
        Ok(ProfileLaunch::Ready(start)) => start,
        Ok(ProfileLaunch::Gone) => {
            return Err(refused(
                PROFILE_GONE,
                format!(
                    "{agent}'s profile {name} is gone, so it was not started; start a new agent with another profile (hide agent spawn --kind {kind} --profile <name>)"
                ),
            ));
        }
        Ok(ProfileLaunch::Refused { text }) => {
            return Err(refused(
                PROFILE_REFUSED,
                format!("{agent} was not started: {text}"),
            ));
        }
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
            return Err(refused(
                PROFILES_UNSUPPORTED,
                format!(
                    "this machine does not support profiles, so {agent} starts there only with its default profile"
                ),
            ));
        }
        Err(error) => {
            crate::diagnostic!(serde_json::json!({
                "component": "launch",
                "kind": "profile.unanswered",
                "agent": kind,
                "profile": id.0,
                "reason": error.to_string(),
            }));
            return Err(refused(
                PROFILE_UNANSWERED,
                format!(
                    "{agent} with the profile {name} was not started: its machine did not answer for the profile"
                ),
            ));
        }
    };
    Ok(Launch {
        kind: kind.to_owned(),
        profile: Some((id, start)),
    })
}

/// What a new start's parent says of the profile the start runs with
/// (D-32): a parent that is the same agent on the same machine hands its
/// child the profile it runs with, the built-in default included.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Parent {
    /// No parent hands one down: the start has none, or it is another
    /// agent, on another machine, or one whose start Hide has no record of.
    None,
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
/// agent's default on the machine (`profiles`). A choice the machine does not
/// list, or lists as being removed, is the caller's to refuse ([`chosen`]);
/// a parent's profile is asked of its machine, which refuses a removed one
/// by name; a project's or machine default that is gone falls through to
/// the next.
pub(crate) fn pick(
    kind: &str,
    chosen: Option<(ProfileId, String)>,
    parent: &Parent,
    project: Option<&ProfileId>,
    profiles: Option<&ProfilesSnapshot>,
) -> Request {
    let listed = |id: &ProfileId| listed(kind, id, profiles);
    if let Some(chosen) = chosen {
        return Request::new(kind, Some(chosen));
    }
    match parent {
        Parent::Default => return Request::new(kind, None),
        Parent::Profile(profile) => {
            let named =
                listed(&profile.id).unwrap_or_else(|| (profile.id.clone(), profile.name.clone()));
            return Request::new(kind, Some(named));
        }
        Parent::None => {}
    }
    let profile = project.and_then(listed).or_else(|| {
        let agent = hide_agent_adapter::adapter(kind)?.id;
        let id = profiles?.defaults.get(agent)?;
        listed(&ProfileId(id.clone()))
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

/// The profile `id` of `kind` as `profiles` lists it, with its name.
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
