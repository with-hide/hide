//! Pull request addresses a session's tools printed (PRD
//! overview-request-view D-31).
//!
//! Only tool output counts: an address in the agent's own reply or in the
//! operator's request is a mention, never a creation. The scan runs on the
//! records a read already decodes and adds no file read, host command or
//! host request (B53). A sighting in the output of a call that makes a pull
//! request is that making (`made`); whether any other one means the session
//! made the pull request is decided later against GitHub's `createdAt`.

use serde::{Deserialize, Serialize};

/// One pull request address seen in a tool's output, and when the tool's
/// output was recorded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrSighting {
    /// The repository's stored name, as the address spells it: `owner/name`
    /// on github.com, `gitlab.com/group/sub/project`.
    pub repository: String,
    pub number: u64,
    pub at_unix_ms: u64,
    /// The output answered a call that makes a pull request, and gave this
    /// address back as the one it made.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub made: bool,
}

/// The most addresses one tool output contributes; a listing of every pull
/// request in a repository is no creation, and must not grow the read.
pub const MAX_SIGHTINGS_PER_OUTPUT: usize = 8;

/// Every distinct change request address of a registered host in `text`
/// (`https://github.com/<owner>/<repo>/pull/<number>`,
/// `https://gitlab.com/<group>/<project>/-/merge_requests/<number>`), each
/// with its repository's stored name, in order, up to
/// [`MAX_SIGHTINGS_PER_OUTPUT`].
pub fn pull_request_addresses(text: &str) -> Vec<(String, u64)> {
    let mut all: Vec<(usize, String, u64)> = crate::hosts::STANDARD
        .iter()
        .flat_map(|host| host.addresses_in(text))
        .collect();
    all.sort_by_key(|(at, _, _)| *at);
    distinct(
        all.into_iter()
            .map(|(_, repository, number)| (repository, number)),
    )
}

/// Every distinct issue address in `text`: issues are read on GitHub only
/// (`crate::hosts::issue_addresses_in`).
pub fn issue_addresses(text: &str) -> Vec<(String, u64)> {
    distinct(
        crate::hosts::issue_addresses_in(text)
            .into_iter()
            .map(|(_, repository, number)| (repository, number)),
    )
}

fn distinct(addresses: impl Iterator<Item = (String, u64)>) -> Vec<(String, u64)> {
    let mut found: Vec<(String, u64)> = Vec::new();
    for (repository, number) in addresses {
        if !found
            .iter()
            .any(|(seen, n)| *n == number && seen.eq_ignore_ascii_case(&repository))
        {
            found.push((repository, number));
            if found.len() == MAX_SIGHTINGS_PER_OUTPUT {
                break;
            }
        }
    }
    found
}

/// Keep `address` unless `found` holds it (a repository's name in either
/// case); false once `found` is full, when the scan stops. A host's scan
/// keeps only its first distinct addresses, so the merge by position over
/// every host's scan is the first ones in the text.
pub(crate) fn keep_distinct(
    found: &mut Vec<(usize, String, u64)>,
    address: (usize, String, u64),
) -> bool {
    if !found
        .iter()
        .any(|(_, seen, n)| *n == address.2 && seen.eq_ignore_ascii_case(&address.1))
    {
        found.push(address);
    }
    found.len() < MAX_SIGHTINGS_PER_OUTPUT
}

pub(crate) fn is_name(part: &str) -> bool {
    !part.is_empty()
        && part.len() <= 100
        && part
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_read_once_each_and_anything_else_is_not_one() {
        let output = "Creating pull request for prd/x into main\n\
                      https://github.com/herdrdev/hide/pull/341\n\
                      see https://github.com/herdrdev/hide/pull/341.\n\
                      https://github.com/other/repo/pull/7#issuecomment-1\n\
                      https://github.com/herdrdev/hide/issues/12\n\
                      https://github.com/herdrdev/hide/pull/\n\
                      https://github.com/a b/c/pull/9";
        assert_eq!(
            pull_request_addresses(output),
            [
                ("herdrdev/hide".to_owned(), 341),
                ("other/repo".to_owned(), 7)
            ]
        );
    }

    #[test]
    fn a_listing_of_many_pull_requests_is_capped() {
        let listing = (1..=40)
            .map(|n| format!("https://github.com/o/r/pull/{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            pull_request_addresses(&listing).len(),
            MAX_SIGHTINGS_PER_OUTPUT
        );
    }

    /// Both hosts' addresses are read in the order they were printed, a
    /// merge request's under its project's stored name with its subgroups;
    /// a merge request's new-page and an issue are not one.
    #[test]
    fn every_hosts_addresses_are_read_in_order() {
        let output = "https://gitlab.com/acme/sub/app/-/merge_requests/4/diffs\n\
                      https://github.com/acme/app/pull/5\n\
                      https://gitlab.com/acme/app/-/issues/6\n\
                      https://gitlab.com/acme/app/-/merge_requests/new?x=1\n\
                      https://notgitlab.com/acme/app/-/merge_requests/7\n\
                      https://gitlab.com/ACME/Sub/App/-/merge_requests/4";
        assert_eq!(
            pull_request_addresses(output),
            [
                ("gitlab.com/acme/sub/app".to_owned(), 4),
                ("acme/app".to_owned(), 5)
            ]
        );
    }
}
