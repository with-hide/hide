//! The code host contract: what Hide asks of the service a project's
//! repository lives on, the registration unit each such service is, and the
//! one place a project's host is chosen.
//!
//! A host is one registration unit ([`CodeHost`]): how it names a repository
//! and reads a remote's address, the addresses it makes, its read of a
//! project's change requests, issues and identity ([`HostReads`]), and each
//! optional operation it offers, one by one. Common code asks the registry
//! ([`CodeHosts`]) and the unit, receives a [`Place`] and a [`HostFailure`],
//! and never branches on a host's key or spells a host's command or address;
//! `scripts/check-code-host-seam.py` holds that line. GitHub
//! (`crate::github::host`) and gitlab.com (`crate::gitlab::host`) are the
//! registered hosts.
//!
//! An optional operation a host does not declare cannot be called: its
//! accessor answers `None`, and the caller refuses the request as
//! unsupported rather than reading the absence as an empty success.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{GithubFailureCategory, GithubProjectSnapshot};
use crate::node_access::NodeLink;

pub(crate) mod read;

/// A repository on a code host, as Hide names it in memory: the key of the
/// host that serves it, the server it is on (its host name, lowercase, with a
/// port when the address names one), and its path on that server (`owner/name`
/// on GitHub). Only its host's unit makes one, so the three always agree.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Deserialize, Serialize)]
pub struct RepositoryRef {
    pub host: String,
    pub authority: String,
    pub path: String,
}

/// A change request as its host names it, made by the host's unit when it
/// read the change request: common code and the screen compare and open it
/// by these, and never read its address again to find them.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Deserialize, Serialize)]
pub struct ChangeRequestRef {
    pub repository: RepositoryRef,
    pub number: u32,
    /// The name every reference to this change request shares, folded the
    /// way its host folds names ([`CodeHost::repository_key`]): two
    /// references name the same change request exactly when their keys are
    /// equal.
    ///
    /// This and the fields below are what the unit makes from the repository
    /// and number; one saved before the unit made one of them reads it empty,
    /// and restore has the unit make them again.
    #[serde(default)]
    pub key: String,
    /// How the host writes its number (`#12`).
    #[serde(default)]
    pub label: String,
    /// Its own page.
    #[serde(default)]
    pub url: String,
    /// Its checks page.
    #[serde(default)]
    pub checks_url: String,
    /// Its changed files page.
    #[serde(default)]
    pub files_url: String,
}

/// Change request `number` of `repository`, with the label and the
/// addresses `unit`, the repository's host, makes for it.
pub(crate) fn change_request(
    unit: &dyn CodeHost,
    repository: RepositoryRef,
    number: u32,
) -> ChangeRequestRef {
    ChangeRequestRef {
        key: format!("{}#{number}", unit.repository_key(&repository)),
        label: unit.number_label(number),
        url: unit.change_request_url(&repository, number, ChangeRequestPage::Overview),
        checks_url: unit.change_request_url(&repository, number, ChangeRequestPage::Checks),
        files_url: unit.change_request_url(&repository, number, ChangeRequestPage::Files),
        repository,
        number,
    }
}

/// A change request as a person or an agent writes it on a command line
/// (`hide links pr`, a `--pr` option): its number, written bare, with a
/// registered host's label (`#12`, `!12`) or as a registered host's address.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WrittenChangeRequest {
    pub number: u64,
    /// The key of the host its label or address is of; `None` for a bare
    /// number, which any host's project may hold.
    pub host: Option<String>,
    /// The record name of the repository its address names
    /// ([`CodeHosts::record_name`]); `None` without an address.
    pub repository: Option<String>,
}

/// Read `word` as [`WrittenChangeRequest`], or `None` for anything else.
pub fn written_change_request(word: &str) -> Option<WrittenChangeRequest> {
    let hosts = CodeHosts::STANDARD;
    let word = word.trim();
    let positive = |digits: &str| {
        (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| digits.parse::<u32>().ok())
            .flatten()
            .filter(|number| *number > 0)
    };
    if let Some(number) = positive(word) {
        return Some(WrittenChangeRequest {
            number: u64::from(number),
            host: None,
            repository: None,
        });
    }
    let digits = word.trim_start_matches(|character: char| !character.is_ascii_digit());
    if let Some(number) = positive(digits)
        && let Some(host) = hosts.hosts().find(|host| host.number_label(number) == word)
    {
        return Some(WrittenChangeRequest {
            number: u64::from(number),
            host: Some(host.key().to_owned()),
            repository: None,
        });
    }
    let (host, repository, number) = hosts.change_request_at(word)?;
    Some(WrittenChangeRequest {
        number: u64::from(number),
        host: Some(host.key().to_owned()),
        repository: hosts.record_name(&repository),
    })
}

