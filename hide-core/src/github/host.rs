//! GitHub's registration unit: how GitHub names a repository and reads a
//! remote, the addresses it makes, and every operation Hide asks of it, each
//! through the operator's own `gh` (the rest of `crate::github`).

use std::path::Path;
use std::sync::Arc;

use hide_node_link::change_request::PrMergeMethod;
use hide_node_link::gh::{JOB_LOG_TAIL_BYTES, repository_of_remote, valid_repository};

use super::Where;
use crate::code_host::{
    BasePolicy, ByNumber, CardRead, ChangeHead, ChangeHeadFailure, ChangeRequestPage, CodeHost,
    CommitChecks, CreateFailure, EditBody, FailedLog, Feedback, Handover, HeadMerge, HostAccess,
    HostFailure, HostReads, Issues, JobLog, MergeProof, MergedHead, Place, ReadClock,
    RepositoryRef, Search, SearchTarget, Standing, UpdateBranch, Watch, WatchAnswer, WatchReader,
    WatchRequest, WriteFailure,
};
use crate::node_access::NodeLink;

/// The key a GitHub reference names its host by.
const KEY: &str = "github";

/// The server GitHub's own repositories are on, which a stored name leaves
/// out (`OWNER/NAME`); any other server is a GitHub Enterprise Server's and
/// is named in front (`HOST/OWNER/NAME`).
pub(super) const SERVER: &str = "github.com";

pub(crate) struct Github;

pub(crate) static GITHUB: Github = Github;

/// Where `gh` finds a place's repository: the checkout it runs in, or the
/// repository's stored name.
pub(super) fn where_of(place: &Place) -> Where {
    match place {
        Place::Repository { repository, .. } => Where::Repository(GITHUB.stored_name(repository)),
        Place::Checkout { root, .. } => Where::Checkout(root.clone()),
    }
}

/// The repository `OWNER/NAME` on `server` (`github.com`, or a GitHub
/// Enterprise Server's host and port) is.
pub(super) fn repository_on(server: &str, name: &str) -> Option<RepositoryRef> {
    if server.eq_ignore_ascii_case(SERVER) {
        GITHUB.repository(name)
    } else {
        GITHUB.repository(&format!("{server}/{name}"))
    }
}

/// The page of a repository on GitHub, as GitHub's unit makes it.
pub(super) fn repository_url(repository: &RepositoryRef) -> String {
    GITHUB.repository_url(repository)
}

/// The repository and number a GitHub pull request address names
/// (`https://SERVER/OWNER/NAME/pull/N`, on any server), and nothing else.
pub(super) fn change_request_of(url: &str) -> Option<(RepositoryRef, u32)> {
    let rest = url.strip_prefix("https://")?;
    let (server, path) = rest.split_once('/')?;
    let (repository, number) = path.split_once("/pull/")?;
    let number = number.parse::<u32>().ok().filter(|number| *number > 0)?;
    Some((repository_on(server, repository)?, number))
}

