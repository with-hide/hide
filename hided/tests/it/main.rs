//! hided's integration tests, built as one binary. Each module here was
//! a `tests/*.rs` target of its own once, and every target linked the
//! crate and all its dependencies again; docs/BUILD.md, "One integration
//! test binary per crate", owns the rule.

mod agent_exec;
mod arguments;
mod browser_control;
mod closed_stdout;
mod connect_build;
mod connect_stop;
mod core_move_checks;
mod device_own_engine;
mod handshake;
mod held_letter_notice;
mod mobile;
mod mobile_relay;
mod node_contract;
#[cfg(unix)]
mod node_home;
mod node_link_facts;
mod node_owner;
mod node_session_activity;
mod opener_lifecycle;
mod remote_delivery;
mod seat;
mod shell_exec;
#[allow(dead_code)] // This fixture needs readiness only, not stand-in placement.
#[path = "../../../hide-platform/tests/it/stand_ins.rs"]
mod stand_ins;
#[cfg(unix)]
mod support;
mod system_ssh;
mod windows_ssh_device;
mod wsl_device;
