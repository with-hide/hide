//! The contract every registered host passes, run against GitHub's unit and
//! against a host registered only here that has the base contract alone.
//! Common code is not changed for the test host: it is chosen, read and
//! refused through the same registry and reader as GitHub.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::read::{GithubProjectRequest, GithubRequest, Reads};
use super::*;
use crate::model::{
    GithubReadOutcome, GithubStatusSnapshot, PartRead, PullRequestBadge, PullRequestChecks,
    PullRequestSnapshot, RepositoryIdentity,
};
use hide_node_link::protocol::Call;

/// A host known only to this test binary: one server it names, no host of
/// its own that finds a checkout's remote, and no optional operation.
pub(crate) struct Plain;

pub(crate) static PLAIN: Plain = Plain;

const PLAIN_SERVER: &str = "code.example";

/// GitHub and the test host, as a core that registered both would have them.
pub(crate) const BOTH: CodeHosts = CodeHosts(&[&crate::github::host::GITHUB, &PLAIN]);

const PLAIN_ONLY: CodeHosts = CodeHosts(&[&PLAIN]);

impl CodeHost for Plain {
    fn key(&self) -> &'static str {
        "plain"
    }

    fn names_server(&self, authority: &str) -> bool {
        authority == PLAIN_SERVER
    }

    fn repository(&self, stored: &str) -> Option<RepositoryRef> {
        let path = stored.strip_prefix(PLAIN_SERVER)?.strip_prefix('/')?;
        let (owner, name) = path.split_once('/')?;
        let plain = |part: &str| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        };
        (plain(owner) && plain(name)).then(|| RepositoryRef {
            host: self.key().to_owned(),
            authority: PLAIN_SERVER.to_owned(),
            path: path.to_owned(),
        })
    }

    fn remote_repository(&self, url: &str) -> Option<RepositoryRef> {
        let path = if let Some(rest) = url.strip_prefix("https://") {
            let rest = rest.rsplit_once('@').map_or(rest, |(_, rest)| rest);
            rest.strip_prefix(PLAIN_SERVER)?.strip_prefix('/')?
        } else {
            url.strip_prefix("git@")?
                .strip_prefix(PLAIN_SERVER)?
                .strip_prefix(':')?
        };
        let path = path.strip_suffix(".git").unwrap_or(path);
        self.repository(&format!("{PLAIN_SERVER}/{path}"))
    }

    fn change_request_at(&self, url: &str) -> Option<(RepositoryRef, u32)> {
        let rest = url.strip_prefix("https://")?;
        let (repository, number) = rest.split_once("/-/merge_requests/")?;
        Some((self.repository(repository)?, number.parse().ok()?))
    }

    fn stored_name(&self, repository: &RepositoryRef) -> String {
        format!("{}/{}", repository.authority, repository.path)
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

    fn reads(&self) -> Box<dyn HostReads> {
        Box::new(PlainReads)
    }

    fn read_clock(&self) -> ReadClock {
        ReadClock {
            reread: Duration::from_secs(60),
            retry_first: Duration::from_secs(10),
        }
    }

    fn project_limit(&self) -> usize {
        2
    }

    fn card(&self, _node: &dyn NodeLink, _place: &Place, _number: u32) -> Result<CardRead, String> {
        Err("the test host reads no card".to_owned())
    }
}

/// The test host's read: answers from what each repository's path says it
/// is, and never touches the node.
struct PlainReads;

