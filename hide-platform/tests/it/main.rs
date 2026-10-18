//! hide-platform's integration tests, built as one binary. Each module here was
//! a `tests/*.rs` target of its own once, and every target linked the
//! crate and all its dependencies again; docs/BUILD.md, "One integration
//! test binary per crate", owns the rule.

mod fs;
mod host;
mod ipc;
mod listeners;
mod path;
mod process;
mod programs;
#[cfg(unix)]
mod stand_ins;
mod stdio;
mod time;
mod user_agents;
mod watch;