/// What a host tells the screen and the phone about itself, beside a
/// project's read and a change request's card: its key, the optional
/// operations it offers, and what to do when it cannot be read. Made from
/// the unit alone ([`declaration`]), so a button the screen draws and an
/// event the core takes ask the same question.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Deserialize, Serialize)]
pub struct HostDeclaration {
    /// The unit's key ([`CodeHost::key`]): `github`, `gitlab`.
    pub key: String,
    /// The optional operations the host offers; one not named is refused by
    /// the core.
    pub operations: Vec<Operation>,
    pub access: HostAccess,
}

/// The diagnostic component of a read's line that names no one host's
/// project: a pass's projects no host places, a count over every host's
/// projects, an address a session printed. A line about one placed
/// project names its host's key instead.
pub(crate) const COMPONENT: &str = "code_host";

/// One optional operation of the contract, named as the accessor that
/// offers it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Merge,
    UpdateBranch,
    EditBody,
    MergeProof,
    BasePolicy,
    CommitChecks,
    FailedLog,
    ByNumber,
    Handover,
    Issues,
    Search,
    Watch,
}

/// How a host is reached, for what the screen says when it cannot be: the
/// program it runs, the oldest release of it Hide reads with, and the
/// command that signs it in. Words around them are the screen's.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Deserialize, Serialize)]
pub struct HostAccess {
    pub program: String,
    pub minimum_version: String,
    pub sign_in: String,
}

/// The declaration of `unit`, its optional operations read from its own
/// accessors: nothing lists a host's operations a second time.
pub(crate) fn declaration(unit: &dyn CodeHost) -> HostDeclaration {
    let offered = [
        (Operation::Merge, unit.merge().is_some()),
        (Operation::UpdateBranch, unit.update_branch().is_some()),
        (Operation::EditBody, unit.edit_body().is_some()),
        (Operation::MergeProof, unit.merge_proof().is_some()),
        (Operation::BasePolicy, unit.base_policy().is_some()),
        (Operation::CommitChecks, unit.commit_checks().is_some()),
        (Operation::FailedLog, unit.failed_log().is_some()),
        (Operation::ByNumber, unit.by_number().is_some()),
        (Operation::Handover, unit.handover().is_some()),
        (Operation::Issues, unit.issues().is_some()),
        (Operation::Search, unit.search().is_some()),
        (Operation::Watch, unit.watch().is_some()),
    ];
    HostDeclaration {
        key: unit.key().to_owned(),
        operations: offered
            .into_iter()
            .filter_map(|(operation, offered)| offered.then_some(operation))
            .collect(),
        access: unit.access(),
    }
}

/// A registered host's unit, compared and shown by its key.
#[derive(Clone, Copy)]
pub(crate) struct Host(&'static dyn CodeHost);

impl Host {
    pub(crate) fn unit(self) -> &'static dyn CodeHost {
        self.0
    }

    /// Where this host reads the repository a stored name of its own names.
    pub(crate) fn place_named(self, stored: &str) -> Option<Place> {
        Some(Place::Repository {
            host: self,
            repository: self.repository(stored)?,
        })
    }
}

impl std::ops::Deref for Host {
    type Target = dyn CodeHost;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl PartialEq for Host {
    fn eq(&self, other: &Self) -> bool {
        self.0.key() == other.0.key()
    }
}

impl Eq for Host {}

impl std::hash::Hash for Host {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.key().hash(state);
    }
}

impl std::fmt::Debug for Host {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0.key())
    }
}

/// Which page of a change request an address opens.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ChangeRequestPage {
    /// The change request itself.
    Overview,
    /// Its checks.
    Checks,
    /// Its changed files.
    Files,
}

/// Why a host could not answer, in the words common code acts on: a
/// category the screen words and the host's own text for the diagnostic log.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HostFailure {
    pub(crate) category: GithubFailureCategory,
    pub(crate) reason: String,
}

/// Where a host reads one project's repository.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Place {
    /// By the repository's name: a repository the operator connected the
    /// project to, a device's checkout read by the name its `origin` gives,
    /// or a checkout whose host does not find its remote itself.
    Repository {
        host: Host,
        repository: RepositoryRef,
    },
    /// A checkout on the core's machine, whose remote its host resolves
    /// itself ([`CodeHost::resolves_checkout`]), as `gh` does in a folder.
    /// `remotes` are the repositories its remotes name that the registry
    /// gives this host, as data for the host's unit; `remotes_complete` says
    /// whether every remote of the checkout is known, read by this host or by
    /// another registered unit (whose remotes are left out). It is false when
    /// the node did not list them, or listed one no registered unit reads.
    Checkout {
        host: Host,
        root: PathBuf,
        remotes: Vec<RepositoryRef>,
        remotes_complete: bool,
    },
}

