//! The kit's step that has Codex trust Hide's own hooks (PRD codex-hook-trust).
//!
//! `hide_agent_hooks::codex_trust` does the work; this decides when. A pass
//! that finds the Codex hook part in place asks Codex to record trust for it,
//! so Codex starts without its "Hooks need review" screen for Hide's entries.
//!
//! A failure is the part's one-line reason and its detail goes to the log.
//! `status` runs every few seconds while Settings is open and starts no
//! process, so it cannot ask Codex again; it repeats what the last pass found
//! instead. That is a memory of this process, not a record: a connection or
//! a launch is a new process and starts with a pass (`apply`), and a pass
//! replaces it, so a failure the operator then fixed in Codex stays on the row
//! until the next pass or Reinstall.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use hide_agent_hooks::account::AccountAt;
use hide_agent_hooks::codex_trust::{
    TrustFailure, TrustFailureKind, TrustOutcome, trust_own_hooks,
};

use crate::KitTarget;

/// What the last pass in this process found, by account home: the reason the
/// part shows, present only for a failure.
static LAST_FAILURE: Mutex<BTreeMap<PathBuf, String>> = Mutex::new(BTreeMap::new());

fn memory() -> std::sync::MutexGuard<'static, BTreeMap<PathBuf, String>> {
    LAST_FAILURE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Asks Codex to trust Hide's entries and answers the reason the part is not
/// ready, when it is not. Nothing is asked of a machine with no Codex, and a
/// Codex with no hook trust says nothing (D-07).
pub(crate) fn ensure(target: &KitTarget) -> Option<String> {
    ensure_at(target, AccountAt::Home(&target.home), &target.home)
}

/// [`ensure`] for the Codex account `at`, remembered by `key`: the home for
/// the default account, a profile account's folder for its own.
pub(crate) fn ensure_at(target: &KitTarget, at: AccountAt<'_>, key: &Path) -> Option<String> {
    let helper = target.kit_dir.join(hide_agent_hooks::HELPER_BINARY_NAME);
    let state = crate::layout::state_dir_from_process(&target.home);
    let hook_target = match hide_agent_hooks::route::HookTarget::new(&helper, &state)
        .and_then(|hook| hook.of_install(&target.install))
    {
        Ok(target) => target,
        Err(reason) => {
            memory().insert(key.to_path_buf(), reason.to_owned());
            return Some(reason.to_owned());
        }
    };
    let reason = match target.codex.as_ref() {
        None => None,
        Some(codex) => match trust_own_hooks(codex, at, hook_target, &target.stop) {
            TrustOutcome::Unsupported | TrustOutcome::Trusted { .. } => None,
            // Hide is quitting: the pass is abandoned, so nothing is said of
            // the part and nothing is remembered.
            TrustOutcome::Failed(failure) if failure.kind == TrustFailureKind::Stopped => {
                return None;
            }
            TrustOutcome::Failed(failure) => Some(reason(key, &failure)),
        },
    };
    let mut remembered = memory();
    match &reason {
        Some(reason) => remembered.insert(key.to_path_buf(), reason.clone()),
        None => remembered.remove(key),
    };
    reason
}

/// The reason the last pass left, for a read that asks Codex nothing.
pub(crate) fn remembered(target: &KitTarget) -> Option<String> {
    remembered_at(&target.home)
}

/// [`remembered`] for the account remembered by `key` ([`ensure_at`]).
pub(crate) fn remembered_at(key: &Path) -> Option<String> {
    memory().get(key).cloned()
}

fn reason(home: &std::path::Path, failure: &TrustFailure) -> String {
    eprintln!(
        "{}",
        serde_json::json!({
            "component": "kit",
            "kind": "codex_trust_failed",
            "home": home.display().to_string(),
            "cause": format!("{:?}", failure.kind),
            "detail": failure.detail,
        })
    );
    format!(
        "Codex has not trusted Hide's hook: {}; it will ask you to review it",
        failure.kind.summary()
    )
}
