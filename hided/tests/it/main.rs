//! hided's integration tests, built as one binary. Each module here was
//! a `tests/*.rs` target of its own once, and every target linked the
//! crate and all its dependencies again; docs/BUILD.md, "One integration
//! test binary per crate", owns the rule.

mod access;
mod agent_exec;
mod always_on_update;
mod arguments;
mod automations;
mod browser_control;
mod closed_stdout;
mod connect_build;
mod connect_stop;
mod device_own_engine;
mod fixtures;
mod guest_node;
mod handshake;
mod mobile;
mod mobile_relay;
mod node_contract;
#[cfg(unix)]
mod node_home;
mod node_link_facts;
mod node_owner;
mod node_session_activity;
mod old_state_folder;
mod opener_lifecycle;
mod pairing;
mod remote_delivery;
mod seat;
mod shell_exec;
#[allow(dead_code)] // `hide agent exec` needs only the table form.
#[path = "../../../hide-agent-hooks/tests/it/stand_in_daemon.rs"]
mod stand_in_daemon;
#[allow(dead_code)] // This fixture needs readiness only, not stand-in placement.
#[path = "../../../hide-platform/tests/it/stand_ins.rs"]
mod stand_ins;
mod stress_gate_canary;
#[cfg(unix)]
mod support;
mod system_ssh;
mod windows_ssh_device;
mod wsl_device;
