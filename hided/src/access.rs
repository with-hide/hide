//! The daemon's side of external tools (PRD hide-external-control): the
//! bootstrap socket's answer to a tool's ask, what each refusal tells the
//! tool to do, and which commands each mode runs.
//!
//! A tool proves itself with the key it keeps in its own home; this daemon
//! hashes the key, hands the core only the hash, and forgets it. Nothing
//! here writes a key, a key's hash or a key file's path to a log (B33).

use std::path::PathBuf;

use hide_agent_hooks::hook_request::install::{AccessAsk, PaneRequest};
use hide_core::access::{
    Call, NEEDS_AGENT, NOT_OWN_WORK, OPERATOR_ONLY, SPAWN_PARENT, key_hash, valid_key, valid_name,
};
use hide_core::engine::EngineHandle;
use serde_json::{Value, json};

use crate::core::CoreHandle;
use crate::pane_auth::Registry;

mod origin;

/// What a command refused because the connection is gone says to do.
pub const REVOKED_NEXT_ACTION: &str =
    "The operator disconnected this tool; run hide access request --name <name> to ask again";

/// Every refusal a tool's command can get, each with its `next_action`
/// (`hide contract --json` lists them, B36).
pub const REFUSALS: &[&str] = &[
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
    "access_reapprove",
    NOT_OWN_WORK,
    OPERATOR_ONLY,
    NEEDS_AGENT,
    SPAWN_PARENT,
    "access_core_only",
    "access_unavailable",
    origin::UNAVAILABLE,
];