impl CodeHost for Github {
    fn key(&self) -> &'static str {
        KEY
    }

    fn names_server(&self, authority: &str) -> bool {
        authority == SERVER
    }

    fn repository(&self, stored: &str) -> Option<RepositoryRef> {
        if !valid_repository(stored) {
            return None;
        }
        let parts: Vec<&str> = stored.split('/').collect();
        let (authority, path) = match parts.as_slice() {
            [owner, name] => (SERVER.to_owned(), format!("{owner}/{name}")),
            [server, owner, name] => {
                let server = server.to_ascii_lowercase();
                // `github.com` named in front is GitHub's own server, which
                // `gh` and the stored name write without it.
                let server = if server == "github.com:443" {
                    SERVER.to_owned()
                } else {
                    server
                };
                (server, format!("{owner}/{name}"))
            }
            _ => return None,
        };
        Some(RepositoryRef {
            host: KEY.to_owned(),
            authority,
            path,
        })
    }

    fn remote_repository(&self, url: &str) -> Option<RepositoryRef> {
        self.repository(&repository_of_remote(url)?)
    }

    fn change_request_at(&self, url: &str) -> Option<(RepositoryRef, u32)> {
        change_request_of(url)
    }

    fn stored_name(&self, repository: &RepositoryRef) -> String {
        if repository.authority == SERVER {
            repository.path.clone()
        } else {
            format!("{}/{}", repository.authority, repository.path)
        }
    }

    fn repository_key(&self, repository: &RepositoryRef) -> String {
        format!(
            "{}/{}",
            repository.authority,
            repository.path.to_ascii_lowercase()
        )
    }

    fn resolves_checkout(&self) -> bool {
        true
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
            ChangeRequestPage::Checks => "/checks",
            ChangeRequestPage::Files => "/files",
        };
        format!("{}/pull/{number}{page}", self.repository_url(repository))
    }

    fn commit_url(&self, repository: &RepositoryRef, sha: &str) -> String {
        format!("{}/commit/{sha}", self.repository_url(repository))
    }

    fn number_label(&self, number: u32) -> String {
        format!("#{number}")
    }

    fn access(&self) -> HostAccess {
        HostAccess {
            program: "gh".to_owned(),
            minimum_version: hide_node_link::gh::MINIMUM_VERSION.to_string(),
            sign_in: "gh auth login".to_owned(),
        }
    }

    fn reads(&self) -> Box<dyn HostReads> {
        Box::<super::GithubReads>::default()
    }

    /// Five minutes after an answer, 30 seconds after a failed read.
    fn read_clock(&self) -> ReadClock {
        ReadClock {
            reread: std::time::Duration::from_secs(5 * 60),
            retry_first: std::time::Duration::from_secs(30),
        }
    }

    /// Each project costs `gh` calls every five minutes.
    fn project_limit(&self) -> usize {
        64
    }

    fn card(&self, node: &dyn NodeLink, place: &Place, number: u32) -> Result<CardRead, String> {
        super::card::read(node, &where_of(place), number)
    }

    fn merge(&self) -> Option<&dyn HeadMerge> {
        Some(self)
    }

    fn update_branch(&self) -> Option<&dyn UpdateBranch> {
        Some(self)
    }

    fn edit_body(&self) -> Option<&dyn EditBody> {
        Some(self)
    }

    fn merge_proof(&self) -> Option<&dyn MergeProof> {
        Some(self)
    }

    fn base_policy(&self) -> Option<&dyn BasePolicy> {
        Some(self)
    }

    fn commit_checks(&self) -> Option<&dyn CommitChecks> {
        Some(self)
    }

    fn failed_log(&self) -> Option<&dyn FailedLog> {
        Some(self)
    }

    fn by_number(&self) -> Option<&dyn ByNumber> {
        Some(self)
    }

    fn handover(&self) -> Option<&dyn Handover> {
        Some(self)
    }

    fn issues(&self) -> Option<&dyn Issues> {
        Some(self)
    }

    fn search(&self) -> Option<&dyn Search> {
        Some(self)
    }

    fn watch(&self) -> Option<&dyn Watch> {
        Some(self)
    }
}

impl HeadMerge for Github {
    fn merge(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        number: u32,
        method: Option<PrMergeMethod>,
        head: &str,
    ) -> Result<(), WriteFailure> {
        super::card::run_write(
            node,
            &where_of(place),
            &super::card::Write::Merge {
                number,
                method,
                head: head.to_owned(),
            },
        )
    }
}

impl UpdateBranch for Github {
    fn update_branch(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        number: u32,
        head: &str,
    ) -> Result<(), WriteFailure> {
        super::card::run_write(
            node,
            &where_of(place),
            &super::card::Write::UpdateBranch {
                number,
                head: head.to_owned(),
            },
        )
    }
}

impl EditBody for Github {
    fn write_closing_line(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        number: u32,
        issue: u32,
    ) -> Result<bool, String> {
        super::write_closing_line(node, &where_of(place), number, issue)
    }
}

impl MergeProof for Github {
    fn merge_commit(
        &self,
        node: &dyn NodeLink,
        repository: &RepositoryRef,
        number: u32,
    ) -> Result<Option<String>, String> {
        super::merge_commit(node, &self.stored_name(repository), number)
    }

    fn contains_commit(
        &self,
        node: &dyn NodeLink,
        repository: &RepositoryRef,
        commit: &str,
        head: &str,
    ) -> Result<bool, String> {
        super::contains_commit(node, &self.stored_name(repository), commit, head)
    }

    fn merged_heads(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        branch: &str,
    ) -> Result<Vec<MergedHead>, String> {
        super::merged_pull_request_proofs(node, &where_of(place), branch)
    }
}

impl BasePolicy for Github {
    fn base_push(&self, node: &dyn NodeLink, root: &Path, base: &str) -> Result<bool, String> {
        super::base_push(node, root, base)
    }
}

impl CommitChecks for Github {
    /// The exact-commit read asks github.com alone: its query names no
    /// server, and the commit address it validates is github.com's.
    fn covers(&self, repository: &RepositoryRef) -> bool {
        repository.authority == SERVER
    }

    fn read(
        &self,
        node: &dyn NodeLink,
        repository: &RepositoryRef,
        commits: &[String],
    ) -> Result<Vec<crate::checkout_git::Ci>, String> {
        super::commits::read(node, &self.stored_name(repository), commits)
    }
}

impl FailedLog for Github {
    fn failed_log(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        job: u64,
    ) -> Result<JobLog, HostFailure> {
        let text = super::failed_job_log(node, &where_of(place), job)?;
        Ok(JobLog {
            cut: text.len() >= JOB_LOG_TAIL_BYTES,
            text,
        })
    }

    /// The job of a GitHub Actions check page
    /// (`.../actions/runs/RUN/job/JOB`).
    fn job_of(&self, address: &str) -> Option<u64> {
        let (_, rest) = address.split_once("/actions/runs/")?;
        let (_, job) = rest.split_once("/job/")?;
        job.split(['/', '?', '#']).next()?.parse().ok()
    }
}

