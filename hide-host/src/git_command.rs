//! The fixed git commands the core has a node run in one of its
//! repositories (`hide_node_link::git::GitCommand`): reading a worktree's
//! state, switching its branch, fetching a branch, and the branch settings
//! Hide keeps in the repository's configuration.

use std::path::Path;

use hide_node_link::git::GitCommand;

use crate::error::{ErrorCode, HostError, HostResult};

/// How much of a search's output is read before the answer is cut to
/// `GREP_OUTPUT_BYTES`; a search that prints more fails as too broad.
const GREP_READ_BYTES: u64 = 1024 * 1024;

/// Runs `command` in `root` and answers its output, trimmed. A read, a
/// branch check and a fetch are bounded (`worktrees::git`); a checkout and a
/// branch setting get the long bound of a change (`git_run::CHANGE_DEADLINE`).
pub fn run(root: &Path, command: &GitCommand) -> HostResult<String> {
    let args = command
        .args()
        .map_err(|reason| HostError::new(ErrorCode::InvalidRequest, reason))?;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let failed = |message: String| HostError::new(ErrorCode::Io, message);
    match command {
        GitCommand::Status
        | GitCommand::CommonDir
        | GitCommand::RefCommit { .. }
        | GitCommand::CountCommits { .. }
        | GitCommand::HasLocalBranch { .. }
        | GitCommand::FetchBranch { .. } => crate::worktrees::git(root, &args)
            .map(|output| output.trim().to_owned())
            .map_err(failed),
        GitCommand::DocumentTime { path } => {
            let status = crate::worktrees::git(
                root,
                &[
                    "--literal-pathspecs",
                    "status",
                    "--porcelain=v1",
                    "--",
                    path,
                ],
            )
            .map_err(failed)?;
            if !status.trim().is_empty() {
                return Ok(String::new());
            }
            crate::worktrees::git(root, &args)
                .map(|output| output.trim().to_owned())
                .map_err(failed)
        }
        // A path may begin or end with a space, so the list is not trimmed.
        GitCommand::ListFiles => crate::worktrees::git(root, &args).map_err(failed),
        GitCommand::Log { .. } => crate::worktrees::git(root, &args)
            .map(|output| output.trim_end().to_owned())
            .map_err(failed),
        GitCommand::Grep { .. } => {
            let output = crate::git_run::output(
                crate::git_run::at(root).args(&args),
                crate::git_run::DEADLINE,
                Some(GREP_READ_BYTES),
            )
            .map_err(|error| failed(format!("git could not be run: {error}")))?
            .ok_or_else(|| {
                failed(format!(
                    "git grep did not finish within {} s",
                    crate::git_run::DEADLINE.as_secs()
                ))
            })?;
            // `git grep` exits 1 when nothing matched, which is an answer.
            if output.status.success() || output.status.code() == Some(1) {
                let kept = output
                    .stdout
                    .len()
                    .min(hide_node_link::git::GREP_OUTPUT_BYTES as usize);
                return Ok(String::from_utf8_lossy(&output.stdout[..kept]).into_owned());
            }
            Err(failed(format!("git grep exited with {}", output.status)))
        }
        GitCommand::CurrentBranch
        | GitCommand::Checkout { .. }
        | GitCommand::SetBranchConfig { .. }
        | GitCommand::UnsetBranchConfig { .. } => {
            let output = crate::git_run::output(
                crate::git_run::at(root).args(&args),
                crate::git_run::CHANGE_DEADLINE,
                None,
            )
            .map_err(|error| failed(format!("git could not be run: {error}")))?
            .ok_or_else(|| {
                failed(format!(
                    "git {} did not finish within {} minutes and was stopped",
                    args[0],
                    crate::git_run::CHANGE_DEADLINE.as_secs() / 60
                ))
            })?;
            // `config --unset-all` exits 5 when the setting is not there,
            // which is the state it was asked for.
            let unset_absent = matches!(command, GitCommand::UnsetBranchConfig { .. })
                && output.status.code() == Some(5);
            if output.status.success() || unset_absent {
                return Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned());
            }
            let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            Err(failed(if detail.is_empty() {
                format!("git {} exited with {}", args[0], output.status)
            } else {
                format!("git {}: {detail}", args[0])
            }))
        }
    }
}
