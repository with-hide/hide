//! How the core reads a node's files (PRD S5.5 D-05, core-host-node D-21).
//!
//! Every document open, save and revision read, on this machine or on an SSH
//! device, is one request to that node's [`NodeLink`]: the core's own node
//! answers in this process, and a device's node at the other end of an SSH
//! channel. The callers see one interface and one failure vocabulary, so a
//! local path and a remote one cannot drift apart in what they refuse. What
//! the core adds here is its own distrust of an answer: names and paths that
//! leave the root, and entries past a cap, never reach the page.

use std::time::Duration;

use hide_node_link::ErrorCode;
use hide_node_link::list::Listing;
use hide_node_link::protocol::{Call, RootOpened, RootRef};
pub use hide_node_link::{LinkAnswer, LinkError, NodeLink, call_as, call_as_with_progress};

const LIST_TIMEOUT: Duration = Duration::from_secs(30);

/// Why a call to a checkout's host failed, as the code a screen words:
/// `host.<code>` for the host's refusal, `link.not_connected`, `link.busy`
/// or `link.unknown` for the link. The host's own sentence is the log's.
pub fn link_failure_code(error: &LinkError) -> String {
    match error {
        LinkError::Refused(error) => format!("host.{}", error.code.as_str()),
        LinkError::NotConnected(_) => "link.not_connected".to_owned(),
        LinkError::Busy => "link.busy".to_owned(),
        LinkError::Unknown(_) => "link.unknown".to_owned(),
    }
}

/// Whether `path` is a directory on `node`. A node that cannot answer says
/// no, so the caller falls back the way it does for a folder that is gone.
pub fn is_directory(node: &dyn NodeLink, path: &str) -> bool {
    call_as::<Option<String>>(
        node,
        hide_node_link::protocol::Call::Directory {
            path: path.to_owned(),
        },
        std::time::Duration::from_secs(10),
    )
    .is_ok_and(|real| real.is_some())
}

/// `root` as the channel's requests name it: its pinned identity, or the one
/// a fresh `root_open` reports, which is then pinned.
pub fn pinned_root(
    channel: &(impl NodeLink + ?Sized),
    root: &str,
    timeout: Duration,
) -> Result<RootRef, LinkError> {
    let identity = match channel.pinned(root) {
        Some(identity) => identity,
        None => {
            let opened: RootOpened = call_as(
                channel,
                Call::RootOpen {
                    root: root.to_owned(),
                },
                timeout,
            )?;
            channel.pin(root, Some(opened.identity));
            opened.identity
        }
    };
    Ok(RootRef {
        path: root.to_owned(),
        identity,
    })
}

/// One folder of a checkout, for the Explorer. A root that was replaced is
/// refused and unpinned, so the operator's next explicit read adopts the
/// folder now at that path.
pub fn list_folder(
    channel: &(impl NodeLink + ?Sized),
    root: &str,
    relative: &str,
) -> Result<Listing, LinkError> {
    let root_ref = pinned_root(channel, root, LIST_TIMEOUT)?;
    let result = call_as(
        channel,
        Call::List {
            root: root_ref,
            path: relative.to_owned(),
        },
        LIST_TIMEOUT,
    );
    if let Err(LinkError::Refused(error)) = &result
        && error.code == ErrorCode::RootReplaced
    {
        channel.pin(root, None);
    }
    result.map(|mut listing: Listing| {
        // A device's answer is untrusted input: a name that is not one plain
        // path component, or entries past the cap, never reach the page.
        let answered = listing.entries.len();
        listing
            .entries
            .retain(|entry| hide_node_link::mutate::valid_name(&entry.name).is_ok());
        if listing.entries.len() > hide_node_link::list::LIST_CAP {
            listing.entries.truncate(hide_node_link::list::LIST_CAP);
            listing.truncated = true;
        }
        if listing.entries.len() < answered.min(hide_node_link::list::LIST_CAP) {
            crate::diagnostic!(serde_json::json!({
                "component": "node_access", "kind": "host.listing_names_refused",
                "root": root, "refused": answered - listing.entries.len(),
            }));
        }
        listing
    })
}

