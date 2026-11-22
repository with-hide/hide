//! The one reader of every project's change requests, whichever host serves
//! it.
//!
//! Like the worktree reader, it runs on a worker thread: a host's read
//! reaches the network and routinely takes a second or more, which would
//! otherwise be a second of coordinator latency for every engine pane event.
//! A pass decides which projects are due, answers a checkout no registered
//! host reads as "no remote" without asking any, hands each host the due
//! projects it reads ([`HostReads`]), and keeps every answer, so the next
//! pass re-reads only what is due and hands the rest back as it was.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{CodeHosts, DueRead, Host, HostReads, Place};
use crate::model::{
    GithubFailureCategory, GithubProjectSnapshot, GithubReadOutcome, GithubSnapshot,
    GithubStatusSnapshot,
};
use crate::node_access::NodeLink;
use crate::reader::BackgroundRead;
use hide_node_link::git::GitCommand;
use hide_node_link::protocol::Call;

/// How long the node may take to name a checkout's main worktree.
const MAIN_WORKTREE_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GithubProjectRequest {
    pub(crate) links: Vec<crate::issues::IssueReference>,
    /// A path inside the repository.
    pub(crate) root: PathBuf,
    /// Where its host reads it, as the registry chose
    /// ([`CodeHosts::place_of_checkout`], [`CodeHosts::place_of`]); `None`
    /// when its node read the checkout's remotes and none names a repository
    /// a registered host reads, which is answered "no remote" with no host
    /// asked.
    pub(crate) place: Option<Place>,
    /// Bumped on Git section opening and explicit header refresh.
    /// Each repository's generation coalesces repeated requests independently.
    pub(crate) generation: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct GithubRequest {
    pub(crate) projects: Vec<GithubProjectRequest>,
}

pub(crate) struct GithubAnswer {
    pub(crate) request: GithubRequest,
    pub(crate) snapshot: GithubSnapshot,
}

/// Why one project is read on this pass, stated in the read log so "did the
/// refresh button / section opening actually re-read?" is answered
/// afterwards without a debugger (G5).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReadReason {
    First,
    Generation,
}

impl ReadReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::First => "first",
            Self::Generation => "generation",
        }
    }
}

/// Whether a project must be read again, given what the reader last answered
/// for it: nothing yet or an answer for an older request. An answer for the
/// same request is reused indefinitely, which is what keeps one project's
/// trigger from re-reading every other project.
pub(crate) fn read_reason(cached: Option<u64>, generation: u64) -> Option<ReadReason> {
    match cached {
        None => Some(ReadReason::First),
        Some(read_generation) if read_generation != generation => Some(ReadReason::Generation),
        Some(_) => None,
    }
}

struct Cached {
    generation: u64,
    place: Option<Place>,
    answer: GithubProjectSnapshot,
}

/// The reader's own state: each host's read, and the last answer per
/// requested root. The runtime receives a whole snapshot every time and does
/// not know which half is new.
/// What the reader keeps between passes: each host's own read and the
/// cache of answers.
pub(super) struct Reads {
    hosts: Vec<(Host, Box<dyn HostReads>)>,
    cache: HashMap<PathBuf, Cached>,
}

impl Reads {
    pub(super) fn new(hosts: CodeHosts) -> Self {
        Self {
            hosts: hosts.hosts().map(|host| (host, host.reads())).collect(),
            cache: HashMap::new(),
        }
    }
}

pub(crate) struct CodeHostReader {
    inner: BackgroundRead<GithubRequest, GithubAnswer>,
}

impl CodeHostReader {
    /// A reader whose hosts run on `node`, the core's own machine, where the
    /// operator's sign-ins are.
    pub(crate) fn new(node: Arc<dyn NodeLink>, hosts: CodeHosts) -> Self {
        let reads = std::sync::Mutex::new(Reads::new(hosts));
        Self {
            inner: BackgroundRead::on_change(Duration::ZERO, move |request: &GithubRequest| {
                // The worker is the only thread that touches the state, and
                // one worker runs at a time; the lock lets the closure be
                // shared with it.
                let mut reads = reads
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                GithubAnswer {
                    request: request.clone(),
                    snapshot: read(&mut reads, node.as_ref(), request),
                }
            }),
        }
    }

    pub(crate) fn read_if_due(&mut self, request: GithubRequest) -> Option<GithubAnswer> {
        self.inner.poll(request)
    }
}

/// One due project, placed for its host's read.
struct Placed<'a> {
    project: &'a GithubProjectRequest,
    place: Place,
    root_path: String,
}