impl HostReads for PlainReads {
    fn read(&mut self, _node: &dyn NodeLink, due: &[DueRead<'_>]) -> Vec<GithubProjectSnapshot> {
        due.iter()
            .map(|due| {
                let repository = due
                    .place
                    .repository()
                    .expect("the test host reads a repository by its name")
                    .clone();
                let read = || GithubProjectSnapshot {
                    root_path: due.root_path.to_owned(),
                    status: GithubStatusSnapshot {
                        available: true,
                        last_success_at_unix_ms: Some(1),
                        ..GithubStatusSnapshot::default()
                    },
                    pull_requests_read: true,
                    issues_read: PartRead::Unsupported,
                    repository: Some(RepositoryIdentity {
                        repository: repository.clone(),
                        id: None,
                        public: None,
                        default_branch: Some("main".to_owned()),
                    }),
                    ..GithubProjectSnapshot::default()
                };
                match repository.path.as_str() {
                    "acme/app" => GithubProjectSnapshot {
                        pull_requests: vec![listed(&repository, 3)],
                        ..read()
                    },
                    "acme/down" => GithubProjectSnapshot {
                        root_path: due.root_path.to_owned(),
                        status: GithubStatusSnapshot {
                            available: true,
                            stale: true,
                            unavailable_reason: Some("the server did not answer".to_owned()),
                            ..GithubStatusSnapshot::default()
                        },
                        ..GithubProjectSnapshot::default()
                    },
                    _ => read(),
                }
            })
            .collect()
    }

    fn forget(&mut self, _asked: &dyn Fn(&Path) -> bool) {}
}

fn listed(repository: &RepositoryRef, number: u32) -> PullRequestSnapshot {
    PullRequestSnapshot {
        reference: Some(change_request(&PLAIN, repository.clone(), number)),
        closing_issues: Vec::new(),
        title: format!("Change {number}"),
        checks: PullRequestChecks::Passing,
        number,
        head_branch: format!("change-{number}"),
        base_branch: "main".to_owned(),
        url: PLAIN.change_request_url(repository, number, ChangeRequestPage::Overview),
        badge: PullRequestBadge::Open,
        review: None,
        is_draft: false,
        merged_at_unix_ms: None,
        updated_at_unix_ms: Some(1),
        created_at_unix_ms: Some(1),
        closed_at_unix_ms: None,
        head_oid: Some(format!("{number:040x}")),
        cross_repository: false,
        work: Default::default(),
    }
}

/// A node no host may ask: the test host reads by name and the reader
/// answers a checkout with no remote itself; the only call it takes is the
/// reader's own question for a checkout's main worktree.
struct NoHostAsked;

impl NodeLink for NoHostAsked {
    fn call(
        &self,
        call: Call,
        _timeout: Duration,
    ) -> Result<hide_node_link::LinkAnswer, hide_node_link::LinkError> {
        match call {
            Call::Git { root, .. } => Ok(serde_json::Value::String(format!("{root}/.git")).into()),
            other => panic!("no host is asked, yet the node was: {other:?}"),
        }
    }
}

/// Whether two references name the same repository: a path is matched
/// without regard to ASCII case, as the hosts resolve it.
fn same(left: &RepositoryRef, right: &RepositoryRef) -> bool {
    left.host == right.host
        && left.authority == right.authority
        && left.path.eq_ignore_ascii_case(&right.path)
}

fn host(hosts: CodeHosts, key: &str) -> Host {
    hosts.host(key).expect("the host is registered")
}

/// What every host's unit promises, for one stored name and the remotes
/// that name the same repository.
fn holds_the_base_contract(host: Host, stored: &str, remotes: &[&str]) {
    let repository = host
        .repository(stored)
        .unwrap_or_else(|| panic!("{host:?} reads {stored}"));
    assert_eq!(repository.host, host.key(), "{stored}");
    assert_eq!(
        host.stored_name(&repository),
        stored,
        "a stored name is kept as it was"
    );
    for remote in remotes {
        let read = host
            .remote_repository(remote)
            .unwrap_or_else(|| panic!("{host:?} reads {remote}"));
        assert!(
            same(&read, &repository),
            "{remote} names {repository:?}, not {read:?}"
        );
        assert!(
            ![&read.authority, &read.path]
                .iter()
                .any(|part| part.contains("token")),
            "a remote's credentials never reach the reference: {read:?}"
        );
    }

    // Every address is the unit's own, on the repository's server.
    let page = host.repository_url(&repository);
    assert!(
        page.starts_with(&format!("https://{}/", repository.authority)),
        "{page}"
    );
    assert!(page.ends_with(&repository.path), "{page}");
    let pages = [
        ChangeRequestPage::Overview,
        ChangeRequestPage::Checks,
        ChangeRequestPage::Files,
    ]
    .map(|which| host.change_request_url(&repository, 12, which));
    for address in &pages {
        assert!(
            address.starts_with(&page) && address.contains("12"),
            "{address}"
        );
    }
    assert!(
        pages[0] != pages[1] && pages[1] != pages[2] && pages[0] != pages[2],
        "{pages:?}"
    );
    let read = host
        .change_request_at(&pages[0])
        .unwrap_or_else(|| panic!("{host:?} reads {}", pages[0]));
    assert!(
        same(&read.0, &repository) && read.1 == 12,
        "the unit reads the address it made"
    );
    let reference = change_request(host.unit(), repository.clone(), 12);
    assert_eq!(
        (reference.checks_url, reference.files_url, reference.label),
        (pages[1].clone(), pages[2].clone(), host.number_label(12)),
        "a carried reference holds the unit's own pages and label"
    );
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let commit = host.commit_url(&repository, sha);
    assert!(
        commit.starts_with(&page) && commit.ends_with(sha),
        "{commit}"
    );
    assert!(host.number_label(12).contains("12"));
    assert_ne!(host.number_label(12), host.number_label(13));
}

#[test]
fn github_and_a_host_with_the_base_contract_alone_keep_the_same_promises() {
    let github = host(BOTH, "github");
    holds_the_base_contract(
        github,
        "acme/app",
        &[
            "git@github.com:acme/app.git",
            "https://x-access-token:token@github.com/acme/app.git",
            "ssh://git@github.com/acme/app",
        ],
    );
    // An Enterprise server keeps its name, and an HTTPS port, which `gh`
    // needs to reach its API; an SSH port is not the API's.
    holds_the_base_contract(
        github,
        "ghe.example.com/acme/app",
        &[
            "git@ghe.example.com:acme/app.git",
            "ssh://git@ghe.example.com:2222/acme/app.git",
        ],
    );
    holds_the_base_contract(
        github,
        "ghe.example.com:8443/acme/app",
        &["https://ghe.example.com:8443/acme/app.git"],
    );
    holds_the_base_contract(
        host(BOTH, "plain"),
        "code.example/acme/app",
        &[
            "git@code.example:acme/app.git",
            "https://user:token@code.example/acme/app.git",
        ],
    );
}

#[test]
fn github_writes_its_own_addresses_and_number_labels() {
    let github = host(BOTH, "github");
    let repository = github.repository("acme/app").unwrap();
    assert_eq!(
        (
            github.change_request_url(&repository, 7, ChangeRequestPage::Overview),
            github.change_request_url(&repository, 7, ChangeRequestPage::Checks),
            github.change_request_url(&repository, 7, ChangeRequestPage::Files),
            github.commit_url(&repository, "abc"),
            github.number_label(7),
        ),
        (
            "https://github.com/acme/app/pull/7".to_owned(),
            "https://github.com/acme/app/pull/7/checks".to_owned(),
            "https://github.com/acme/app/pull/7/files".to_owned(),
            "https://github.com/acme/app/commit/abc".to_owned(),
            "#7".to_owned(),
        )
    );
    let enterprise = github.repository("GHE.example.com:8443/acme/app").unwrap();
    assert_eq!(
        github.repository_url(&enterprise),
        "https://ghe.example.com:8443/acme/app"
    );
}

#[test]
fn a_stored_name_belongs_to_the_host_that_names_its_server_else_the_first_that_reads_it() {
    let named = |stored: &str| {
        BOTH.repository(stored)
            .map(|(host, repository)| (host.key(), repository.authority))
    };
    assert_eq!(named("acme/app"), Some(("github", "github.com".to_owned())));
    // GitHub reads any server as an Enterprise one, but a server another
    // registered host names is that host's.
    assert_eq!(
        named("code.example/acme/app"),
        Some(("plain", "code.example".to_owned()))
    );
    assert_eq!(
        named("ghe.example.com/acme/app"),
        Some(("github", "ghe.example.com".to_owned()))
    );
    assert_eq!(
        named("github.com:443/acme/app"),
        Some(("github", "github.com".to_owned())),
        "GitHub's own server with its HTTPS port is GitHub's own server"
    );
    assert_eq!(named("not a repository"), None);
    assert_eq!(
        BOTH.change_request_at("https://code.example/acme/app/-/merge_requests/4")
            .map(|(host, repository, number)| (host.key(), repository.path, number)),
        Some(("plain", "acme/app".to_owned(), 4))
    );
    let on_named = |stored: &str| BOTH.on_named_server(&BOTH.repository(stored).unwrap().1);
    assert!(on_named("acme/app") && on_named("code.example/acme/app"));
    assert!(
        !on_named("ghe.example.com/acme/app"),
        "an Enterprise server is not one GitHub is known by"
    );
    assert_eq!(
        BOTH.remote("git@code.example:acme/app.git")
            .map(|(host, _)| host.key()),
        Some("plain")
    );
    assert_eq!(
        BOTH.of(&PLAIN.repository("code.example/acme/app").unwrap()),
        Some(host(BOTH, "plain"))
    );
}

/// The node names each remote's repository as a stored name and never its
/// URL (`PathFacts::remotes`); a remote it could not name is left out.
#[test]
fn a_checkout_is_placed_by_its_remotes_and_none_is_no_remote_with_no_host_asked() {
    let root = Path::new("/work/app");
    let place = |hosts: CodeHosts, remotes: Option<&[&str]>| {
        let remotes: Option<Vec<String>> =
            remotes.map(|remotes| remotes.iter().map(|remote| (*remote).to_owned()).collect());
        hosts.place_of_checkout(root, remotes.as_deref())
    };
    let in_checkout = |hosts: CodeHosts, key: &str| {
        Some(Place::Checkout {
            host: host(hosts, key),
            root: root.to_path_buf(),
        })
    };
    let github = "acme/app";
    let plain = "code.example/acme/app";

    assert_eq!(place(BOTH, Some(&[])), None, "no remote: no host is asked");
    assert_eq!(
        place(BOTH, Some(&["not a repository"])),
        None,
        "a remote no host reads is no remote"
    );
    assert_eq!(place(BOTH, Some(&[github])), in_checkout(BOTH, "github"));
    assert_eq!(
        place(BOTH, Some(&[plain])),
        BOTH.place_of("code.example/acme/app"),
        "a host that does not find a checkout's remote reads it by name"
    );
    assert_eq!(
        place(BOTH, Some(&[plain, github])),
        in_checkout(BOTH, "github"),
        "remotes of two hosts go to the one that picks in the checkout"
    );
    assert_eq!(
        place(BOTH, Some(&[github, "ghe.example.com/acme/app"])),
        in_checkout(BOTH, "github"),
        "github.com and an Enterprise server are one host, which picks in the checkout"
    );
    assert_eq!(
        place(BOTH, None),
        in_checkout(BOTH, "github"),
        "remotes the node did not name are read in the checkout"
    );
    assert_eq!(place(PLAIN_ONLY, None), None);
}

#[test]
fn an_explicit_repository_or_a_devices_origin_is_read_by_its_name() {
    let by_name = |stored: &str| BOTH.place_of(stored);
    let github = by_name("acme/app").unwrap();
    assert_eq!(*github.host(), host(BOTH, "github"));
    assert_eq!(github.stored_name().as_deref(), Some("acme/app"));
    let enterprise = by_name("ghe.example.com:8443/acme/app").unwrap();
    assert_eq!(
        enterprise.stored_name().as_deref(),
        Some("ghe.example.com:8443/acme/app")
    );
    let plain = by_name("code.example/acme/app").unwrap();
    assert_eq!(*plain.host(), host(BOTH, "plain"));
    assert_eq!(
        host(BOTH, "github").place_named("code.example/acme/app"),
        Some(Place::Repository {
            host: host(BOTH, "github"),
            repository: RepositoryRef {
                host: "github".to_owned(),
                authority: "code.example".to_owned(),
                path: "acme/app".to_owned(),
            },
        }),
        "a host asked by name reads the name as its own"
    );
}

#[test]
fn an_operation_a_host_does_not_declare_is_not_there_to_call() {
    let plain = host(BOTH, "plain").unit();
    assert!(plain.merge().is_none());
    assert!(plain.update_branch().is_none());
    assert!(plain.edit_body().is_none());
    assert!(plain.merge_proof().is_none());
    assert!(plain.base_policy().is_none());
    assert!(plain.commit_checks().is_none());
    assert!(plain.failed_log().is_none());
    assert!(plain.by_number().is_none());
    assert!(plain.handover().is_none());
    assert!(plain.issues().is_none());
    assert!(plain.search().is_none());
    assert!(plain.watch().is_none());
    assert_eq!(PLAIN_ONLY.search_query_limit(), None, "no host searches");
    assert!(BOTH.search_query_limit().is_some());
}

/// Each host keeps its own cap and clock: a host's projects past its cap
/// are left out without taking another host's place.
#[test]
fn each_host_caps_and_clocks_its_own_projects() {
    let github = host(BOTH, "github");
    let plain = host(BOTH, "plain");
    let projects: Vec<(&str, Option<Host>)> = vec![
        ("p1", Some(plain)),
        ("g1", Some(github)),
        ("p2", Some(plain)),
        ("p3", Some(plain)),
        ("none", None),
        ("g2", Some(github)),
    ];
    let kept: Vec<&str> = BOTH
        .within_limits(projects, |(_, host)| *host)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(kept, ["p1", "g1", "p2", "none", "g2"]);
    let many = (0..70).map(|_| Some(github));
    assert_eq!(BOTH.within_limits(many, |host| *host).len(), 64);

    let clock = github.read_clock();
    assert_eq!(
        [0, 1, 2, 4, 20].map(|failures| clock.wait(failures).as_secs()),
        [300, 30, 60, 240, 300],
        "GitHub's clock: five minutes, 30 seconds doubling after failures"
    );
    assert_eq!(plain.read_clock().wait(0).as_secs(), 60);
    assert_eq!(
        BOTH.unplaced_clock().reread.as_secs(),
        300,
        "the slowest asks least"
    );
}

#[test]
fn githubs_actions_job_is_read_from_its_check_page_alone() {
    let logs = host(BOTH, "github").unit().failed_log().unwrap();
    assert_eq!(
        logs.job_of("https://github.com/o/r/actions/runs/123/job/456"),
        Some(456)
    );
    assert_eq!(
        logs.job_of("https://github.com/o/r/actions/runs/123/job/456?pr=7"),
        Some(456)
    );
    assert_eq!(
        logs.job_of("https://app.circleci.com/pipelines/gh/o/r/1"),
        None
    );
    assert_eq!(logs.job_of("https://github.com/o/r/actions/runs/123"), None);
}

#[test]
fn githubs_exact_commit_checks_cover_github_com_alone() {
    let github = host(BOTH, "github");
    let checks = github.unit().commit_checks().unwrap();
    assert!(checks.covers(&github.repository("acme/app").unwrap()));
    assert!(!checks.covers(&github.repository("ghe.example.com/acme/app").unwrap()));
}

/// A read's issues part is unsupported, not read, failed, or read (and
/// maybe empty); only a part its host offers and did not read holds the
/// project's clock back.
#[test]
fn the_four_read_outcomes_of_a_part_are_told_apart() {
    let outcome = |issues_read: PartRead| {
        GithubProjectSnapshot {
            pull_requests_read: true,
            issues_read,
            ..GithubProjectSnapshot::default()
        }
        .read_outcome()
    };
    assert_eq!(outcome(PartRead::Unsupported), GithubReadOutcome::Read);
    assert_eq!(outcome(PartRead::Read), GithubReadOutcome::Read);
    assert_eq!(outcome(PartRead::Failed), GithubReadOutcome::IssuesFailed);
    assert_eq!(outcome(PartRead::NotRead), GithubReadOutcome::IssuesFailed);
    assert_eq!(
        GithubProjectSnapshot::default().issues_read,
        PartRead::NotRead
    );
    let saved_before: GithubProjectSnapshot =
        serde_json::from_value(serde_json::json!({"root_path": "/repo"})).unwrap();
    assert_eq!(
        saved_before.issues_read,
        PartRead::Read,
        "an answer saved before the part was kept had its issues read"
    );
    assert_eq!(saved_before.repository, None);
}

/// The common reader hands the test host its projects and keeps its
/// answers; a checkout whose remotes name nothing is answered "no remote"
/// with no host asked.
#[test]
fn the_common_reader_reads_a_host_with_the_base_contract_alone() {
    let project = |root: &str, place: Option<Place>| GithubProjectRequest {
        links: Vec::new(),
        root: PathBuf::from(root),
        place,
        generation: 1,
    };
    let request = GithubRequest {
        projects: vec![
            project("/app", BOTH.place_of("code.example/acme/app")),
            project("/empty", BOTH.place_of("code.example/acme/empty")),
            project("/down", BOTH.place_of("code.example/acme/down")),
            project("/none", None),
        ],
    };
    // One pass of the reader's worker, run here: what it answers is what the
    // background read hands the core.
    let answer = super::read::read(&mut Reads::new(BOTH), &NoHostAsked, &request);
    let read = |root: &str| {
        answer
            .project(root)
            .unwrap_or_else(|| panic!("{root} is answered"))
    };

    let app = read("/app");
    assert_eq!(app.read_outcome(), GithubReadOutcome::Read);
    assert_eq!(
        app.issues_read,
        PartRead::Unsupported,
        "not an empty success"
    );
    assert_eq!(
        app.pull_requests[0].url, "https://code.example/acme/app/-/merge_requests/3",
        "the address is the host's own"
    );
    assert_eq!(app.default_branch(), Some("main"));
    let reference = app.pull_requests[0].reference.as_ref().unwrap();
    assert_eq!(
        (reference.label.as_str(), reference.checks_url.as_str()),
        (
            "!3",
            "https://code.example/acme/app/-/merge_requests/3/pipelines"
        ),
        "the label and the pages are the unit's, carried with the change request"
    );

    let empty = read("/empty");
    assert_eq!(empty.read_outcome(), GithubReadOutcome::Read);
    assert!(empty.pull_requests.is_empty() && empty.status.unavailable_reason.is_none());

    let down = read("/down");
    assert_eq!(down.read_outcome(), GithubReadOutcome::Failed);
    assert_eq!(down.issues_read, PartRead::NotRead);

    assert_eq!(read("/none").read_outcome(), GithubReadOutcome::NoRemote);
}
