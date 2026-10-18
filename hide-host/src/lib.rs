//! The file host contract: what Hide does to files on the machine that owns
//! them, always through an opened checkout root.
//!
//! Every operation takes a `cap_std::fs::Dir` for the checkout (or a folder
//! under it) and a path relative to it, so a `..`, an absolute path or a
//! symlink that leaves the directory is refused by the handle rather than by a
//! string check, and a checkout renamed or replaced after it was opened cannot
//! redirect the work. The daemon and the core call these functions in process
//! for this machine; `hided node serve` serves the same functions over one SSH
//! exec channel on a registered device, so a local and a remote checkout obey
//! one contract (PRD S5.5 D-05, D-06).

pub mod ai;
pub mod attachments;
pub mod bytes;
pub mod catalog;
pub mod cleanup;
pub mod clone;
pub mod copy;
pub mod disk;
pub mod disk_layers;
pub mod document;
#[cfg(all(test, unix))]
mod executable_fixture;
pub mod factory;
pub mod feedback;
pub mod gh;
pub mod git;
pub mod git_command;
pub mod git_watch;
pub mod home;
pub mod index;
pub mod kit;
pub mod label_lock;
pub mod link_bridge;
pub mod list;
pub mod mutate;
pub mod opencode_fork;
pub mod pane_peer;
pub mod panes;
pub mod ports;
pub mod project;
pub mod register;
pub mod reporting;
pub mod root;
pub mod save;
pub mod serve;
pub mod sessions;
pub mod shell_integration;
pub mod usage;
pub mod worktrees;

pub use hide_node_link::{ErrorCode, HostError, HostResult};
pub use hide_node_link::{error, protocol};
pub use root::{Root, RootIdentity};
