//! hide-agent-hooks' integration tests, built as one binary. Each module here was
//! a `tests/*.rs` target of its own once, and every target linked the
//! crate and all its dependencies again; docs/BUILD.md, "One integration
//! test binary per crate", owns the rule.

mod adapter_contract;
mod agent_version;
mod agy_binary;
mod agy_hooks;
mod bell_intake;
mod captured_route;
mod claude_subagent_counts;
mod codex_binary;
mod codex_folder_trust;
mod codex_subagent_counts;
mod codex_trust;
mod grok_cursor_hooks;
mod guardrails;
mod guidance_hook;
mod hook_deadline;
mod host_helper;
mod letter_origin;
mod lossless_install;
mod opencode_helper;
mod opencode_plugin;
mod opener_guard;
mod pi_extension;
mod plan_record;
mod pre_tool;
mod programs;
mod records_refused;
mod records_route;
mod session_start_probes;
mod stand_in_daemon;
#[path = "../../../hide-platform/tests/it/stand_ins.rs"]
mod stand_ins;
mod windows_hook_command;
mod windows_scripts;

// These modules share one process under `cargo test`, and no test here
// changes its environment: a case that has to read `HIDE_STATE_DIR` or
// `HIDE_INSTALL` runs in a child of its own (`Command::env`), so no lock
// guards the environment and no other test's thread can see a value a test
// named (docs/TESTING.md, and `records_route` for the child).

/// A validated route for public installer fixtures, independent of the account.
fn hook_target(helper: &std::path::Path) -> hide_agent_hooks::route::HookTarget<'_> {
    hide_agent_hooks::route::HookTarget::new(helper, std::path::Path::new(HOOK_STATE)).unwrap()
}

/// A validated route naming `state`: the route a kit writes for the daemon
/// that reads it. A hook whose route names a folder nothing made writes
/// nothing anywhere, so a test that runs its hook must name the state folder
/// the hook's own home reads, as the kit does
/// (`hide_agent_hooks::route::records_folder_of`).
#[cfg(windows)]
fn hook_target_at<'a>(
    helper: &'a std::path::Path,
    state: &'a std::path::Path,
) -> hide_agent_hooks::route::HookTarget<'a> {
    hide_agent_hooks::route::HookTarget::new(helper, state).unwrap()
}

/// The state folder a home's own daemon uses when nothing moves it, which a
/// route may name before the daemon has made it: its records folder is then
/// the account's own.
#[cfg(windows)]
fn state_of(home: &std::path::Path) -> std::path::PathBuf {
    hide_agent_hooks::state_folder::default_state_dir(home)
}

const HOOK_STATE: &str = if cfg!(windows) {
    r"C:\captured-state"
} else {
    "/captured-state"
};

fn routed_script(pinned: &str) -> String {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(HOOK_STATE);
    let (header, body) = pinned.split_once('\n').unwrap();
    let body = body.replace(
        "const ROUTE = [];",
        &format!("const ROUTE = [\"--state-dir-b64\",\"{encoded}\"];"),
    );
    // The route changes the content hash; all other pinned bytes stay exact.
    let (marker, _) = header.split_once(" sha256=").unwrap();
    format!(
        "{marker} sha256={:x}\n{body}",
        Sha256::digest(body.as_bytes())
    )
}