impl Place {
    pub(crate) fn host(&self) -> &Host {
        match self {
            Self::Repository { host, .. } | Self::Checkout { host, .. } => host,
        }
    }

    /// The repository it names, when it names one rather than a checkout.
    pub(crate) fn repository(&self) -> Option<&RepositoryRef> {
        match self {
            Self::Repository { repository, .. } => Some(repository),
            Self::Checkout { .. } => None,
        }
    }

    /// The stored name of the repository it names, when it names one.
    pub(crate) fn stored_name(&self) -> Option<String> {
        self.repository()
            .map(|repository| self.host().stored_name(repository))
    }
}

/// One code host's registration unit. The base contract is every method
/// without a default; each optional operation is an accessor that answers
/// `None` unless the host offers it.
pub(crate) trait CodeHost: Sync {
    /// The unit's key, which a [`RepositoryRef`] names its host by.
    fn key(&self) -> &'static str;

    /// Whether `authority` is a server this host is known by name
    /// (`github.com`). A host can also read servers it does not name (GitHub
    /// Enterprise has any name); a server a registered host names is that
    /// host's, never such a host's ([`CodeHosts::repository`]).
    fn names_server(&self, authority: &str) -> bool;

    /// The repository a stored name of this host names (`OWNER/NAME` or
    /// `HOST/OWNER/NAME` on GitHub), or `None` for a name it does not read.
    fn repository(&self, stored: &str) -> Option<RepositoryRef>;

    /// The repository a remote's address names (its SSH or HTTPS form), or
    /// `None` for an address it does not read. A URL's credentials never
    /// reach the reference.
    fn remote_repository(&self, url: &str) -> Option<RepositoryRef>;

    /// The change request an address of this host names, with its number,
    /// or `None` for any other address: only for an address Hide holds
    /// without the reference its unit made (a saved record, an answer from
    /// before references were carried).
    fn change_request_at(&self, url: &str) -> Option<(RepositoryRef, u32)>;

    /// The name a reference of this host is stored and shown by: the inverse
    /// of [`CodeHost::repository`].
    fn stored_name(&self, repository: &RepositoryRef) -> String;

    /// The name every reference to `repository` shares, folded the way the
    /// host folds names (GitHub reads a repository's path in any case), so
    /// common code and the screen compare references by equality alone.
    fn repository_key(&self, repository: &RepositoryRef) -> String;

    /// Whether the host finds a checkout's repository itself from the
    /// checkout's remotes, so a checkout whose remotes Hide does not know is
    /// read in place ([`Place::Checkout`]).
    fn resolves_checkout(&self) -> bool;

    /// The repository's own page.
    fn repository_url(&self, repository: &RepositoryRef) -> String;

    /// A page of change request `number`.
    fn change_request_url(
        &self,
        repository: &RepositoryRef,
        number: u32,
        page: ChangeRequestPage,
    ) -> String;

    /// A commit's page.
    fn commit_url(&self, repository: &RepositoryRef, sha: &str) -> String;

    /// How the host writes a change request's number (`#12`).
    fn number_label(&self, number: u32) -> String;

    /// How the host is reached: its program, the oldest release Hide reads
    /// with, and the command that signs it in.
    fn access(&self) -> HostAccess;

    /// The host's read of its projects, made once for each reader that
    /// reads them; what it keeps between passes is its own.
    fn reads(&self) -> Box<dyn HostReads>;

    /// How often the core reads one of this host's projects again.
    fn read_clock(&self) -> ReadClock;

    /// The most of this host's projects the core reads on one machine; past
    /// it the shortfall is logged, and the one in front is read first.
    fn project_limit(&self) -> usize;

    /// The parts of change request `number`'s card. `Err` when the host
    /// could not be asked at all, which fails every part.
    fn card(&self, node: &dyn NodeLink, place: &Place, number: u32) -> Result<CardRead, String>;

    /// A merge conditioned on the head commit the operator saw.
    fn merge(&self) -> Option<&dyn HeadMerge> {
        None
    }

    /// Bringing a change request's branch up to date with its base.
    fn update_branch(&self) -> Option<&dyn UpdateBranch> {
        None
    }

    /// Writing a closing line into a change request's description.
    fn edit_body(&self) -> Option<&dyn EditBody> {
        None
    }

    /// Proof that a change request merged, and where its commits went.
    fn merge_proof(&self) -> Option<&dyn MergeProof> {
        None
    }

    /// The repository's rule for pushing straight to a base branch.
    fn base_policy(&self) -> Option<&dyn BasePolicy> {
        None
    }

    /// The checks of exact commits.
    fn commit_checks(&self) -> Option<&dyn CommitChecks> {
        None
    }

    /// A failed check's log, cut to its end.
    fn failed_log(&self) -> Option<&dyn FailedLog> {
        None
    }

    /// One change request read by its number, outside the list.
    fn by_number(&self) -> Option<&dyn ByNumber> {
        None
    }