/// One pass: what is due is read, each host's projects by its own read, and
/// the answer holds every asked project.
pub(super) fn read(
    reads: &mut Reads,
    node: &dyn NodeLink,
    request: &GithubRequest,
) -> GithubSnapshot {
    let asked = |root: &Path| request.projects.iter().any(|project| project.root == root);
    reads.cache.retain(|root, _| asked(root));
    for (_, host) in &mut reads.hosts {
        host.forget(&asked);
    }
    let due: Vec<(&GithubProjectRequest, ReadReason)> = request
        .projects
        .iter()
        .filter_map(|project| {
            let cached = reads
                .cache
                .get(&project.root)
                .filter(|cached| cached.place == project.place)
                .map(|cached| cached.generation);
            read_reason(cached, project.generation).map(|reason| (project, reason))
        })
        .collect();
    if !due.is_empty() {
        // One line per pass naming each project read and why, with the
        // generation that asked for it.
        crate::diagnostic!(serde_json::json!({
            "component": "github",
            "kind": "pull_requests.read",
            "projects": due
                .iter()
                .map(|(project, reason)| serde_json::json!({
                    "project": project.root.to_string_lossy(),
                    "generation": project.generation,
                    "reason": reason.as_str(),
                }))
                .collect::<Vec<_>>(),
        }));
        let mut placed: Vec<Placed> = Vec::new();
        for (project, _) in due {
            let started = Instant::now();
            let found = match &project.place {
                Some(Place::Repository { .. }) => Some((
                    project.place.clone(),
                    hide_platform::path::to_wire_lossy(&project.root),
                )),
                Some(Place::Checkout {
                    host,
                    root,
                    remotes,
                    remotes_complete,
                }) => main_worktree(node, root).map(|main| {
                    let root_path = hide_platform::path::to_wire_lossy(&main);
                    (
                        Some(Place::Checkout {
                            host: *host,
                            root: main,
                            remotes: remotes.clone(),
                            remotes_complete: *remotes_complete,
                        }),
                        root_path,
                    )
                }),
                None => main_worktree(node, &project.root)
                    .map(|main| (None, hide_platform::path::to_wire_lossy(&main))),
            };
            let answer = match found {
                // A path inside no repository has nothing to report.
                None => GithubProjectSnapshot::default(),
                Some((Some(place), root_path)) => {
                    placed.push(Placed {
                        project,
                        place,
                        root_path,
                    });
                    continue;
                }
                Some((None, root_path)) => {
                    let answer = no_remote(root_path);
                    log_answer(project.generation, &answer, started, None);
                    answer
                }
            };
            reads.cache.insert(
                project.root.clone(),
                Cached {
                    generation: project.generation,
                    place: project.place.clone(),
                    answer,
                },
            );
        }
        for (host, host_reads) in &mut reads.hosts {
            let mine: Vec<&Placed> = placed
                .iter()
                .filter(|placed| *placed.place.host() == *host)
                .collect();
            if mine.is_empty() {
                continue;
            }
            let due: Vec<DueRead> = mine
                .iter()
                .map(|placed| DueRead {
                    root: &placed.project.root,
                    place: &placed.place,
                    root_path: &placed.root_path,
                    links: &placed.project.links,
                    generation: placed.project.generation,
                })
                .collect();
            let declaration = super::declaration(host.unit());
            let mut answers = host_reads.read(node, &due);
            // Every answer a host gives carries its declaration, set here
            // once rather than by each host.
            for answer in &mut answers {
                answer.status.host = Some(declaration.clone());
            }
            debug_assert_eq!(
                answers.len(),
                mine.len(),
                "a host answers every due project"
            );
            for (placed, answer) in mine.iter().zip(answers) {
                reads.cache.insert(
                    placed.project.root.clone(),
                    Cached {
                        generation: placed.project.generation,
                        place: placed.project.place.clone(),
                        answer,
                    },
                );
            }
        }
    }
    let mut projects: Vec<GithubProjectSnapshot> = Vec::new();
    for project in &request.projects {
        let Some(cached) = reads.cache.get(&project.root) else {
            continue;
        };
        // Two navigator entries inside one repository share its answer.
        if cached.answer.root_path.is_empty()
            || projects
                .iter()
                .any(|known| known.root_path == cached.answer.root_path)
        {
            continue;
        }
        projects.push(cached.answer.clone());
    }
    GithubSnapshot { projects }
}

/// The main worktree of the repository `path` is in, from its node's git;
/// `None` when `path` is in no repository.
fn main_worktree(node: &dyn NodeLink, path: &Path) -> Option<PathBuf> {
    let call = Call::Git {
        root: path.to_string_lossy().into_owned(),
        command: GitCommand::CommonDir,
    };
    let common: String = crate::node_access::call_as(node, call, MAIN_WORKTREE_TIMEOUT).ok()?;
    if common.is_empty() {
        return None;
    }
    PathBuf::from(common).parent().map(Path::to_path_buf)
}

/// A repository with nothing a registered host can answer for: an answer,
/// not a failure (design principle 13), so it is neither stale nor retried,
/// and the screen draws no host state for it.
pub(crate) fn no_remote(root_path: String) -> GithubProjectSnapshot {
    GithubProjectSnapshot {
        root_path,
        status: GithubStatusSnapshot {
            available: true,
            failure_category: Some(GithubFailureCategory::NoGithubRemote),
            ..GithubStatusSnapshot::default()
        },
        ..GithubProjectSnapshot::default()
    }
}

/// States how one project's read ended, with a failure and an empty answer
/// told apart: an empty list with no reason is a repository with no change
/// requests. A read that answered without an optional part is a success,
/// with the reason as its warning.
pub(crate) fn log_answer(
    generation: u64,
    project: &GithubProjectSnapshot,
    started: Instant,
    warning: Option<String>,
) {
    crate::diagnostic!(serde_json::json!({
        "component": "github",
        "kind": if project.read_outcome() == GithubReadOutcome::NoRemote {
            "pull_requests.no_remote"
        } else if project.status.unavailable_reason.is_some() {
            "pull_requests.failed"
        } else if project.pull_requests.is_empty() {
            "pull_requests.empty"
        } else {
            "pull_requests.ok"
        },
        "generation": generation,
        "project": project.root_path,
        "available": project.status.available,
        "pull_requests": project.pull_requests.len(),
        "duration_ms": started.elapsed().as_millis() as u64,
        "message": project.status.unavailable_reason,
        "warning": warning,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_is_read_first_then_only_when_its_generation_moves() {
        assert_eq!(read_reason(None, 0), Some(ReadReason::First));
        assert_eq!(read_reason(Some(0), 1), Some(ReadReason::Generation));
        assert_eq!(read_reason(Some(1), 1), None);
    }
}
