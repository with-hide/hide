//! The fixtures more than one module shares: the device link's, which the
//! `remote_delivery` and `node_contract` modules use, and the fake
//! `tailscale`; declared once, so the binary compiles each once.

pub mod core_move;
pub mod fake_tailscale;
pub mod remote_core;
pub mod remote_delivery;
pub mod run;
pub mod screen;
pub mod sshd;
