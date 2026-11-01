//! hide-host's integration tests, built as one binary. Each module here was
//! a `tests/*.rs` target of its own once, and every target linked the
//! crate and all its dependencies again; docs/BUILD.md, "One integration
//! test binary per crate", owns the rule.

mod branch_merge;
mod clone;
mod copy;
mod document;
mod git;
mod item_identity;
mod mutate;
mod project;
mod register;
mod save;
mod stamps;
mod usage;
mod worktrees;
