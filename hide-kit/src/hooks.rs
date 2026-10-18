//! Hide's entries in each agent runtime's hook file, through
//! `hide-agent-hooks`, which owns the file format and the rule that another
//! tool's entry is never rewritten (D-15).
//!
//! The entry runs `<kit_dir>/hide-agent-hooks` (`.exe` on Windows) and exits
//! quietly when that file is gone (D-11), so it points at the kit folder that
//! outlives a build: the app bundle on this Mac, the helper root's `current`
//! on a device.

use std::path::{Path, PathBuf};

use hide_agent_hooks::route::HookTarget;
use hide_agent_hooks::{AgentRuntime, HookStatus, InstallFailure, MemoryCompatibility};

use crate::{KitTarget, Observed, RemoveOutcome};

fn helper(target: &KitTarget) -> PathBuf {
    target.kit_dir.join(hide_agent_hooks::HELPER_BINARY_NAME)
}

pub(crate) fn observe(target: &KitTarget, runtime: AgentRuntime) -> Observed {
    let status = hide_agent_hooks::install::status(runtime, &target.home);
    let observed = match status {
        HookStatus::RuntimeAbsent => {
            return Observed::Absent(format!("{} is not set up on this machine", runtime.label()));
        }
        // Reading a file never says Off: the kit's record does (`HookStatus::Off`).
        HookStatus::NotInstalled | HookStatus::Off => Observed::Missing,
        HookStatus::Outdated { version } => Observed::Stale(format!(
            "version {version} of the hook is there; this build writes version {}",
            hide_agent_hooks::HOOK_VERSION
        )),
        HookStatus::Failed {
            reason: InstallFailure::HelperMissing { helper, .. },
        } => Observed::Stale(format!("the hook points at {helper}, which is gone")),
        HookStatus::Failed { reason } => return Observed::Blocked(reason.message()),
        HookStatus::Installed { .. } => {
            let wanted = helper(target);
            match hide_agent_hooks::installed_helper_path(runtime, &target.home) {
                Some(found) if Path::new(&found) == wanted => {
                    let state = crate::layout::state_dir_from_process(&target.home);
                    if hide_agent_hooks::install::matches_install(
                        runtime,
                        &target.home,
                        HookTarget::new(&wanted, &state),
                    ) {
                        Observed::Current
                    } else if hide_agent_hooks::install::matches_helper(
                        runtime,
                        &target.home,
                        &wanted,
                    ) {
                        Observed::Stale("the hook's captured state route changed".to_owned())
                    } else {
                        Observed::Edited(
                            "Hide's hook entries were edited; Reinstall puts Hide's back"
                                .to_owned(),
                        )
                    }
                }
                Some(found) => Observed::Stale(format!(
                    "the hook runs {found}, not this build's {}",
                    wanted.display()
                )),
                None => Observed::Stale("the hook's command is not one Hide wrote".to_owned()),
            }
        }
    };
    if matches!(observed, Observed::Current) {
        return observed;
    }
    if !helper(target).is_file() {
        return Observed::Blocked(format!(
            "this build has no hook helper at {}",
            helper(target).display()
        ));
    }
    // Only a CLI that answers with a version older than the hook needs is a
    // reason not to install; one Hide cannot find may be on a PATH the
    // daemon does not see, and a hook for it does no harm.
    if let MemoryCompatibility::UpdateRequired {
        installed_version: Some(installed),
        minimum_version,
    } = hide_agent_hooks::runtime_compatibility(runtime, &target.home)
    {
        return Observed::Blocked(format!(
            "{} {installed} is older than {minimum_version}, which Hide's hook needs; update it",
            runtime.label()
        ));
    }
    observed
}

pub(crate) fn install(target: &KitTarget, runtime: AgentRuntime) -> Result<(), String> {
    let helper = helper(target);
    let state = crate::layout::state_dir_from_process(&target.home);
    hide_agent_hooks::install(runtime, &target.home, HookTarget::new(&helper, &state))
        .map(|_| ())
        .map_err(|failure| failure.message())
}

/// Takes out Hide's hook entries and, with them, the folder trust Hide
/// recorded for the worktrees it started this agent in, as switching agy off
/// does (docs/agent-hooks.md, Folder trust for the worktrees Hide makes).
pub(crate) fn remove(target: &KitTarget, runtime: AgentRuntime) -> RemoveOutcome {
    use hide_agent_hooks::folder_trust::{self, Agent, Tools};
    let removed = match hide_agent_hooks::remove(runtime, &target.home) {
        Ok(outcome) => outcome.removed_entries,
        Err(failure) => {
            return RemoveOutcome::Failed {
                reason: failure.message(),
            };
        }
    };
    let agent = match runtime {
        AgentRuntime::ClaudeCode => Agent::ClaudeCode,
        // Codex's trust is taken out by Codex itself; with no Codex left to
        // ask, its entries and Hide's record of them stay for a later pass
        // that finds one, and nothing reads them meanwhile.
        AgentRuntime::Codex if target.codex.is_none() => {
            return if removed > 0 {
                RemoveOutcome::Removed
            } else {
                RemoveOutcome::Absent
            };
        }
        AgentRuntime::Codex => Agent::Codex,
    };
    let tools = Tools {
        codex: target.codex.as_deref(),
        stop: &target.stop,
    };
    match folder_trust::forget_all(agent, &target.home, tools) {
        Ok(forgotten) if removed + forgotten > 0 => RemoveOutcome::Removed,
        Ok(_) => RemoveOutcome::Absent,
        Err(failure) => RemoveOutcome::Failed {
            reason: failure.message(),
        },
    }
}

/// An agent switched off: Hide's entries come out, and only Hide's. The
/// part then reads Off, and no pass puts it back until the operator switches
/// the agent on.
pub(crate) fn turn_off(target: &KitTarget, runtime: AgentRuntime) -> Result<(), String> {
    match remove(target, runtime) {
        RemoveOutcome::Removed | RemoveOutcome::Absent => Ok(()),
        RemoveOutcome::Kept { reason } | RemoveOutcome::Failed { reason } => Err(reason),
    }
}