    /// What to hand an agent about a change request: its failed checks and
    /// the change requests still standing.
    fn handover(&self) -> Option<&dyn Handover> {
        None
    }

    /// The repository's issues.
    fn issues(&self) -> Option<&dyn Issues> {
        None
    }

    /// A search of change requests and issues.
    fn search(&self) -> Option<&dyn Search> {
        None
    }

    /// A cheap watch for changes between reads.
    fn watch(&self) -> Option<&dyn Watch> {
        None
    }
}

/// How often a host's project is read again: `reread` after an answer, and
/// `retry_first` after a read that failed, doubling for each further failure
/// in a row and stopping at `reread`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReadClock {
    pub(crate) reread: std::time::Duration,
    pub(crate) retry_first: std::time::Duration,
}

impl ReadClock {
    /// The wait after an answer that came after `failures` failed reads in a
    /// row.
    pub(crate) fn wait(self, failures: u32) -> std::time::Duration {
        match failures {
            0 => self.reread,
            failures => self
                .retry_first
                .saturating_mul(1u32.checked_shl(failures - 1).unwrap_or(u32::MAX))
                .min(self.reread),
        }
    }
}

/// The registered hosts, in the order a project's remotes are matched
/// against them. Production code has one registry ([`CodeHosts::STANDARD`]);
/// a test registers its own units the same way.
#[derive(Clone, Copy)]
pub(crate) struct CodeHosts {
    units: &'static [&'static dyn CodeHost],
    /// The host every reference saved or sent before hosts were named
    /// belongs to: a link record row, a session's named change request or an
    /// older node's payload that names no host. Said here once, by name,
    /// and never read from the order of `units`; `hide_session::hosts` names
    /// the same host for what a session's reader sends, and a test holds the
    /// two equal.
    before_hosts: &'static dyn CodeHost,
}

impl CodeHosts {
    pub(crate) const STANDARD: Self = Self {
        units: &[&crate::github::host::GITHUB, &crate::gitlab::host::GITLAB],
        before_hosts: &crate::github::host::GITHUB,
    };