/// What each tool refusal says to do, in the words the CLI, `hide guide`
/// and the contract use (B36); `None` for a reason that is not a tool's.
pub fn next_action(reason: &str) -> Option<&'static str> {
    Some(match hide_core::coordination::split_refusal(reason).0 {
        "access_from_pane" => {
            "An agent in a Hide pane acts as itself; run hide access request from the tool, outside Hide's panes"
        }
        "access_pending" => {
            "Wait until the operator approves the request in Settings > Bots; the next hide command then runs as the tool"
        }
        "access_refused" => {
            "The operator refused this tool; run hide access request --name <name> to ask again"
        }
        "access_request_expired" => {
            "The request was dropped; run hide access request --name <name> to ask again"
        }
        "access_limit" => {
            "16 tools are connected; ask the operator to disconnect one in Settings > Bots, then run hide access request again"
        }
        "access_name_taken" => {
            "A connected tool has this name; ask the operator to disconnect it in Settings > Bots, or choose another --name, then run hide access request again"
        }
        "access_revoked" => REVOKED_NEXT_ACTION,
        "access_ambiguous" => {
            "This home keeps several tools' keys; set HIDE_ACCESS=<name> to choose one"
        }
        "access_not_connected" => "Run hide access request --name <name> first",
        "access_key_unreadable" => {
            "This tool's key file cannot be read; run hide access request --name <name>, which replaces it and asks the operator again"
        }
        "access_reapprove" => {
            "External tools became bots with two modes, and a tool that was View only is approved again before it runs anything; run hide access request --name <name> to ask the operator"
        }
        "access_not_own_work" => hide_core::access::NOT_OWN_WORK_NEXT_ACTION,
        "access_operator_only" => {
            "Only the operator adds or removes a device, in Settings > Devices; ask the operator"
        }
        "access_needs_agent" => {
            "Only an agent in a Hide pane runs this; start one with hide agent spawn and ask it with hide request send"
        }
        "access_spawn_parent" => {
            "A tool is no agent's parent; run hide agent spawn without --parent and follow the agent with hide agent show and hide inbox"
        }
        "access_core_only" => "Run the tool on the machine where Hide's core runs",
        "access_unavailable" => "Start Hide on this machine, then run the same command again",
        origin::UNAVAILABLE => "Reconnect Hide, then retry from a terminal outside Hide's panes",
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
        "access_reapprove" => "access_reapprove",
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
        | AccessAsk::Use { key, .. } => key,
    };
    if !valid_key(key) {
        return (refused("invalid_request"), None);
    }
    // An agent in a pane is never a tool, whatever its environment says
    // (D-08, B8): a peer that descends from a pane's program, of this
    // core's engine, is refused before any key is looked at.
    let in_pane = match in_a_pane(peer, request, engine, core) {
        Ok(in_pane) => in_pane,
        Err(reason) => return (refused(reason), None),
    };
    if in_pane {
        note("access.from_pane", None, "access_from_pane");
        return (refused("access_from_pane"), None);
    }
    let key_sha256 = key_hash(key);
    let call = match ask {
        AccessAsk::Request { name, .. } => {
            if !valid_name(name) {
                return (refused("invalid_request"), None);
            }
            let Some(id) = fresh_id() else {
                return (refused("access_unavailable"), None);
            };
            Call::Request {
                name: name.clone(),
                key_sha256,
                program: program_of(peer),
                id,
            }
        }
        AccessAsk::Status { .. } => Call::Status { key_sha256 },
        AccessAsk::Leave { .. } => Call::Leave { key_sha256 },
        AccessAsk::Use { folder, .. } => {
            let folder = folder.as_deref();
            return match use_credential(peer, request, key_sha256, folder, registry, core, port) {
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
    folder: Option<&str>,
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
    let attestation = crate::pane_auth::attest_access(peer, key_sha256, folder, core)?;
    crate::pane_auth::issue_access(registry, &attestation, &request.nonce, port, holder)
        .map_err(str::to_owned)
}

/// Who asks a pairing question (`crate::pairing::Caller`): a pane's
/// program, a bot whose key the request carries, in the mode the core holds
/// for it now (mission-87), or else the operator at a terminal. A key from a
/// pane, or one the core does not hold, is refused as a bot's command is.
pub(crate) fn pairing_caller(
    peer: i32,
    request: &PaneRequest,
    engine: Option<&EngineHandle>,
    core: &CoreHandle,
) -> Result<crate::pairing::Caller, Value> {
    let in_pane = in_a_pane(peer, request, engine, core).map_err(refused)?;
    let key = match &request.access {
        None if in_pane => return Ok(crate::pairing::Caller::Pane),
        None => return Ok(crate::pairing::Caller::Operator),
        Some(AccessAsk::Use { key, folder: None }) if valid_key(key) => key,
        Some(_) => return Err(refused("invalid_request")),
    };
    if in_pane {
        note("access.from_pane", None, "access_from_pane");
        return Err(refused("access_from_pane"));
    }
    let proved = core
        .access(Call::Prove {
            key_sha256: key_hash(key),
        })
        .map_err(|reason| refused(&reason))?;
    let mode = serde_json::from_value(proved["mode"].clone())
        .map_err(|_| refused("access_unavailable"))?;
    Ok(crate::pairing::Caller::Bot(mode))
}

/// Whether `peer` runs in a Hide pane of this machine. Kernel ancestry
/// covers every holder owned by this home or placed for this core's
/// guests; a hinted pane or the host's own retained program records also
/// prove it. No unreadable proof becomes an outside-terminal answer.
pub(crate) fn in_a_pane(
    peer: i32,
    request: &PaneRequest,
    engine: Option<&EngineHandle>,
    core: &CoreHandle,
) -> Result<bool, &'static str> {
    if origin::in_holder(peer, core.home(), core.state_dir())? {
        return Ok(true);
    }
    if !request.pane_id.is_empty()
        && let Some(engine) = engine
        && crate::pane_auth::attest_local(peer, &request.pane_id, engine, core).is_ok()
    {
        return Ok(true);
    }
    let entries = match std::fs::read_dir(hide_kit::layout::pane_records(core.state_dir())) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(origin::UNAVAILABLE),
    };
    let Ok(peer) = u32::try_from(peer) else {
        return Err(origin::UNAVAILABLE);
    };
    let mut count = 0;
    for entry in entries {
        let entry = entry.map_err(|_| origin::UNAVAILABLE)?;
        if entry
            .path()
            .extension()
            .is_none_or(|extension| extension != "json")
        {
            continue;
        }
        count += 1;
        // The holder-record owner already caps this retained set. A
        // truncated authorization proof cannot establish "outside", and
        // this reader must not delete another owner's live evidence.
        if count > hide_kit::panes::MAX_RECORD_FILES {
            return Err(origin::UNAVAILABLE);
        }
        let record =
            hide_holder_protocol::record::read(&entry.path()).map_err(|_| origin::UNAVAILABLE)?;
        if let Some(program) = record.and_then(|record| record.program) {
            match hide_platform::process::start_time(program.pid) {
                Ok(started)
                    if started == program.started
                        && hide_platform::process::descends_from(peer, program.pid) =>
                {
                    return Ok(true);
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(origin::UNAVAILABLE),
            }
        }
    }
    Ok(false)
}

/// A request's id, random so that nothing of the key is in it
/// (`hide_core::access::valid_id`).
fn fresh_id() -> Option<String> {
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes).ok()?;
    Some(hex::encode(bytes))
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
    fn every_tool_refusal_says_what_to_do() {
        for reason in REFUSALS {
            assert!(next_action(reason).is_some(), "{reason}");
        }
        assert_eq!(next_action("pane_changed"), None);
    }
}
