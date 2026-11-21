//! Claude Code's one exception to how a hook reads the account it runs in
//! (`account::reported_variables`): with
//! `CLAUDE_CODE_SUBPROCESS_ENV_SCRUB=1` Claude Code drops its variables from
//! a hook's environment, so the account's folder is read from the session's
//! `CLAUDE_ENV_FILE`, which Claude Code keeps below that folder.

use std::path::Path;

use hide_agent_adapter::{AgentAdapter, AgentId};

/// The account folder a scrubbed Claude Code hook runs in, as `read` reads
/// the hook's variables; `None` for another agent or an unscrubbed hook.
pub(crate) fn scrubbed_folder(
    adapter: &AgentAdapter,
    read: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    if adapter.id != AgentId::ClaudeCode.adapter().id
        || read("CLAUDE_CODE_SUBPROCESS_ENV_SCRUB").as_deref() != Some("1")
    {
        return None;
    }
    env_file_folder(&read("CLAUDE_ENV_FILE")?)
}

/// The account folder above a `CLAUDE_ENV_FILE`, which Claude Code writes at
/// `<folder>/session-env/<session>/…`: the folder above its `session-env`.
fn env_file_folder(file: &str) -> Option<String> {
    let path = Path::new(file);
    path.ancestors()
        .find(|folder| folder.file_name().is_some_and(|name| name == "session-env"))
        .and_then(Path::parent)
        .filter(|folder| folder.is_absolute())
        .and_then(|folder| folder.to_str())
        .map(str::to_owned)
}
