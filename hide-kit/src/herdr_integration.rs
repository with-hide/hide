//! Retires Herdr's own integration for an agent where Hide installed it
//! (PRD settings-cleanup D-13, engine-bundle-4-herdr-removal D-12).
//!
//! Earlier builds ran `herdr integration install <agent>` so Herdr learned
//! an agent's session; Hide runs no Herdr now, so the kit installs nothing of
//! Herdr's and takes out only what the record says Hide put there, without
//! a Herdr binary: the record's `herdr:<agent>` piece and the hook entries
//! it recorded Herdr writing (`Record::herdr_hook_entries`, Codex's). An
//! entry is taken out only when it is still byte for byte what was recorded;
//! one the operator edited stays and is logged. An integration the operator
//! installed first has no piece and stays. A write that fails keeps the
//! piece, so the next pass tries again. This retirement stays for operators
//! who skip the releases that ran Herdr.

use crate::record::Record;
use crate::{KitTarget, RemoveOutcome};

/// The record's piece for `agent`'s Herdr integration.
pub(crate) fn code(agent: &str) -> String {
    format!("herdr:{agent}")
}

/// Takes `agent`'s Herdr integration out when the record holds its piece;
/// `None` when it holds none, so there is nothing of Hide's to retire.
pub(crate) fn retire(
    target: &KitTarget,
    record: &mut Record,
    agent: &str,
) -> Option<RemoveOutcome> {
    let piece = code(agent);
    if !record.contains_piece(&piece) {
        return None;
    }
    let recorded = record.herdr_hook_entries(agent).to_vec();
    let outcome = if recorded.is_empty() {
        // Herdr's integration for this agent was never recorded entry by
        // entry, so nothing of it can be told from the operator's; the
        // piece goes and the files stay.
        log(agent, "herdr_integration.unrecorded_left", 0, 0);
        RemoveOutcome::Absent
    } else {
        match hide_agent_hooks::codex_trust::remove_herdr_entries(&target.home, &recorded) {
            Ok(done) => {
                if done.kept_or_gone > 0 {
                    log(
                        agent,
                        "herdr_integration.entries_kept",
                        done.removed,
                        done.kept_or_gone,
                    );
                }
                if done.removed > 0 {
                    RemoveOutcome::Removed
                } else {
                    RemoveOutcome::Absent
                }
            }
            Err(failure) => {
                return Some(RemoveOutcome::Failed {
                    reason: failure.message(),
                });
            }
        }
    };
    record.forget_piece(&piece);
    Some(outcome)
}

fn log(agent: &str, kind: &str, removed: usize, kept_or_gone: usize) {
    eprintln!(
        "{}",
        serde_json::json!({
            "component": "kit",
            "kind": kind,
            "agent": agent,
            "removed": removed,
            "kept_or_gone": kept_or_gone,
        })
    );
}