    /// A registry of `units`, with the host of references from before
    /// hosts were named, which is one of them: a test's own.
    #[cfg(test)]
    pub(crate) const fn new(
        units: &'static [&'static dyn CodeHost],
        before_hosts: &'static dyn CodeHost,
    ) -> Self {
        Self {
            units,
            before_hosts,
        }
    }

    pub(crate) fn hosts(self) -> impl Iterator<Item = Host> {
        self.units.iter().map(|unit| Host(*unit))
    }

    /// The host a reference that names none belongs to: one saved or sent
    /// before hosts were named.
    pub(crate) fn before_hosts(self) -> Host {
        Host(self.before_hosts)
    }

    /// The host a reference's recorded key names, or, for none, the host of
    /// references from before hosts were named.
    pub(crate) fn recorded(self, key: Option<&str>) -> Option<Host> {
        match key {
            None | Some("") => Some(self.before_hosts()),
            Some(key) => self.host(key),
        }
    }

    /// The registered host of `key`.
    pub(crate) fn host(self, key: &str) -> Option<Host> {
        self.hosts().find(|host| host.key() == key)
    }

    /// The host and the repository a stored name names: the host that names
    /// its server, else the first registered host that reads it.
    pub(crate) fn repository(self, stored: &str) -> Option<(Host, RepositoryRef)> {
        self.best(|host| host.repository(stored))
    }

    /// The host and the repository a remote's address names, by the same
    /// rule as [`CodeHosts::repository`].
    pub(crate) fn remote(self, url: &str) -> Option<(Host, RepositoryRef)> {
        self.best(|host| host.remote_repository(url))
    }

    /// The host, repository and number a change request's address names, by
    /// the same rule as [`CodeHosts::repository`].
    pub(crate) fn change_request_at(self, url: &str) -> Option<(Host, RepositoryRef, u32)> {
        let (host, repository) = self.best(|host| {
            host.change_request_at(url)
                .map(|(repository, _)| repository)
        })?;
        let (_, number) = host.change_request_at(url)?;
        Some((host, repository, number))
    }

    /// The reference the unit of the host an address is on makes for it:
    /// only for an address Hide holds without one (a session's record, a
    /// mission node's or a promotion's saved address).
    pub(crate) fn reference_at(self, url: &str) -> Option<ChangeRequestRef> {
        let (host, repository, number) = self.change_request_at(url)?;
        Some(change_request(host.unit(), repository, number))
    }

    /// Whether `repository` is on a server its host is known by name
    /// (github.com for GitHub): where a session's printed address
    /// (`hide_session::sightings`) and the link record have always found
    /// change requests, and a GitHub Enterprise server's never were.
    pub(crate) fn on_named_server(self, repository: &RepositoryRef) -> bool {
        self.of(repository)
            .is_some_and(|host| host.names_server(&repository.authority))
    }

    /// The name the link record and a session's references key
    /// `repository` by, the one place it is folded: its host's stored name
    /// in lower case, as a session's reader spells it (`hide_session::hosts`),
    /// so github.com's is the bare `owner/name` the record has always held
    /// and gitlab.com's carries its server (`gitlab.com/group/project`); on a
    /// server its host does not name (GitHub Enterprise), its path, as the
    /// record held it before hosts. `None` for a host not registered.
    pub(crate) fn record_name(self, repository: &RepositoryRef) -> Option<String> {
        let host = self.of(repository)?;
        let name = if host.names_server(&repository.authority) {
            host.stored_name(repository)
        } else {
            repository.path.clone()
        };
        Some(name.to_ascii_lowercase())
    }

    /// The record's key for the host's own id of `repository`: the id under
    /// the server part of its record name, so two hosts' ids never meet
    /// (`gitlab.com/<id>`), and github.com's bare id as it always was.
    pub(crate) fn record_id(self, repository: &RepositoryRef, id: &str) -> Option<String> {
        let name = self.record_name(repository)?;
        let server = name
            .strip_suffix(&repository.path.to_ascii_lowercase())
            .unwrap_or_default();
        Some(format!("{server}{id}"))
    }

    fn best(self, read: impl Fn(Host) -> Option<RepositoryRef>) -> Option<(Host, RepositoryRef)> {
        let read: Vec<(Host, RepositoryRef)> = self
            .hosts()
            .filter_map(|host| read(host).map(|repository| (host, repository)))
            .collect();
        let named = read
            .iter()
            .position(|(host, repository)| host.names_server(&repository.authority))
            .unwrap_or(0);
        read.into_iter().nth(named)
    }

    /// The clock of a project no registered host reads (answered "no
    /// remote"): the slowest registered host's, which asks least.
    pub(crate) fn unplaced_clock(self) -> ReadClock {
        self.hosts()
            .map(|host| host.read_clock())
            .max_by_key(|clock| clock.reread)
            .expect("at least one host is registered")
    }

    /// `items` in their order, at most the largest registered
    /// [`CodeHost::project_limit`] of them in all, and of those each host's
    /// past its own limit left out; an item no host reads (`None`) takes a
    /// place in the total and in no host's. With one host this is its limit
    /// over every item, as before hosts.
    pub(crate) fn within_limits<T>(
        self,
        items: impl IntoIterator<Item = T>,
        host_of: impl Fn(&T) -> Option<Host>,
    ) -> Vec<T> {
        let total = self
            .hosts()
            .map(|host| host.project_limit())
            .max()
            .unwrap_or(0);
        let mut kept = 0;
        let mut counts: std::collections::HashMap<Host, usize> = std::collections::HashMap::new();
        items
            .into_iter()
            .filter(|item| {
                if kept == total {
                    return false;
                }
                if let Some(host) = host_of(item) {
                    let count = counts.entry(host).or_default();
                    if *count == host.project_limit() {
                        return false;
                    }
                    *count += 1;
                }
                kept += 1;
                true
            })
            .collect()
    }

    /// The longest query every host that searches takes; `None` when no
    /// registered host searches.
    pub(crate) fn search_query_limit(self) -> Option<usize> {
        self.hosts()
            .filter_map(|host| host.unit().search())
            .map(Search::query_limit)
            .min()
    }

    /// The host of a reference, which names it by key.
    pub(crate) fn of(self, repository: &RepositoryRef) -> Option<Host> {
        self.host(&repository.host)
    }

    /// Where a repository is read by its stored name: a repository the
    /// operator connected a project to, or a device's `origin`.
    pub(crate) fn place_of(self, stored: &str) -> Option<Place> {
        self.repository(stored)
            .map(|(host, repository)| Place::Repository { host, repository })
    }

    /// Where a checkout of the core's machine is read, from its remotes as
    /// its node read them:
    ///
    /// - no remote that names a server (an empty list of addresses):
    ///   [`Placement::NoRemote`], answered "no remote" with no host asked;
    /// - an older node's GitHub-shaped names and none of them:
    ///   [`Placement::RemotesNotKnown`], since those names say nothing of
    ///   another host's remote; answered with no host asked and kept apart
    ///   from no remote;
    /// - remotes of one host alone: that host, in the checkout when it finds
    ///   the remote itself, else by the first remote's repository;
    /// - remotes the node did not list, an address no registered unit reads,
    ///   or remotes of several hosts: the first registered host that finds a
    ///   checkout's remote itself, which picks one in the checkout (GitHub's
    ///   `gh` does, which keeps a project with a GitHub mirror beside another
    ///   host's `origin` on GitHub); `None` when no host does.
    ///
    /// An older node's GitHub-shaped names are read as stored names, as they
    /// were; a name no unit reads names nothing.
    pub(crate) fn placement_of_checkout(
        self,
        root: &Path,
        remotes: CheckoutRemotes<'_>,
    ) -> Placement {
        let in_checkout =
            |host: Host, named: &[(Host, RepositoryRef)], complete: bool| Place::Checkout {
                host,
                root: root.to_path_buf(),
                remotes: named
                    .iter()
                    .filter(|(named, _)| *named == host)
                    .map(|(_, repository)| repository.clone())
                    .collect(),
                remotes_complete: complete,
            };
        let resolving = |named: &[(Host, RepositoryRef)], complete: bool| {
            self.hosts()
                .find(|host| host.resolves_checkout())
                .map_or(Placement::NoRemote, |host| {
                    Placement::At(in_checkout(host, named, complete))
                })
        };
        let (named, listed): (Vec<(Host, RepositoryRef)>, usize) = match remotes {
            CheckoutRemotes::Unknown => return resolving(&[], false),
            CheckoutRemotes::Named([]) => return Placement::RemotesNotKnown,
            CheckoutRemotes::Addresses(addresses) => (
                addresses
                    .iter()
                    .filter_map(|address| self.remote(address))
                    .collect(),
                addresses.len(),
            ),
            CheckoutRemotes::Named(names) => (
                names
                    .iter()
                    .filter_map(|name| self.repository(name))
                    .collect(),
                names.len(),
            ),
        };
        let complete = named.len() == listed;
        if !complete && matches!(remotes, CheckoutRemotes::Addresses(_)) {
            return resolving(&named, false);
        }
        let Some((first, rest)) = named.split_first() else {
            return Placement::NoRemote;
        };
        if rest.iter().any(|(host, _)| *host != first.0) {
            return resolving(&named, complete);
        }
        let (host, repository) = first.clone();
        Placement::At(if host.resolves_checkout() {
            in_checkout(host, &named, complete)
        } else {
            Place::Repository { host, repository }
        })
    }
}

