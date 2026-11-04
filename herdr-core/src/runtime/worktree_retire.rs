//! Retiring a worktree whose work has landed: the one decision Hide AI's
//! attempts (`memory_promotion`, `branch_decision`, `suggestion_apply`) and a
//! mission's merged nodes (`mission_cleanup`) share. The worktree is removed
//! when its tree is clean and no agent or running program is in it; the
//! branch always stays. Each caller logs what happened under its own subject
//! and decides whether to try again.
//!
//! Everything here runs off `Mutex<Runtime>`: the panes and agents are the
//! machine's Herdr, read through the engine, and the git work is the node's,
//! on the machine that holds the checkout, which is where the system
//! differences live (`hide-host`, `hide-platform`).

use hide_node_link::git::GitCommand;
use hide_node_link::protocol::Call;

/// How long one of Herdr's reads here may take.
const READ_WAIT: crate::engine::Wait = crate::engine::Wait::Each(std::time::Duration::from_secs(5));
/// How long one of the node's git calls here may take.
const NODE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Why a worktree stays.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Kept {
    /// The branch is checked out in the Project's own folder, which is the
    /// operator's, never a worktree to retire.
    OwnFolder,
    /// An agent other than the ones the caller ends works in it.
    AgentInside,
    /// A program runs in one of its panes.
    ProgramRunning,
    /// It holds uncommitted or untracked changes.
    Uncommitted,
}

impl Kept {
    /// The words the diagnostic log keeps.
    pub(super) fn reason(self) -> &'static str {
        match self {
            Kept::OwnFolder => "the Project's own folder",
            Kept::AgentInside => "another agent works in it",
            Kept::ProgramRunning => "a program is running in it",
            Kept::Uncommitted => "it holds uncommitted changes",
        }
    }

    /// Whether what keeps it may pass by itself: an agent or a program
    /// leaves, while changes and the Project's own folder are the
    /// operator's to settle.
    pub(super) fn passes(self) -> bool {
        matches!(self, Kept::AgentInside | Kept::ProgramRunning)
    }
}