impl ByNumber for Github {
    fn standing(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        number: u32,
    ) -> Result<Standing, String> {
        super::pr_standing(node, &where_of(place), number)
    }

    fn head(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        number: u32,
    ) -> Result<ChangeHead, ChangeHeadFailure> {
        super::pr_head(node, &where_of(place), number)
    }
}

impl Handover for Github {
    fn feedback(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        number: u32,
    ) -> Result<Feedback, String> {
        super::pr_feedback(node, &where_of(place), number)
    }
}

impl Issues for Github {
    fn issues_repository(&self, node: &dyn NodeLink, place: &Place) -> Result<String, String> {
        super::issue_repository(node, &where_of(place))
    }

    fn create(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        title: &str,
        body: &str,
    ) -> Result<crate::issues::IssueSnapshot, CreateFailure> {
        super::create_issue(node, &where_of(place), title, body)
    }

    fn close(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        reference: &crate::issues::IssueReference,
    ) -> Result<(), HostFailure> {
        super::close_issue(
            node,
            &where_of(place),
            &reference.repository,
            reference.number,
        )
        .map_err(HostFailure::from)
    }

    fn detail(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        reference: &crate::issues::IssueReference,
        after: Option<&str>,
    ) -> Result<crate::tasks::TaskDetail, String> {
        super::issue_detail(node, &where_of(place), reference, after)
    }

    fn linked(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        reference: &crate::issues::IssueReference,
    ) -> Result<crate::issues::IssueSnapshot, String> {
        super::read_linked_issue(node, &where_of(place), reference)
    }

    fn reconcile(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        repository: &str,
        title: &str,
        since: u64,
        claimed: &[u64],
    ) -> Result<Option<crate::issues::IssueSnapshot>, String> {
        super::reconcile_issue_attempt(node, &where_of(place), repository, title, since, claimed)
    }
}

impl Search for Github {
    fn repository_limit(&self) -> usize {
        super::SEARCH_REPOSITORY_LIMIT
    }

    fn query_limit(&self) -> usize {
        super::SEARCH_QUERY_LIMIT
    }

    fn search(
        &self,
        node: &dyn NodeLink,
        targets: &[SearchTarget],
        query: &str,
    ) -> Result<Vec<crate::model::GithubSearchResult>, String> {
        super::search(node, targets, query)
    }
}

impl Watch for Github {
    fn project_limit(&self) -> usize {
        crate::github_poll::POLL_PROJECT_LIMIT
    }

    fn head_limit(&self) -> usize {
        crate::github_poll::POLL_PENDING_LIMIT
    }

    fn reader(&self, node: Arc<dyn NodeLink>) -> Box<dyn WatchReader> {
        Box::new(crate::github_poll::GithubPollReader::new(node))
    }
}

/// What the poll reads of a watch request: the repositories on github.com,
/// by `owner/name`, which its `gh api` calls address. An Enterprise server's
/// repository is left to the regular read, as it always was: the poll never
/// names a server, so it would ask github.com about another server's
/// repository.
fn poll_request(request: WatchRequest) -> crate::github_poll::PollRequest {
    crate::github_poll::PollRequest {
        projects: request
            .projects
            .into_iter()
            .filter(|watched| watched.repository.authority == SERVER)
            .map(|watched| crate::github_poll::PollProject {
                repository: watched.repository.path.to_ascii_lowercase(),
                path: watched.path,
                in_front: watched.in_front,
                heads: watched.heads,
            })
            .collect(),
    }
}

impl WatchReader for crate::github_poll::GithubPollReader {
    fn read_if_due(&mut self, request: WatchRequest) -> Option<WatchAnswer> {
        crate::github_poll::GithubPollReader::read_if_due(self, poll_request(request)).map(
            |answer| WatchAnswer {
                changed: answer.changed,
                held: answer.held,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The poll's `gh api` calls address github.com, so only github.com's
    /// repositories reach it, background or in front; an Enterprise
    /// server's, with or without a port, is left to the regular read.
    #[test]
    fn only_github_coms_repositories_reach_the_poll() {
        let watched = |path: &str, stored: &str, in_front: bool| crate::code_host::Watched {
            path: path.to_owned(),
            repository: GITHUB.repository(stored).unwrap(),
            in_front,
            heads: Vec::new(),
        };
        let request = poll_request(WatchRequest {
            projects: vec![
                watched("/public", "Acme/App", false),
                watched("/enterprise", "ghe.example.com/acme/app", false),
                watched("/port", "ghe.example.com:8443/acme/app", false),
                watched("/front", "ghe.example.com/acme/app", true),
            ],
        });
        let polled: Vec<(&str, &str)> = request
            .projects
            .iter()
            .map(|project| (project.path.as_str(), project.repository.as_str()))
            .collect();
        assert_eq!(polled, [("/public", "acme/app")]);
    }
}
