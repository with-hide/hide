//! The metadata Hide writes on the engine's panes and workspaces and reads
//! back: who spawned a pane, which sessions a relationship was written for,
//! a fork's source, the server a saved read state belongs to, and issues.
//! These are Hide's conventions, not the engine's protocol, so every engine
//! carries the same tokens.

use serde_json::Value;
use sha2::{Digest, Sha256};

/// The pane metadata token by which whoever created a pane declares the pane
/// it was spawned from. It is the only lineage there is: Herdr records none,
/// so an agent is a child exactly when its spawner said so.
///
/// hcoord is the sole writer, including for a child created by Hide's Fork.
/// It declares the token after both executions exist with
/// `pane.report_metadata`, the same display-only channel the hook helper
/// already uses. The value is the parent's pane id, it dies
/// with the pane, and Hide reads it here and nowhere else.
///
/// The token outlives the agent that earned it, because it dies with the
/// pane and a pane can host another agent later, so it is only a claim until
/// `CHILD_SESSION_TOKEN` and `PARENT_SESSION_TOKEN` prove that both panes
/// still host the sessions it was written for.
pub(crate) const PARENT_PANE_TOKEN: &str = "parent_pane";

/// The digest of the child's agent session when the relationship was written.
/// A relationship holds only while the child's pane still reports that session.
pub(crate) const CHILD_SESSION_TOKEN: &str = "child_session";

/// The digest of the parent's agent session when the relationship was written.
/// `crate::agent_state::apply_lineage` compares it with the parent pane's current session,
/// because only there are both rows at hand, on this machine or another.
pub(crate) const PARENT_SESSION_TOKEN: &str = "parent_session";

/// The pane a fork was forked from, written by `crate::coordination` on the
/// pane that hosts a fork's registration (`AgentRecord::forked` with its
/// `origin`), so a wake that continues the registration in another pane writes
/// it there too. It is provenance, never responsibility: a fork is the
/// operator's root, which is why it is not `PARENT_PANE_TOKEN`. Like that
/// token it is a claim that holds only while `FORK_SESSION_TOKEN` and
/// `FORK_SOURCE_SESSION_TOKEN` still name the sessions the two panes host.
pub(crate) const FORK_PANE_TOKEN: &str = "fork_pane";

/// The digest of the fork's own session when the provenance was written. A
/// pane that hosts another session later is not a fork.
pub(crate) const FORK_SESSION_TOKEN: &str = "fork_session";

/// The digest of the source's session when the provenance was written. The
/// source is named only while its pane still hosts it
/// (`crate::agent_state::apply_fork_sources`).
pub(crate) const FORK_SOURCE_SESSION_TOKEN: &str = "fork_source_session";

/// Hide's mark on a pane whose saved read state it vouches for, written by
/// the lineage writer (`crate::coordination::lineage`). Herdr keeps a pane
/// token only for as long as the server that took it, so a pane that still
/// carries the mark a saved state names is on the server that state was read
/// from, and its `state_change_seq` can be compared with the saved one
/// (`crate::agent_state::apply_read_state`).
pub(crate) const SERVER_MARK_TOKEN: &str = "hide_server";

/// The server mark a pane carries, when it carries one.
pub(crate) fn server_mark(tokens: &std::collections::BTreeMap<String, Value>) -> Option<String> {
    tokens
        .get(SERVER_MARK_TOKEN)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|mark| !mark.is_empty())
        .map(str::to_owned)
}

/// The tokens a sleeping fork's row stands in for, as its pane carried them.
pub(crate) fn fork_tokens(
    source_pane: &str,
    own_session: Option<&str>,
    source_session: Option<&str>,
) -> std::collections::BTreeMap<String, Value> {
    [
        (FORK_PANE_TOKEN, Some(source_pane)),
        (FORK_SESSION_TOKEN, own_session),
        (FORK_SOURCE_SESSION_TOKEN, source_session),
    ]
    .into_iter()
    .filter_map(|(key, value)| Some((key.to_owned(), Value::String(value?.to_owned()))))
    .collect()
}

/// What a fork's pane declares about its source: the source's pane and the
/// session it hosted, read only while the pane still hosts the fork's own
/// session (`own_session`, its digest).
pub(crate) fn fork_declaration(
    tokens: &std::collections::BTreeMap<String, Value>,
    own_session: Option<&str>,
) -> Option<(String, Option<String>)> {
    let token = |name: &str| lineage_parent(tokens.get(name).and_then(Value::as_str));
    if !session_holds(token(FORK_SESSION_TOKEN).as_deref(), own_session) {
        return None;
    }
    Some((token(FORK_PANE_TOKEN)?, token(FORK_SOURCE_SESSION_TOKEN)))
}

/// How a session is written into a token: the lowercase hex SHA-256 of the
/// value Herdr reports in `agent_session.value`. Herdr cuts a token value at
/// 80 characters and a session can be a path, so a digest is the one form
/// that keeps two sessions apart whatever they look like; hcoord writes the
/// same function. A blank value names no session.
pub(crate) fn session_digest(value: &str) -> Option<String> {
    if value.trim().is_empty() {
        return None;
    }
    Some(
        Sha256::digest(value.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

/// Whether a pane still hosts the session a relationship was written for.
/// A missing side proves nothing, so it does not hold: a pane Herdr reports
/// without a session (Codex before its first turn) cannot confirm a match, and
/// a token written before sessions were recorded has nothing to compare.
pub(crate) fn session_holds(declared: Option<&str>, current: Option<&str>) -> bool {
    matches!((declared, current), (Some(declared), Some(current)) if declared == current)
}

/// Stable machine identity paired with `parent_pane` for cross-device
/// lineage. Same-machine children omit it so an ordinary local parent keeps
/// the existing device-scoped path.
pub(crate) const PARENT_MACHINE_TOKEN: &str = "parent_machine";

/// The source under which Hide writes its own pane tokens. Herdr keeps one
/// token set per source, so a token Hide wrote can never overwrite one an
/// orchestrator or the hook helper wrote under theirs.
pub(crate) const HIDE_METADATA_SOURCE: &str = "hide";

/// The parent a spawner declared. An empty value is no declaration, because
/// Herdr clears a token by setting it empty.
pub(crate) fn lineage_parent(declared: Option<&str>) -> Option<String> {
    declared
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IssueTokens {
    pub panes: std::collections::BTreeMap<String, String>,
    pub workspaces: std::collections::BTreeMap<String, String>,
}

pub(crate) fn issue_tokens(payload: &crate::sidebar::SessionSnapshotPayload) -> IssueTokens {
    fn issue(tokens: &std::collections::BTreeMap<String, Value>) -> Option<String> {
        tokens
            .get("issue")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    }
    let mut panes: std::collections::BTreeMap<_, _> = payload
        .agents
        .iter()
        .filter_map(|agent| {
            Some((
                agent.pane_id.as_ref().or(agent.id.as_ref())?.clone(),
                issue(&agent.tokens)?,
            ))
        })
        .collect();
    for pane in &payload.panes {
        if let Some(value) = issue(&pane.tokens) {
            panes.insert(pane.pane_id.clone(), value);
        }
    }
    IssueTokens {
        panes,
        workspaces: payload
            .workspaces
            .iter()
            .filter_map(|workspace| {
                Some((workspace.workspace_id.clone(), issue(&workspace.tokens)?))
            })
            .collect(),
    }
}
