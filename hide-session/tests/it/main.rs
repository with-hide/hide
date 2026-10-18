//! hide-session's integration tests, built as one binary. Each module here was
//! a `tests/*.rs` target of its own once, and every target linked the
//! crate and all its dependencies again; docs/BUILD.md, "One integration
//! test binary per crate", owns the rule.

mod adapters;
mod conversation_cursor;
mod cursor;
mod environment;
mod links;
mod opencode_2;
mod usage;
mod user_turns;
