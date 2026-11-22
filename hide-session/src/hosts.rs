//! What each code host's commands and addresses look like in a session's
//! record: the session half of a host's registration unit.
//!
//! Session files are read on the node that holds them, which does not link
//! the core, so the part of a host's unit that reads a recorded call or a
//! printed address lives here, one [`SessionHost`] per host, registered once
//! in [`STANDARD`]. The common reading (`crate::named_prs`, the sightings)
//! walks a call's commands and a tool's output and asks each host; it never
//! spells a host's program or address itself. The core's own unit of each
//! host (`hide_core::code_host`) has the same key, and a core test holds the
//! two lists to the same keys and the same host for references from before
//! hosts were named.
//!
//! A host names a repository by its stored name, the one its core unit reads
//! back (`owner/name` on github.com, `gitlab.com/group/sub/project`), so a
//! reference the core receives is resolved by the registry alone. What a
//! host cannot resolve to a repository of its own (a numeric project id,
//! another server) names nothing: it is not guessed to be the current one.

use crate::named_prs::{Found, Place};

mod github;
mod gitlab;

/// One code host's reading of a session's calls and output.
pub(crate) trait SessionHost: Sync {
    /// The key of the host's core unit (`hide_core::code_host::CodeHost::key`).
    fn key(&self) -> &'static str;

    /// The program name of the host's command line tool.
    fn program(&self) -> &'static str;

    /// Whether `server` (lower case) is one the host is known by name, as
    /// its core unit's `names_server` answers: a host that also reads
    /// servers it does not name (GitHub Enterprise) never takes one of
    /// these.
    fn names_server(&self, server: &str) -> bool;

    /// Read one call of the host's program: `args` are the words after the
    /// program, `assignments` the `NAME=value` words before it.
    fn command(&self, args: &[String], assignments: &[&str], place: &Place, found: &mut Found);

    /// The repository's stored name and the number of a word that is one of
    /// the host's change request addresses and nothing else (a trailing page,
    /// query or fragment is part of the address).
    fn address(&self, word: &str) -> Option<(String, u64)>;

    /// Every change request address of the host in `text`, each with where
    /// it starts, in order.
    fn addresses_in(&self, text: &str) -> Vec<(usize, String, u64)>;

    /// The change request a tool's own arguments name (a host tool given a
    /// repository and number), beside any argument that is an address.
    fn tool_arguments(
        &self,
        object: &serde_json::Map<String, serde_json::Value>,
        found: &mut Found,
    );

    /// Whether a tool named `name` (lower case) makes a change request.
    fn creating_tool(&self, name: &str) -> bool;

    /// The field of the host's answer to a create request that holds the
    /// address of the change request it made.
    fn made_field(&self) -> &'static str;

    /// Whether the value of a `git push -o` makes a change request.
    fn creating_push_option(&self, option: &str) -> bool;
}

/// The registered hosts.
pub(crate) static STANDARD: [&dyn SessionHost; 2] = [&github::GITHUB, &gitlab::GITLAB];

/// The host every reference from before hosts were named belongs to: a
/// `NamedPr` with no `host`, an older node's payload, a record row with no
/// host. The core's registry names the same one
/// (`hide_core::code_host::CodeHosts::BEFORE_HOSTS`), and a core test holds
/// the two answers equal.
static BEFORE_HOSTS: &dyn SessionHost = &github::GITHUB;

/// The keys of the registered hosts, for the core's test that holds this
/// list to its own registry.
pub fn keys() -> Vec<&'static str> {
    STANDARD.iter().map(|host| host.key()).collect()
}

/// The key of the host a reference from before hosts were named belongs
/// to ([`BEFORE_HOSTS`]).
pub fn before_hosts_key() -> &'static str {
    BEFORE_HOSTS.key()
}

/// The key, repository stored name and number of a word that is a change
/// request address of a registered host.
pub fn change_request_address(word: &str) -> Option<(&'static str, String, u64)> {
    let word = word.trim();
    if word.is_empty() || word.chars().any(char::is_whitespace) {
        return None;
    }
    STANDARD.iter().find_map(|host| {
        host.address(word)
            .map(|(repository, number)| (host.key(), repository, number))
    })
}

/// The key a reference carries on the wire and in the record: none for the
/// host of references from before hosts ([`BEFORE_HOSTS`]), so its records
/// and an older node's read as they did.
pub(crate) fn recorded_key(key: &'static str) -> Option<String> {
    (key != BEFORE_HOSTS.key()).then(|| key.to_owned())
}

/// Whether a registered host other than `key`'s names `server`.
pub(crate) fn named_by_another(key: &str, server: &str) -> bool {
    let server = server.to_ascii_lowercase();
    STANDARD
        .iter()
        .any(|host| host.key() != key && host.names_server(&server))
}

/// The host whose program a command word names.
pub(crate) fn of_program(program: &str) -> Option<&'static dyn SessionHost> {
    STANDARD
        .iter()
        .copied()
        .find(|host| host.program() == program)
}

/// The value of `NAME=value` among a command's assignments.
pub(crate) fn assigned<'a>(assignments: &[&'a str], name: &str) -> Option<&'a str> {
    assignments.iter().rev().find_map(|word| {
        word.strip_prefix(name)
            .and_then(|rest| rest.strip_prefix('='))
    })
}

/// The GitHub issue addresses in `text`, each with where it starts.
pub(crate) fn issue_addresses_in(text: &str) -> Vec<(usize, String, u64)> {
    github::issue_addresses_in(text)
}

/// Whether the address that reaches `text[start..]` (where its server
/// starts) carries a user part: an `@` anywhere in its word before the
/// server, as in `https://user:secret@server/...`. Such an address is no
/// reference on any host, so a credential written into one is never read,
/// kept or logged. The word starts after whitespace or a quote or bracket.
pub(crate) fn carries_user(text: &str, start: usize) -> bool {
    text[..start]
        .bytes()
        .rev()
        .take_while(|byte| {
            !byte.is_ascii_whitespace()
                && !matches!(
                    byte,
                    b'"' | b'\'' | b'`' | b'<' | b'>' | b'(' | b')' | b'[' | b']' | b'{' | b'}'
                )
        })
        .any(|byte| byte == b'@')
}

/// A path part of a repository: what a host lets a name be.
pub(crate) fn is_name(part: &str) -> bool {
    crate::sightings::is_name(part)
}
