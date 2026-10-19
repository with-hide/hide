//! hide-ai's integration tests, built as one binary. Each module here was
//! a `tests/*.rs` target of its own once, and every target linked the
//! crate and all its dependencies again; docs/BUILD.md, "One integration
//! test binary per crate", owns the rule.

/// The fakes in `fixtures/` read their behaviour from `FAKE_*` variables in
/// this process's environment, which every module here shares: a test that
/// sets one, or starts a fake that reads one, holds this lock until its
/// fake has started.
static FAKE_ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

mod claude_cli;
mod codex_app_server;
mod omp_sdk;
mod process_ownership;
mod search_path;
mod text_clis;
