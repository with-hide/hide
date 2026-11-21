//! The pane bootstrap's words for process identity (an `i32` pid, a start as
//! an optional number) over `hide-platform`, which answers each question for
//! the system it runs on. hided and a device's node (`panes`) ask the
//! same questions through here.

use std::path::PathBuf;

use hide_platform::process;

pub use hide_node_link::panes::PaneIdentity;

/// Whether `pid` is `shell_pid` or one of its descendants.
pub fn descends_from(pid: i32, shell_pid: i32) -> bool {
    match (u32::try_from(pid), u32::try_from(shell_pid)) {
        (Ok(pid), Ok(shell)) => process::descends_from(pid, shell),
        _ => false,
    }
}

/// When `pid` started, comparable only with another start of the same
/// system; `None` once the process is gone.
pub fn process_start(pid: i32) -> Option<u64> {
    process::start_time(u32::try_from(pid).ok()?).ok()
}

/// The peer's current directory as the system reports it, or `None` when the
/// process is gone or refuses inspection.
pub fn process_cwd(pid: i32) -> Option<PathBuf> {
    process::cwd_of(u32::try_from(pid).ok()?).ok()
}

/// The processes from `pid` up to the pane's shell `shell_pid`, both
/// included, each kept apart from a later process given its pid: the
/// lineage a hook's report is judged by, so a session an agent's own hook
/// names is told from one an agent it ran names (`Engine::hook_session`).
/// `None` when the system cannot show it.
pub fn lineage(
    pid: i32,
    shell_pid: i32,
) -> Option<Vec<hide_agent_hooks::hook_request::ExecProcess>> {
    let lineage = process::lineage(u32::try_from(pid).ok()?, u32::try_from(shell_pid).ok()?)?;
    Some(
        lineage
            .into_iter()
            .map(|(pid, started)| hide_agent_hooks::hook_request::ExecProcess { pid, started })
            .collect(),
    )
}
