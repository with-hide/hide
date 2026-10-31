//! The holder's contract, checked against the built `hide-holder` started
//! as a node starts it, without a node (docs/TESTING.md, The pane holder).

mod connections;
#[cfg(windows)]
mod conpty;
mod folders;
mod lifecycle;
mod role;
mod support;