/// What the registry makes of a checkout's remotes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Placement {
    /// Its host reads it there.
    At(Place),
    /// No remote names a server a registered host reads.
    NoRemote,
    /// The node did not say enough to tell: an older node names only
    /// GitHub-shaped remotes, and named none. Not a failed read and not no
    /// remote; a later pass from a node that sends its addresses decides.
    RemotesNotKnown,
}

impl Placement {
    pub(crate) fn as_place(&self) -> Option<&Place> {
        match self {
            Self::At(place) => Some(place),
            Self::NoRemote | Self::RemotesNotKnown => None,
        }
    }

    /// The place alone, for a test that asserts only where a checkout is
    /// read.
    #[cfg(test)]
    pub(crate) fn place(self) -> Option<Place> {
        match self {
            Self::At(place) => Some(place),
            Self::NoRemote | Self::RemotesNotKnown => None,
        }
    }
}

/// A checkout's remotes as its node read them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CheckoutRemotes<'a> {
    /// The node did not say (its config could not be read or includes
    /// another file, or the node has not answered).
    Unknown,
    /// Each remote's address, without a credential, a local path left out.
    Addresses(&'a [String]),
    /// The GitHub-shaped names a node older than the addresses sends.
    Named(&'a [String]),
}

impl<'a> CheckoutRemotes<'a> {
    /// The list as the node sent it, to notice a change; `None` while
    /// unknown.
    pub(crate) fn listed(self) -> Option<&'a [String]> {
        match self {
            Self::Unknown => None,
            Self::Addresses(list) | Self::Named(list) => Some(list),
        }
    }
}

