//! The daemon's side of external tools (PRD hide-external-control): the
//! bootstrap socket's answer to a tool's ask, what each refusal tells the
//! tool to do, and which commands each level runs.
//!
//! A tool proves itself with the key it keeps in its own home; this daemon
//! hashes the key, hands the core only the hash, and forgets it. Nothing
//! here writes a key, a key's hash or a key file's path to a log (B33).

use std::path::PathBuf;

use hide_agent_hooks::hook_request::install::{AccessAsk, PaneRequest};
use hide_core::access::{Call, Level, key_hash, valid_key, valid_name};
use hide_core::engine::EngineHandle;
use serde_json::{Value, json};

use crate::core::CoreHandle;
use crate::pane_auth::Registry;

/// What a command refused because the connection is gone says to do.
pub const REVOKED_NEXT_ACTION: &str =
    "The operator disconnected this tool; run hide access request --name <name> to ask again";

/// The level a command needs from a tool (B20-B23), judged at the command
/// boundary (`server::ScopedRequest::access_need`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Need {
    /// Reads what Hide shows: View only runs it.
    View,
    /// Changes something, as the operator would: Act runs it.
    Act,
    /// Means an agent in a Hide pane: no tool runs it; the tool starts an
    /// agent and asks it instead (B22).
    Agent,
    /// A spawn under a parent: a tool is never one (B23).
    Parent,
}

/// Whether a tool at `level` runs a command that needs `need`, or the
/// reason it is refused.
pub fn admit(level: Level, need: Need) -> Result<(), &'static str> {
    match (level, need) {
        (_, Need::View) | (Level::Act, Need::Act) => Ok(()),
        (Level::View, _) => Err(VIEW_ONLY),
        (Level::Act, Need::Agent) => Err(NEEDS_AGENT),
        (Level::Act, Need::Parent) => Err(SPAWN_PARENT),
    }
}

const VIEW_ONLY: &str = "access_view_only";
const NEEDS_AGENT: &str = "access_needs_agent";
const SPAWN_PARENT: &str = "access_spawn_parent";

/// What each tool refusal says to do, in the words the CLI, `hide guide`
/// and the contract use (B36); `None` for a reason that is not a tool's.
pub fn next_action(reason: &str) -> Option<&'static str> {
    Some(match hide_core::coordination::split_refusal(reason).0 {
        "access_from_pane" => {
            "An agent in a Hide pane acts as itself; run hide access request from the tool, outside Hide's panes"
        }
        "access_pending" => {
            "Wait until the operator approves the request in Settings > External tools, then run hide access request again"
        }
        "access_refused" => {
            "The operator refused this tool; run hide access request --name <name> to ask again"
        }
        "access_request_expired" => {
            "The request was dropped; run hide access request --name <name> to ask again"
        }
        "access_limit" => {
            "16 tools are connected; ask the operator to disconnect one in Settings > External tools, then run hide access request again"
        }
        "access_name_taken" => {
            "A connected tool has this name; choose another --name, or ask the operator to disconnect that tool"
        }
        "access_revoked" => REVOKED_NEXT_ACTION,
        "access_ambiguous" => {
            "This home keeps several tools' keys; set HIDE_ACCESS=<name> to choose one"
        }
        "access_not_connected" => "Run hide access request --name <name> first",
        "access_key_unreadable" => {
            "This tool's key file cannot be read; run hide access request --name <name> to make a new one"
        }
        "access_view_only" => {
            "This tool is View only; ask the operator to switch it to Act in Settings > External tools"
        }
        "access_needs_agent" => {
            "Only an agent in a Hide pane runs this; start one with hide agent spawn and ask it with hide request send"
        }
        "access_spawn_parent" => {
            "A tool is no agent's parent; run hide agent spawn without --parent and follow the agent with hide agent show and hide inbox"
        }
        "access_core_only" => "Run the tool on the machine where Hide's core runs",
        "access_unavailable" => "Start Hide on this machine, then run the same command again",
        _ => return None,
    })
}

/// The `&'static str` a credential refusal carries for a core reason.
pub fn static_reason(reason: &str) -> &'static str {
    match reason {
        "access_revoked" => "access_revoked",
        "access_pending" => "access_pending",
        "access_refused" => "access_refused",
        "access_request_expired" => "access_request_expired",
        _ => "access_unavailable",
    }
}

/// One diagnostic record of a tool's credential: the connection and the
/// reason, never the key or the folder the tool ran in.
pub fn note(kind: &str, connection: Option<&str>, reason: &str) {
    hide_core::diagnostic!(json!({
        "component": "access",
        "kind": kind,
        "connection_id": connection,
        "reason": reason,
    }));
}

fn refused(reason: &str) -> Value {
    json!({
        "ok": false,
        "reason": reason,
        "next_action": next_action(reason).unwrap_or_else(|| crate::pane_auth::refusal_next_action(reason)),
    })
}