/// What retiring a worktree came to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Outcome {
    /// The branch has no worktree, or it is no longer there.
    Gone,
    /// The worktree was removed; the branch stays.
    Removed,
    Kept(Kept),
    /// Herdr or the node refused or did not answer, which may pass.
    Failed(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Retirement {
    /// The worktree's folder, once Herdr named one.
    pub(super) path: Option<String>,
    pub(super) outcome: Outcome,
}

impl Retirement {
    /// The outcome and reason the diagnostic log keeps.
    pub(super) fn logged(&self) -> (&'static str, Option<String>) {
        match &self.outcome {
            Outcome::Gone => ("gone", None),
            Outcome::Removed => ("removed", None),
            Outcome::Kept(kept) => ("kept", Some(kept.reason().to_owned())),
            Outcome::Failed(reason) => ("failed", Some(reason.clone())),
        }
    }
}

/// Retires the worktree of `branch` in the repository at `root` (PRD
/// hide-ai-work-home D-18, D-24, engineering principle 14): the agents in
/// `own` (their panes) end first whatever else keeps it, then the worktree
/// is removed with its idle shells when the tree is clean and no other agent
/// or running program is in it. The branch always stays, for the merge or
/// the operator to reuse.
pub(super) fn retire_worktree(
    engine: &dyn crate::engine::Engine,
    node: &dyn crate::node_access::NodeLink,
    root: &str,
    branch: &str,
    own: &[String],
) -> Retirement {
    let mut at = None;
    let outcome = (|| -> Result<Outcome, String> {
        let Some(path) = worktree_path(engine, root, branch)? else {
            return Ok(Outcome::Gone);
        };
        at = Some(path.clone());
        if path.trim_end_matches('/') == root.trim_end_matches('/') {
            return Ok(Outcome::Kept(Kept::OwnFolder));
        }
        let agents = agents_listed(engine)?;
        let panes = panes_in(engine, &path)?;
        let (ending, others): (Vec<_>, Vec<_>) = agents
            .iter()
            .filter(|agent| panes.contains(&agent.pane_id))
            .partition(|agent| own.contains(&agent.pane_id));
        // The caller's agents end first, whatever else keeps the worktree.
        let ending = ending
            .into_iter()
            .map(|agent| agent.pane_id.clone())
            .collect::<Vec<_>>();
        crate::worktree_control::close_checkout_panes(
            engine,
            node,
            &[],
            &ending,
            crate::worktree_control::ProcessWait::for_folder_removal(true),
            crate::worktree_control::CONFIRM_TIMEOUT,
        )?;
        if !others.is_empty() {
            return Ok(Outcome::Kept(Kept::AgentInside));
        }
        // Idle shells close with the worktree; a program running in one
        // keeps it.
        let remaining = panes
            .into_iter()
            .filter(|pane| !ending.contains(pane))
            .collect::<Vec<_>>();
        for pane in &remaining {
            let held = engine
                .pane_processes(READ_WAIT, pane)
                .map_err(String::from)?;
            if !held.foreground_pids.is_empty() && !held.shell_holds_terminal() {
                return Ok(Outcome::Kept(Kept::ProgramRunning));
            }
        }
        let status = crate::node_access::call_as::<String>(
            node,
            Call::Git {
                root: path.clone(),
                command: GitCommand::Status,
            },
            NODE_TIMEOUT,
        )
        .map_err(|error| error.to_string())?;
        if !status.trim().is_empty() {
            return Ok(Outcome::Kept(Kept::Uncommitted));
        }
        crate::worktree_control::close_checkout_panes(
            engine,
            node,
            std::slice::from_ref(&path),
            &remaining,
            crate::worktree_control::ProcessWait::for_folder_removal(true),
            crate::worktree_control::CONFIRM_TIMEOUT,
        )?;
        crate::node_access::call_as::<()>(
            node,
            Call::WorktreeRetire {
                root: root.to_owned(),
                checkout: path,
                branch: branch.to_owned(),
                discard: false,
            },
            NODE_TIMEOUT,
        )
        .map_err(|error| error.to_string())?;
        Ok(Outcome::Removed)
    })();
    Retirement {
        path: at,
        outcome: outcome.unwrap_or_else(Outcome::Failed),
    }
}

/// The worktree of `branch` in the repository at `repo`, `None` when there
/// is none.
pub(super) fn worktree_path(
    engine: &dyn crate::engine::Engine,
    repo: &str,
    branch: &str,
) -> Result<Option<String>, String> {
    engine
        .worktree_path(READ_WAIT, repo, branch)
        .map_err(String::from)
}

/// Every agent Herdr runs now.
pub(super) fn agents_listed(
    engine: &dyn crate::engine::Engine,
) -> Result<Vec<crate::session_sync::ProjectedAgent>, String> {
    engine.agents(READ_WAIT).map_err(|error| match error {
        crate::engine::EngineError::Answer(_) => "agent.list is malformed".to_owned(),
        error => error.to_string(),
    })
}

/// Whether `cwd` is `path` or below it, however either spells its end: a
/// pane missed here would be taken for no one's, and its folder removed.
fn within(cwd: &str, path: &str) -> bool {
    let (cwd, path) = (cwd.trim_end_matches('/'), path.trim_end_matches('/'));
    cwd == path
        || cwd
            .strip_prefix(path)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The panes Herdr runs in the folder `path` or below it.
pub(super) fn panes_in(
    engine: &dyn crate::engine::Engine,
    path: &str,
) -> Result<Vec<String>, String> {
    let snapshot = engine.session(READ_WAIT).map_err(|error| match error {
        crate::engine::EngineError::Answer(_) => "snapshot_unavailable".to_owned(),
        error => error.to_string(),
    })?;
    Ok(snapshot
        .panes
        .into_iter()
        .filter(|pane| pane.cwd.as_deref().is_some_and(|cwd| within(cwd, path)))
        .map(|pane| pane.pane_id)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    /// One repository's machine as Herdr and the node answer it: the
    /// branch's worktree, the panes in it, which of them runs an agent or
    /// a program, whether its tree has changes, and the removals asked.
    #[derive(Default)]
    struct Repository {
        worktree: Option<String>,
        panes: Vec<(String, String)>,
        agents: Vec<String>,
        busy: Vec<String>,
        changed: bool,
        removals: Vec<(String, bool)>,
    }

    type Shared = Arc<Mutex<Repository>>;

    fn herdr(repository: &Shared) -> Arc<crate::engine::fake::FakeEngine<'static>> {
        use crate::engine::fake::{processes, reply, session};
        let repository = Arc::clone(repository);
        crate::engine::fake::FakeEngine::start(move |method, params| {
            let mut repository = repository.lock().unwrap();
            match method {
                "worktree_path" => reply(
                    repository
                        .worktree
                        .clone()
                        .filter(|_| params["branch"] == "fix"),
                ),
                "session" => reply(session(json!({
                    "panes": repository.panes.iter().map(|(pane, cwd)| json!({
                        "pane_id": pane, "workspace_id": "w2", "tab_id": "w2:t1",
                        "terminal_id": "t", "agent_status": "idle", "cwd": cwd}))
                        .collect::<Vec<_>>(),
                    "agents": repository.agents.iter().map(|pane| json!({"pane_id": pane,
                        "workspace_id": "w2", "tab_id": "w2:t1", "terminal_id": "t",
                        "agent_status": "idle", "name": "worker"})).collect::<Vec<_>>(),
                }))),
                "pane_processes" => {
                    let busy = repository.busy.iter().any(|pane| *pane == params["pane"]);
                    reply(processes(&if busy {
                        json!({"shell_pid": 100, "foreground_process_group_id": 200,
                            "foreground_processes": [{"pid": 200, "name": "cargo"}]})
                    } else {
                        json!({"shell_pid": 100, "foreground_process_group_id": 100,
                            "foreground_processes": [{"pid": 100, "name": "zsh"}]})
                    }))
                }
                "close_pane" => {
                    repository.panes.retain(|(pane, _)| *pane != params["pane"]);
                    reply(())
                }
                other => panic!("unexpected {other}"),
            }
        })
    }

    struct Machine(Shared);

    impl crate::node_access::NodeLink for Machine {
        fn call(
            &self,
            call: Call,
            _timeout: std::time::Duration,
        ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
            let mut repository = self.0.lock().unwrap();
            Ok(crate::node_access::LinkAnswer::Parsed(match call {
                Call::Git {
                    command: GitCommand::Status,
                    ..
                } => json!(if repository.changed {
                    "?? notes.txt\n"
                } else {
                    ""
                }),
                Call::WorktreeRetire {
                    branch, discard, ..
                } => {
                    repository.removals.push((branch, discard));
                    repository.worktree = None;
                    serde_json::Value::Null
                }
                other => panic!("unexpected {other:?}"),
            }))
        }
    }

    /// A merged node's worktree, its worker's pane and a shell beside it.
    fn merged(setup: impl FnOnce(&mut Repository)) -> Shared {
        let repository: Shared = Arc::default();
        {
            let mut repository = repository.lock().unwrap();
            repository.worktree = Some("/r-fix".into());
            repository.panes = vec![
                ("w2:p1".into(), "/r-fix".into()),
                ("w2:p2".into(), "/r-fix/web".into()),
            ];
            setup(&mut repository);
        }
        repository
    }

    /// Operator decision 2026-11-02, with no agent of the caller's to end
    /// (a mission's): a clean worktree whose panes are idle shells is
    /// removed with them and its branch kept; changes, an agent or a running
    /// program keep it and every pane in it, and each says why.
    #[test]
    fn a_worktree_is_removed_only_when_clean_and_nothing_runs_in_it() {
        type Case = (Outcome, fn(&mut Repository));
        let cases: [Case; 4] = [
            (Outcome::Removed, |_| {}),
            (Outcome::Kept(Kept::Uncommitted), |repository| {
                repository.changed = true
            }),
            (Outcome::Kept(Kept::AgentInside), |repository| {
                repository.agents.push("w2:p1".into())
            }),
            (Outcome::Kept(Kept::ProgramRunning), |repository| {
                repository.busy.push("w2:p2".into())
            }),
        ];
        for (expected, setup) in cases {
            let repository = merged(setup);
            let herdr = herdr(&repository);
            let retirement = retire_worktree(
                herdr.engine().as_ref(),
                &Machine(Arc::clone(&repository)),
                "/r",
                "fix",
                &[],
            );
            assert_eq!(retirement.path.as_deref(), Some("/r-fix"), "{expected:?}");
            assert_eq!(retirement.outcome, expected);
            let held = repository.lock().unwrap();
            if expected == Outcome::Removed {
                assert_eq!(
                    held.removals,
                    [("fix".to_owned(), false)],
                    "the branch is kept"
                );
                assert!(held.panes.is_empty(), "its idle shells closed with it");
            } else {
                assert!(held.removals.is_empty(), "{expected:?}");
                assert_eq!(held.panes.len(), 2, "{expected:?}: no pane closed");
            }
        }
        let gone = merged(|repository| repository.worktree = None);
        let herdr = herdr(&gone);
        let retirement = retire_worktree(herdr.engine().as_ref(), &Machine(gone), "/r", "fix", &[]);
        assert_eq!(retirement.outcome, Outcome::Gone);
    }

    /// A folder's panes are those in it or below it, and a sibling that only
    /// shares the name's start is not, whatever the spelling of the end.
    #[test]
    fn a_pane_is_within_a_folder_only_at_a_path_boundary() {
        assert!(within("/r/wt", "/r/wt"));
        assert!(within("/r/wt/", "/r/wt"));
        assert!(within("/r/wt/docs", "/r/wt/"));
        assert!(!within("/r/wt-other", "/r/wt"));
        assert!(!within("/r", "/r/wt"));
    }
}