/// One host's read of its projects' change requests, issues and identity,
/// owned by the common reader ([`read::CodeHostReader`]) for its life. What
/// it keeps between passes (its sign-in for a pass, what it carries from one
/// answer to the next) is its own; the common reader decides which projects
/// are due and keeps the answers.
pub(crate) trait HostReads: Send {
    /// Reads this host's projects a pass found due, and answers each, in the
    /// order given. An answer's `root_path` is the one it was asked with.
    fn read(&mut self, node: &dyn NodeLink, due: &[DueRead<'_>]) -> Vec<GithubProjectSnapshot>;

    /// Lets go of what it keeps for a project no longer asked about.
    fn forget(&mut self, asked: &dyn Fn(&Path) -> bool);
}

/// One project a pass reads.
pub(crate) struct DueRead<'a> {
    /// The path the project was asked about, which names it between passes.
    pub(crate) root: &'a Path,
    pub(crate) place: &'a Place,
    /// The project as the answer names it: the main worktree of a checkout
    /// read in place, else the path asked about.
    pub(crate) root_path: &'a str,
    /// The issues the project's checkouts and records link, read even when
    /// the repository's own list does not hold them.
    pub(crate) links: &'a [crate::issues::IssueReference],
    pub(crate) generation: u64,
}

/// What one card read found. A part that could not be understood is `None`
/// and named in `failed`; the others stand (B52).
#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) struct CardRead {
    pub head: Option<String>,
    pub body: Option<String>,
    pub files: Option<Vec<crate::model::PrCardFile>>,
    pub files_total: u32,
    pub threads: Option<Vec<crate::model::PrCardThread>>,
    pub merge: Option<crate::model::PrCardMerge>,
    pub failed: Vec<&'static str>,
    /// When the head commit's checks last completed: the latest completed
    /// run's time (PRD pr-card-and-session-record-redesign D-15); `None`
    /// while none has completed or the answer lacks it.
    pub checks_completed_at: Option<u64>,
    /// When the review that stands (the latest approval or change request)
    /// was submitted; `None` without one.
    pub review_at: Option<u64>,
    /// The time parts the answer did not carry in a readable shape
    /// (`checks`, `review`): the card stands without them and the caller
    /// logs why (B21). A part that is there and empty is not named.
    pub times_unread: Vec<&'static str>,
}

/// Why a write from a card did not happen: a code the card words, and the
/// host's own text for the diagnostic log.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct WriteFailure {
    pub reason: &'static str,
    pub detail: String,
}

pub(crate) trait HeadMerge: Sync {
    /// Merges change request `number` only while its head is `head`; one
    /// already merged is a success, so asking again converges (D-22).
    fn merge(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        number: u32,
        method: Option<hide_node_link::change_request::PrMergeMethod>,
        head: &str,
    ) -> Result<(), WriteFailure>;
}

pub(crate) trait UpdateBranch: Sync {
    /// Brings change request `number`'s branch up to its base only while its
    /// head is `head`.
    fn update_branch(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        number: u32,
        head: &str,
    ) -> Result<(), WriteFailure>;
}

pub(crate) trait EditBody: Sync {
    /// Writes the line that closes `issue` at the end of change request
    /// `number`'s description, once however often it is asked: `true` when
    /// it wrote it, `false` when the description already closes the issue.
    fn write_closing_line(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        number: u32,
        issue: u32,
    ) -> Result<bool, String>;
}

/// The exact head and base of a merged change request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MergedHead {
    pub head_oid: String,
    /// The branch it merged into. Only `main` proves anything about main; a
    /// stacked change request merged into its parent branch does not.
    pub base_ref: String,
}

pub(crate) trait MergeProof: Sync {
    /// The commit change request `number` merged as, `None` while it has not.
    fn merge_commit(
        &self,
        node: &dyn NodeLink,
        repository: &RepositoryRef,
        number: u32,
    ) -> Result<Option<String>, String>;

    /// Whether `head`'s history holds `commit`.
    fn contains_commit(
        &self,
        node: &dyn NodeLink,
        repository: &RepositoryRef,
        commit: &str,
        head: &str,
    ) -> Result<bool, String>;

    /// The exact heads and bases of the merged change requests of `branch`.
    fn merged_heads(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        branch: &str,
    ) -> Result<Vec<MergedHead>, String>;
}

pub(crate) trait BasePolicy: Sync {
    /// Whether the signed-in account may push straight to `base` in the
    /// repository of the checkout at `root` on `node`.
    fn base_push(&self, node: &dyn NodeLink, root: &Path, base: &str) -> Result<bool, String>;
}

pub(crate) trait CommitChecks: Sync {
    /// Whether the host reads the commit checks of `repository`.
    fn covers(&self, repository: &RepositoryRef) -> bool;

    /// What each of `commits` (exact commit ids) has for checks.
    fn read(
        &self,
        node: &dyn NodeLink,
        repository: &RepositoryRef,
        commits: &[String],
    ) -> Result<Vec<crate::checkout_git::Ci>, String>;
}

/// The end of a failed check job's log.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct JobLog {
    pub(crate) text: String,
    /// Whether the host cut the log's start off to keep it within its bound.
    pub(crate) cut: bool,
}

pub(crate) trait FailedLog: Sync {
    /// The log of check job `job`'s failed steps, cut to its end.
    fn failed_log(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        job: u64,
    ) -> Result<JobLog, HostFailure>;

    /// The job a check's address names, `None` for an address of another
    /// kind of check.
    fn job_of(&self, address: &str) -> Option<u64>;
}

/// How a change request read by its number stands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Standing {
    Open,
    Closed,
    Merged,
}

/// A change request's head, read by its number.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ChangeHead {
    pub(crate) branch: String,
    pub(crate) cross_repository: bool,
    pub(crate) repository: RepositoryRef,
}

