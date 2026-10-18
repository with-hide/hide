//! hide-agent-hooks' integration tests, built as one binary. Each module here was
//! a `tests/*.rs` target of its own once, and every target linked the
//! crate and all its dependencies again; docs/BUILD.md, "One integration
//! test binary per crate", owns the rule.

mod adapter_contract;
mod agy_binary;
mod agy_hooks;
mod bell_intake;
mod captured_route;
mod claude_subagent_counts;
mod codex_binary;
mod codex_folder_trust;
mod codex_trust;
mod grok_cursor_hooks;
mod guardrails;
mod guidance_hook;
mod hook_deadline;
mod letter_origin;
mod lossless_install;
mod opencode_helper;
mod opencode_plugin;
mod pi_extension;
mod programs;
mod session_start_probes;
mod spawn_guard;
#[path = "../../../hide-platform/tests/it/stand_ins.rs"]
mod stand_ins;
mod windows_hook_command;
mod windows_scripts;

/// A validated route for public installer fixtures, independent of the account.
fn hook_target(helper: &std::path::Path) -> hide_agent_hooks::route::HookTarget<'_> {
    hide_agent_hooks::route::HookTarget::new(helper, std::path::Path::new(HOOK_STATE)).unwrap()
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
