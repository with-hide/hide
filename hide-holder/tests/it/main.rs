//! The holder's contract, checked against the built `hide-holder` started
//! as a node starts it, without a node (docs/TESTING.md, The pane holder).

mod asks;
mod connections;
#[cfg(windows)]
mod conpty;
mod fixture_v1;
mod folders;
mod idle;
mod lifecycle;
mod meaning;
mod node;
mod records;
mod role;
mod screen;
mod support;