/// Why a change request's head could not be read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ChangeHeadFailure {
    /// The repository has no change request of that number.
    Missing,
    /// The host could not be asked, or answered something Hide cannot read.
    Unread(String),
}

pub(crate) trait ByNumber: Sync {
    fn standing(&self, node: &dyn NodeLink, place: &Place, number: u32)
    -> Result<Standing, String>;

    fn head(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        number: u32,
    ) -> Result<ChangeHead, ChangeHeadFailure>;
}

/// A change request's failed checks and standing change requests, to hand an
/// agent.
#[derive(Debug)]
pub(crate) struct Feedback {
    pub(crate) body: String,
    pub(crate) failed_checks: Vec<crate::model::PrFailedCheck>,
    pub(crate) change_requests: Vec<crate::model::PrChangeRequest>,
}

pub(crate) trait Handover: Sync {
    fn feedback(&self, node: &dyn NodeLink, place: &Place, number: u32)
    -> Result<Feedback, String>;
}

/// Why a create did not name its issue: the host failed, or it answered with
/// no issue Hide could read, which may still have been made.
#[derive(Clone, Debug)]
pub(crate) enum CreateFailure {
    Host(HostFailure),
    Unreadable(String),
}

impl CreateFailure {
    /// The host's own words for it, for a caller that only reports it.
    pub(crate) fn reason(self) -> String {
        match self {
            Self::Host(failure) => failure.reason,
            Self::Unreadable(reason) => reason,
        }
    }
}

pub(crate) trait Issues: Sync {
    /// The repository the place's issues are in, by its stored name; a
    /// refusal code when it cannot be read.
    fn issues_repository(&self, node: &dyn NodeLink, place: &Place) -> Result<String, String>;

    fn create(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        title: &str,
        body: &str,
    ) -> Result<crate::issues::IssueSnapshot, CreateFailure>;

    fn close(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        reference: &crate::issues::IssueReference,
    ) -> Result<(), HostFailure>;

    fn detail(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        reference: &crate::issues::IssueReference,
        after: Option<&str>,
    ) -> Result<crate::tasks::TaskDetail, String>;

    fn linked(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        reference: &crate::issues::IssueReference,
    ) -> Result<crate::issues::IssueSnapshot, String>;

    /// The issue an earlier create with this title made at or after `since`
    /// (unix ms), which no other attempt has `claimed`, when the create's
    /// answer was lost.
    fn reconcile(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        repository: &str,
        title: &str,
        since: u64,
        claimed: &[u64],
    ) -> Result<Option<crate::issues::IssueSnapshot>, String>;
}

/// A project a search may cover: the host that serves it, where it lives,
/// and its repository's stored name when the reader has already resolved it.
#[derive(Clone, Debug)]
pub(crate) struct SearchTarget {
    pub(crate) host: Host,
    pub(crate) root: PathBuf,
    pub(crate) repository: Option<String>,
}

pub(crate) trait Search: Sync {
    /// The most repositories one search covers.
    fn repository_limit(&self) -> usize;

    /// The longest query it takes.
    fn query_limit(&self) -> usize;

    fn search(
        &self,
        node: &dyn NodeLink,
        targets: &[SearchTarget],
        query: &str,
    ) -> Result<Vec<crate::model::GithubSearchResult>, String>;
}

/// What one watch pass reads.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct WatchRequest {
    pub(crate) projects: Vec<Watched>,
}

/// One project a watch looks at.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Watched {
    /// The project's root path, which names it to the runtime.
    pub(crate) path: String,
    /// The repository its open change requests are in.
    pub(crate) repository: RepositoryRef,
    /// The project the operator is looking at, read on every pass.
    pub(crate) in_front: bool,
    /// The head commits of open change requests whose checks had not
    /// finished (`PullRequestSnapshot::checks_unfinished`).
    pub(crate) heads: Vec<String>,
}

/// The projects a pass found a change in, and those it is not watching now.
#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) struct WatchAnswer {
    pub(crate) changed: Vec<String>,
    /// Paused after a failure, or every project at the host's rate floor:
    /// their marks are stale until a read answers (PRD git-freshness B23).
    pub(crate) held: Vec<String>,
}

pub(crate) trait Watch: Sync {
    /// The most projects one pass reads.
    fn project_limit(&self) -> usize;

    /// The most head commits per project whose checks are read.
    fn head_limit(&self) -> usize;

    /// A watch running on `node`, owned by the caller and ended with it.
    fn reader(&self, node: std::sync::Arc<dyn NodeLink>) -> Box<dyn WatchReader>;
}

pub(crate) trait WatchReader: Send {
    /// The projects changed since the pass before, when a pass finished on
    /// this wake; nothing otherwise. An empty request reads nothing.
    fn read_if_due(&mut self, request: WatchRequest) -> Option<WatchAnswer>;
}

#[cfg(test)]
pub(crate) mod tests;
