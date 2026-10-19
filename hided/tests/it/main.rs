//! hided's integration tests, built as one binary. Each module here was
//! a `tests/*.rs` target of its own once, and every target linked the
//! crate and all its dependencies again; docs/BUILD.md, "One integration
//! test binary per crate", owns the rule.

mod browser_control;
mod closed_stdout;
mod connect_build;
mod connect_stop;
mod core_move;
mod core_move_checks;
mod handshake;
mod held_letter_notice;
mod herdr_handoff;
mod mobile;
mod node_contract;
mod node_home;
mod node_link_facts;
mod node_owner;
mod node_session_activity;
mod observer_review;
mod opener_lifecycle;
#[path = "support/private_herdr.rs"]
mod private_herdr;
mod real_herdr;
mod remote_core;
mod remote_delivery;
mod responsive_state;
mod seat;
mod shell_exec;
#[cfg(unix)]
mod support;
mod terminal_children;