/// Answers a tool's ask on the bootstrap socket. Returns the answer and,
/// for a command's credential, the reference it issued, which the caller
/// revokes when the answer cannot be written.
pub(crate) fn answer_bootstrap(
    peer: i32,
    request: &PaneRequest,
    ask: &AccessAsk,
    registry: &Registry,
    core: &CoreHandle,
    engine: Option<&EngineHandle>,
    port: u16,
) -> (Value, Option<PathBuf>) {
    let key = match ask {
        AccessAsk::Request { key, .. }
        | AccessAsk::Status { key }
        | AccessAsk::Leave { key }
        | AccessAsk::Use { key } => key,
    };
    if !valid_key(key) {
        return (refused("invalid_request"), None);
    }
    // An agent in a pane is never a tool, whatever its environment says
    // (D-08, B8): a peer that descends from a pane's program, of this
    // core's engine, is refused before any key is looked at.
    if in_a_pane(peer, request, engine, core) {
        note("access.from_pane", None, "access_from_pane");
        return (refused("access_from_pane"), None);
    }
    let key_sha256 = key_hash(key);
    let call = match ask {
        AccessAsk::Request { name, .. } => {
            if !valid_name(name) {
                return (refused("invalid_request"), None);
            }
            Call::Request {
                name: name.clone(),
                key_sha256,
                program: program_of(peer),
            }
        }
        AccessAsk::Status { .. } => Call::Status { key_sha256 },
        AccessAsk::Leave { .. } => Call::Leave { key_sha256 },
        AccessAsk::Use { .. } => {
            return match use_credential(peer, request, key_sha256, registry, core, port) {
                Ok(path) => match hide_platform::path::to_wire(&path) {
                    Ok(reference) => (json!({"ok": true, "reference": reference}), Some(path)),
                    Err(_) => (refused("reference_unavailable"), Some(path)),
                },
                Err(reason) => (refused(&reason), None),
            };
        }
    };
    match core.access(call) {
        Ok(access) => (json!({"ok": true, "access": access}), None),
        Err(reason) => (refused(&reason), None),
    }
}

fn use_credential(
    peer: i32,
    request: &PaneRequest,
    key_sha256: String,
    registry: &Registry,
    core: &CoreHandle,
    port: u16,
) -> Result<PathBuf, String> {
    if !request.one_shot {
        return Err("invalid_request".to_owned());
    }
    let holder = (
        peer,
        hide_node::pane_proof::process_start(peer).ok_or("caller_unavailable")?,
    );
    let attestation = crate::pane_auth::attest_access(peer, key_sha256, core)?;
    crate::pane_auth::issue_access(registry, &attestation, &request.nonce, port, holder)
        .map_err(str::to_owned)
}

/// Whether `peer` runs in a Hide pane of this machine: the pane its request
/// names proves it, or it descends from the program of a pane the core's
/// engine keeps a record of, which a pane agent that cleared its
/// environment still does.
fn in_a_pane(
    peer: i32,
    request: &PaneRequest,
    engine: Option<&EngineHandle>,
    core: &CoreHandle,
) -> bool {
    if !request.pane_id.is_empty()
        && let Some(engine) = engine
        && crate::pane_auth::attest_local(peer, &request.pane_id, engine, core).is_ok()
    {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(hide_kit::layout::pane_records(core.state_dir())) else {
        return false;
    };
    let Ok(peer) = u32::try_from(peer) else {
        return false;
    };
    entries
        .flatten()
        .filter_map(|entry| {
            hide_holder_protocol::record::read(&entry.path())
                .ok()
                .flatten()
        })
        .filter_map(|record| record.program)
        .any(|program| program.runs() && hide_platform::process::descends_from(peer, program.pid))
}

/// The program that ran `hide access request`, as the kernel names the
/// requesting process's parent: its file where the system says, else its
/// name. Shown to the operator beside the request, never trusted.
fn program_of(peer: i32) -> Option<String> {
    let parent = hide_platform::process::parent_of(u32::try_from(peer).ok()?).ok()?;
    match hide_platform::process::image_of(parent) {
        Ok(image) => Some(image.path.to_string_lossy().into_owned()),
        Err(_) => hide_platform::process::name_of(parent).ok(),
    }
    .filter(|program| !program.is_empty())
    .map(|program| program.chars().take(1024).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_only_runs_reads_and_act_runs_all_but_what_needs_an_agent() {
        assert_eq!(admit(Level::View, Need::View), Ok(()));
        assert_eq!(admit(Level::View, Need::Act), Err(VIEW_ONLY));
        assert_eq!(admit(Level::View, Need::Agent), Err(VIEW_ONLY));
        assert_eq!(admit(Level::View, Need::Parent), Err(VIEW_ONLY));
        assert_eq!(admit(Level::Act, Need::Act), Ok(()));
        assert_eq!(admit(Level::Act, Need::Agent), Err(NEEDS_AGENT));
        assert_eq!(admit(Level::Act, Need::Parent), Err(SPAWN_PARENT));
    }

    #[test]
    fn every_tool_refusal_says_what_to_do() {
        for reason in [
            "access_from_pane",
            "access_pending",
            "access_refused",
            "access_request_expired",
            "access_limit",
            "access_name_taken",
            "access_revoked",
            "access_ambiguous",
            "access_not_connected",
            "access_key_unreadable",
            VIEW_ONLY,
            NEEDS_AGENT,
            SPAWN_PARENT,
            "access_core_only",
            "access_unavailable",
        ] {
            assert!(next_action(reason).is_some(), "{reason}");
        }
        assert_eq!(next_action("pane_changed"), None);
    }
}
