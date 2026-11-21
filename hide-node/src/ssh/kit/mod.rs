//! A device's kit (with-hide/hide-private#501): the programs a release
//! carries for each device platform, how a core of any system fetches and
//! checks them, and which source a device's programs come from.
//!
//! - [`contract`]: `contracts/device-kits.json`, the one place a platform is
//!   named, and which platforms are refused before anything is installed.
//! - `manifest`: the release's `device-bundles.json`, bound to this build's
//!   compiled version, commit and order.
//! - `archive`: unpacking a kit and checking each program's header.
//! - `cache`: the verified slots under the account's home, within one
//!   budget.
//! - `select`: the one lookup rule `HelperPackages` follows.

mod archive;
mod cache;
pub(in crate::ssh) mod contract;
mod manifest;
mod select;

pub use manifest::ReleaseIdentity;

#[cfg(test)]
mod tests;
