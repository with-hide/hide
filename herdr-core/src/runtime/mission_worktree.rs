//! The build cache of a mission node's finished worktree, deleted when
//! `끝난 작업의 빌드 캐시 자동 삭제` is on (PRD mission-node-simple D-14,
//! B20). Run off the lock on a thread of the runtime's
//! (`Runtime::start_cache_clear`). The mission never removes a worktree; a
//! merged one is the project's disk cleanup's.

use super::memory_promotion::{agents_listed, panes_in, worktree_path};
use std::path::Path;
use std::time::Duration;

/// How long one of Herdr's reads here may take.
const READ_WAIT: crate::engine::Wait = crate::engine::Wait::Each(Duration::from_secs(5));

/// What deleting a finished node's build cache came to (PRD
/// mission-node-simple D-14, B20).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Cleared {
    /// The branch has no worktree.
    Gone,
    /// The branch is checked out in the Project's own folder, which is the
    /// operator's and is not touched.
    ProjectFolder,
    /// An agent or a program works in the worktree at `path` now, read
    /// before the measurement or right before the deletion: nothing moved,
    /// and it is held until what works there changes.
    Busy { path: String },
    /// What the deletion came to.
    Done {
        removed_bytes: u64,
        removed: usize,
        kept: Vec<String>,
        left: usize,
    },
}

/// Whether an agent or a program works in the folder `path` now: an agent
/// at work, waiting on a person or not reporting, or a pane whose shell
/// does not hold its terminal.
fn busy_in(engine: &dyn crate::engine::Engine, path: &str) -> Result<bool, String> {
    let panes = panes_in(engine, path)?;
    let agents = agents_listed(engine)?;
    let mut agent_panes = Vec::new();
    for agent in agents.iter().filter(|agent| panes.contains(&agent.pane_id)) {
        agent_panes.push(agent.pane_id.as_str());
        if !matches!(agent.agent_status.as_deref(), Some("idle" | "done")) {
            return Ok(true);
        }
    }
    for pane in panes
        .iter()
        .filter(|pane| !agent_panes.contains(&pane.as_str()))
    {
        let held = engine
            .pane_processes(READ_WAIT, pane)
            .map_err(String::from)?;
        if !held.foreground_pids.is_empty() && !held.shell_holds_terminal() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Deletes the build cache of the worktree of a finished `branch` in the
/// repository at `repo` (PRD mission-node-simple D-14, B20). Nothing else
/// is removed and no pane closes; a worktree being worked in waits.
pub(super) fn clear_build_cache(
    engine: &dyn crate::engine::Engine,
    node: &dyn crate::node_access::NodeLink,
    repo: &str,
    branch: &str,
) -> Result<Cleared, String> {
    let Some(path) = worktree_path(engine, repo, branch)? else {
        return Ok(Cleared::Gone);
    };
    if path.trim_end_matches('/') == repo.trim_end_matches('/') {
        return Ok(Cleared::ProjectFolder);
    }
    if busy_in(engine, &path)? {
        return Ok(Cleared::Busy { path });
    }
    match crate::worktree_cleanup::clear_build_cache(
        node,
        Path::new(repo),
        Path::new(&path),
        |folder| busy_in(engine, &folder.to_string_lossy()),
    )? {
        crate::worktree_cleanup::CacheClear::Busy => Ok(Cleared::Busy { path }),
        crate::worktree_cleanup::CacheClear::Done {
            removed_bytes,
            removed,
            kept,
            left,
        } => Ok(Cleared::Done {
            removed_bytes,
            removed,
            kept,
            left,
        }),
    }
}
