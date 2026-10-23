//! Pull request addresses a session's tools printed (PRD
//! overview-request-view D-31).
//!
//! Only tool output counts: an address in the agent's own reply or in the
//! operator's request is a mention, never a creation. The scan runs on the
//! records a read already decodes and adds no file read, `gh` call or
//! GitHub request (B53); whether a sighting means the session made that pull
//! request is decided later against GitHub's `createdAt`.

use serde::{Deserialize, Serialize};

/// One pull request address seen in a tool's output, and when the tool's
/// output was recorded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrSighting {
    /// `owner/name`, as GitHub spells it in the address.
    pub repository: String,
    pub number: u64,
    pub at_unix_ms: u64,
}

/// The most addresses one tool output contributes; a listing of every pull
/// request in a repository is no creation, and must not grow the read.
pub const MAX_SIGHTINGS_PER_OUTPUT: usize = 8;

const HOST: &str = "github.com/";

/// Every distinct `https://github.com/<owner>/<repo>/pull/<number>` in `text`,
/// in order, up to [`MAX_SIGHTINGS_PER_OUTPUT`].
pub fn pull_request_addresses(text: &str) -> Vec<(String, u64)> {
    addresses(text, "pull")
}

pub fn issue_addresses(text: &str) -> Vec<(String, u64)> {
    addresses(text, "issues")
}

fn addresses(text: &str, kind: &str) -> Vec<(String, u64)> {
    let mut found: Vec<(String, u64)> = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(HOST) {
        let after = &rest[at + HOST.len()..];
        rest = after;
        let Some((repository, number)) = parse_address(after, kind) else {
            continue;
        };
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

fn parse_address(after_host: &str, kind: &str) -> Option<(String, u64)> {
    let mut parts = after_host.splitn(4, '/');
    let owner = parts.next().filter(|part| is_name(part))?;
    let name = parts.next().filter(|part| is_name(part))?;
    if parts.next()? != kind {
        return None;
    }
    let digits: String = parts
        .next()?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let number = digits.parse::<u64>().ok().filter(|number| *number > 0)?;
    Some((format!("{owner}/{name}"), number))
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
}
