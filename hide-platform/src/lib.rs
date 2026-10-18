//! Operating-system primitives, one module per concern.
//!
//! Every function here answers the same way on macOS, Linux and Windows, or
//! says `ErrorKind::Unsupported` where the system cannot answer; none guesses
//! a default. `tests/` states each module's contract in terms a caller can
//! observe and runs unchanged on all three systems.

pub mod fs;
pub mod host;
pub mod ipc;
pub mod listeners;
pub mod path;
pub mod process;
pub mod programs;
pub mod stdio;
pub mod time;
pub mod user_agents;
pub mod watch;
