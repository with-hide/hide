//! gitlab.com's registration unit: how gitlab.com names a project and reads
//! a remote, the addresses it makes, and what Hide asks of it, each through
//! the operator's own `glab` (the rest of `crate::gitlab`).

use hide_node_link::glab::{HOST, MINIMUM_VERSION, SIGN_IN, is_project_path};

use crate::code_host::{
    CardRead, ChangeRequestPage, CodeHost, CommitChecks, HostAccess, HostReads, Place, ReadClock,
    RepositoryRef,
};
use crate::node_access::NodeLink;

/// The key a gitlab.com reference names its host by.
pub(crate) const KEY: &str = "gitlab";

pub(crate) struct Gitlab;

pub(crate) static GITLAB: Gitlab = Gitlab;

/// The path of a gitlab.com address's repository part, with the server
/// checked: `https://gitlab.com[:443]/P`, `git@gitlab.com:P`,
/// `ssh://git@gitlab.com[:port]/P`. An `https` address with a user part,
/// and any address with a password, is none (see below).
fn remote_path(url: &str) -> Option<&str> {
    let url = url.trim();
    if let Some((scheme, rest)) = url.split_once("://") {
        let authority = rest
            .split_once('/')
            .map_or(rest, |(authority, _)| authority);
        // The node strips the user part of an http(s) address (and the core
        // again on entry); one that still has it was never meant to reach
        // here. An SSH address's user names an account, never a secret,
        // unless it carries a password.
        let user = authority.rsplit_once('@').map(|(user, _)| user);
        let refused = match scheme {
            "https" | "http" => user.is_some(),
            _ => user.is_some_and(|user| user.contains(':')),
        };
        if refused {
            return None;
        }
    }
    let (server, path) = if let Some(rest) = url.strip_prefix("https://") {
        let rest = rest.rsplit_once('@').map_or(rest, |(_, rest)| rest);
        let (server, path) = rest.split_once('/')?;
        (server.strip_suffix(":443").unwrap_or(server), path)
    } else if let Some(rest) = url.strip_prefix("ssh://") {
        let rest = rest.rsplit_once('@').map_or(rest, |(_, rest)| rest);
        let (server, path) = rest.split_once('/')?;
        (server.split(':').next()?, path)
    } else {
        let (user_server, path) = url.split_once(':')?;
        let server = user_server
            .rsplit_once('@')
            .map_or(user_server, |(_, server)| server);
        (server, path)
    };
    if !server.eq_ignore_ascii_case(HOST) {
        return None;
    }
    let path = path.trim_end_matches('/');
    Some(path.strip_suffix(".git").unwrap_or(path))
}

impl CodeHost for Gitlab {
    fn key(&self) -> &'static str {
        KEY
    }

    fn name(&self) -> &'static str {
        "GitLab"
    }

    fn names_server(&self, authority: &str) -> bool {
        authority == HOST
    }

    fn repository(&self, stored: &str) -> Option<RepositoryRef> {
        let path = stored.strip_prefix(HOST)?.strip_prefix('/')?;
        is_project_path(path).then(|| RepositoryRef {
            host: KEY.to_owned(),
            authority: HOST.to_owned(),
            path: path.to_owned(),
        })
    }

    fn remote_repository(&self, url: &str) -> Option<RepositoryRef> {
        self.repository(&format!("{HOST}/{}", remote_path(url)?))
    }

    fn change_request_at(&self, url: &str) -> Option<(RepositoryRef, u32)> {
        let rest = url
            .strip_prefix("https://")?
            .strip_prefix(HOST)?
            .strip_prefix('/')?;
        let (path, number) = rest.split_once("/-/merge_requests/")?;
        let number = number.split(['/', '?', '#']).next()?;
        let number = number.parse::<u32>().ok().filter(|number| *number > 0)?;
        Some((self.repository(&format!("{HOST}/{path}"))?, number))
    }

    fn stored_name(&self, repository: &RepositoryRef) -> String {
        format!("{}/{}", repository.authority, repository.path)
    }

    /// gitlab.com finds a group, subgroup and project by its path in any
    /// case and keeps no two paths that differ only in case, so the whole
    /// path is folded, as GitHub's.
    fn repository_key(&self, repository: &RepositoryRef) -> String {
        format!(
            "{}/{}",
            repository.authority,
            repository.path.to_ascii_lowercase()
        )
    }

    fn resolves_checkout(&self) -> bool {
        false
    }

    fn repository_url(&self, repository: &RepositoryRef) -> String {
        format!("https://{}/{}", repository.authority, repository.path)
    }

    fn change_request_url(
        &self,
        repository: &RepositoryRef,
        number: u32,
        page: ChangeRequestPage,
    ) -> String {
        let page = match page {
            ChangeRequestPage::Overview => "",
            ChangeRequestPage::Checks => "/pipelines",
            ChangeRequestPage::Files => "/diffs",
        };
        format!(
            "{}/-/merge_requests/{number}{page}",
            self.repository_url(repository)
        )
    }

    fn commit_url(&self, repository: &RepositoryRef, sha: &str) -> String {
        format!("{}/-/commit/{sha}", self.repository_url(repository))
    }

    fn number_label(&self, number: u32) -> String {
        format!("!{number}")
    }

    fn access(&self) -> HostAccess {
        HostAccess {
            program: "glab".to_owned(),
            minimum_version: MINIMUM_VERSION.to_string(),
            sign_in: SIGN_IN.to_owned(),
        }
    }

    fn reads(&self) -> Box<dyn HostReads> {
        Box::<super::GitlabReads>::default()
    }

    /// Five minutes after an answer, 30 seconds after a failed read, as
    /// GitHub's.
    fn read_clock(&self) -> ReadClock {
        ReadClock {
            reread: std::time::Duration::from_secs(5 * 60),
            retry_first: std::time::Duration::from_secs(30),
        }
    }

    /// The most projects one pass reads: a cap on one pass, not on the
    /// account. The account's requests are capped by `super::budget`, which
    /// every `glab` request takes from whatever pass or card makes it.
    fn project_limit(&self) -> usize {
        32
    }

    fn card(&self, node: &dyn NodeLink, place: &Place, number: u32) -> Result<CardRead, String> {
        let repository = super::repository_of(place)
            .ok_or_else(|| "the checkout names no gitlab.com project".to_owned())?;
        super::read_card(node, repository, number)
    }

    fn commit_checks(&self) -> Option<&dyn CommitChecks> {
        Some(self)
    }
}

impl CommitChecks for Gitlab {
    fn covers(&self, repository: &RepositoryRef) -> bool {
        repository.host == KEY && repository.authority == HOST
    }

    fn read(
        &self,
        node: &dyn NodeLink,
        repository: &RepositoryRef,
        commits: &[String],
    ) -> Result<Vec<crate::checkout_git::Ci>, String> {
        super::read_commit_checks(node, repository, commits)
    }
}
