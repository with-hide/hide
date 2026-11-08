//! The account a node reads an agent's sessions in: the agent's built-in
//! default profile (`hide_agent_adapter::resolve`), whose `sessions` place
//! replaces every fixed session folder. The node follows only the variable
//! it captured and checked at startup (`crate::environment`: OpenCode's
//! `XDG_DATA_HOME`); every other agent's sessions are read at their default
//! place below the home.

use std::path::{Path, PathBuf};

use hide_agent_adapter::{AccountPath, AgentId, NodeEnv, Profile, resolve};

use crate::Agent;

/// The adapter of a supported agent; retired Cursor keeps no account.
fn adapter(agent: Agent) -> Option<&'static hide_agent_adapter::AgentAdapter> {
    let id = match agent {
        Agent::Claude => AgentId::ClaudeCode,
        Agent::Codex => AgentId::Codex,
        Agent::Grok => AgentId::Grok,
        Agent::Pi => AgentId::Pi,
        Agent::Omp => AgentId::Omp,
        Agent::OpenCode => AgentId::OpenCode,
        Agent::Antigravity => AgentId::Antigravity,
        Agent::Cursor => return None,
    };
    Some(id.adapter())
}

/// Where `agent` keeps its sessions for the account whose home is `home`:
/// a folder, or OpenCode's database. `Ok(None)` for Cursor; an error with
/// the reason when the node's own `XDG_DATA_HOME` is no usable path.
pub(crate) fn sessions(home: &Path, agent: Agent) -> Result<Option<PathBuf>, &'static str> {
    let Some(adapter) = adapter(agent) else {
        return Ok(None);
    };
    let mut node = NodeEnv::new(Some(home.to_path_buf()));
    if agent == Agent::OpenCode {
        node = node.with_variable(
            "XDG_DATA_HOME",
            crate::environment::data_home()?.map(|path| path.as_os_str().to_os_string()),
        );
    }
    let account = resolve(adapter.definition(), &Profile::builtin_default(adapter.id), &node)
        .expect("the node's checked variables and a known home leave nothing to refuse");
    Ok(account.path(AccountPath::Sessions).map(Path::to_path_buf))
}

/// [`sessions`] for an agent whose sessions are a folder below the home,
/// as that folder relative to `home`.
pub(crate) fn sessions_below_home(home: &Path, agent: Agent) -> PathBuf {
    sessions(home, agent)
        .ok()
        .flatten()
        .and_then(|path| path.strip_prefix(home).ok().map(Path::to_path_buf))
        .expect("a file-per-session agent keeps its sessions below the home")
}