/// One range of a device file's bytes (`hide_host::bytes::read`).
pub fn read_bytes(
    channel: &(impl NodeLink + ?Sized),
    root: &str,
    relative: &str,
    offset: u64,
    length: u64,
) -> Result<hide_node_link::bytes::Range, LinkError> {
    let root_ref = pinned_root(channel, root, LIST_TIMEOUT)?;
    let result = call_as(
        channel,
        Call::Bytes {
            root: root_ref,
            path: relative.to_owned(),
            offset,
            length,
        },
        LIST_TIMEOUT,
    );
    if let Err(LinkError::Refused(error)) = &result
        && error.code == ErrorCode::RootReplaced
    {
        channel.pin(root, None);
    }
    result
}

/// A whole-root walk can take a while on a wide checkout far away.
const INDEX_TIMEOUT: Duration = Duration::from_secs(120);

/// Every file of a pinned root the ignore files admit, capped, from the host
/// that holds it (`hide_host::index::walk`).
pub fn index_root(
    channel: &(impl NodeLink + ?Sized),
    root: &str,
) -> Result<hide_node_link::index::Walked, LinkError> {
    let root_ref = pinned_root(channel, root, LIST_TIMEOUT)?;
    let result = call_as(channel, Call::Index { root: root_ref }, INDEX_TIMEOUT);
    if let Err(LinkError::Refused(error)) = &result
        && error.code == ErrorCode::RootReplaced
    {
        channel.pin(root, None);
    }
    result.map(|mut walked: hide_node_link::index::Walked| {
        // As for a listing: only relative paths inside the root, in the
        // wire's spelling whatever system the device runs, and no more of
        // them than the walk cap.
        walked
            .paths
            .retain(|path| hide_platform::path::RelPath::parse(path).is_ok());
        if walked.paths.len() > hide_node_link::index::INDEX_CAP {
            walked.paths.truncate(hide_node_link::index::INDEX_CAP);
            walked.truncated = true;
        }
        walked
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hide_node_link::RootIdentity;

    /// A device helper that answers whatever it likes.
    struct Hostile;

    impl NodeLink for Hostile {
        fn call(&self, call: Call, _timeout: Duration) -> Result<LinkAnswer, LinkError> {
            let entry =
                |name: &str| serde_json::json!({"name": name, "is_directory": false, "inode": 1});
            Ok(match call {
                Call::List { .. } => {
                    let mut entries = vec![
                        entry("ok.txt"),
                        entry("../../etc"),
                        entry("a/b"),
                        entry(".."),
                    ];
                    entries.extend((0..600).map(|n| entry(&format!("f{n}"))));
                    serde_json::json!({"entries": entries, "truncated": false})
                }
                Call::Index { .. } => {
                    let mut paths = vec![
                        "src/a.rs".to_owned(),
                        "../outside".to_owned(),
                        "/etc/passwd".to_owned(),
                    ];
                    paths
                        .extend((0..hide_node_link::index::INDEX_CAP + 5).map(|n| format!("f{n}")));
                    serde_json::json!({"paths": paths, "truncated": false})
                }
                _ => serde_json::json!(null),
            }
            .into())
        }

        fn pinned(&self, _root: &str) -> Option<RootIdentity> {
            Some(RootIdentity {
                device: 1,
                inode: 1,
            })
        }
    }

    /// A device's listing and walk are untrusted input: names that are not
    /// one plain component and paths that leave the root are dropped, and
    /// neither passes its cap (S5.5 B6, B7).
    #[test]
    fn a_hostile_listing_or_walk_is_confined_and_capped() {
        let listing = list_folder(&Hostile, "/r", "").unwrap();
        assert_eq!(listing.entries.len(), hide_node_link::list::LIST_CAP);
        assert!(listing.truncated);
        assert_eq!(listing.entries[0].name, "ok.txt");
        assert!(
            listing
                .entries
                .iter()
                .all(|entry| !entry.name.contains('/') && entry.name != "..")
        );

        let walked = index_root(&Hostile, "/r").unwrap();
        assert_eq!(walked.paths.len(), hide_node_link::index::INDEX_CAP);
        assert!(walked.truncated);
        assert_eq!(walked.paths[0], "src/a.rs");
        assert!(
            walked
                .paths
                .iter()
                .all(|path| !path.starts_with("..") && !path.starts_with('/'))
        );
    }
}
