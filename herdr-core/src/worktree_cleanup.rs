//! Reviewed cleanup of a project's disk: the build caches and dependencies a
//! tool makes again, and clean linked worktrees merged into the repository's
//! default branch (`worktrees::merged`, the judgment the sidebar shares). A
//! worktree whose only panes are an agent that finished or a shell is chosen
//! like any other, and the confirmation closes those panes before it removes.
//! All inspection and filesystem effects run on the existing action-worker
//! path, never under the runtime lock.
use super::{ApiConnector, ChangeNotifier, GhPlace, Runtime, control_request};
use crate::model::ListeningPortSnapshot;
use crate::node_access::{NodeLink, call_as};
use crate::{disk, github};
use hide_node_link::cleanup::{
    CleanRemoval, FolderJudgments, FolderToJudge, FolderVerdict, PathState, RepositoryDirs,
    WalkAllowance,
};
use hide_node_link::disk::{Layer, LayerFolder};
use hide_node_link::git::GitCommand;
use hide_node_link::protocol::Call;
use hide_node_link::worktrees::Registered;
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

/// How long one read of the checkouts' node may take: the node bounds each
/// git command at fifteen seconds and a removal at sixty.
const NODE_TIMEOUT: Duration = Duration::from_secs(90);

/// One run's look through the folders it empties for a nested repository:
/// crossing it leaves the remaining folders unverified, so kept.
const RUN_WALK_ENTRIES: u64 = 20_000_000;
const RUN_WALK_TIME: Duration = Duration::from_secs(300);

/// One fixed git command on the checkouts' node.
fn git(node: &dyn NodeLink, root: &Path, command: GitCommand) -> Result<String, String> {
    let call = Call::Git {
        root: root.to_string_lossy().into_owned(),
        command,
    };
    call_as(node, call, NODE_TIMEOUT).map_err(|error| error.to_string())
}

/// Each path as the checkouts' node resolves it.
fn real_paths(node: &dyn NodeLink, paths: &[PathBuf]) -> Result<Vec<PathState>, String> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let call = Call::RealPaths {
        paths: paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
    };
    let states: Vec<PathState> = call_as(node, call, NODE_TIMEOUT)
        .map_err(|error| format!("Folders could not be resolved: {error}"))?;
    if states.len() != paths.len() {
        return Err("The node resolved another number of folders than it was asked".into());
    }
    Ok(states)
}

/// Each path resolved, or as it is when it cannot be: a folder that is gone
/// is still compared by its own spelling.
fn canonical_all(node: &dyn NodeLink, paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    Ok(real_paths(node, paths)?
        .into_iter()
        .zip(paths)
        .map(|(state, path)| match state {
            PathState::Real { path } => PathBuf::from(path),
            PathState::Missing | PathState::Unreadable { .. } => path.clone(),
        })
        .collect())
}

fn canonical(node: &dyn NodeLink, path: &Path) -> Result<PathBuf, String> {
    Ok(canonical_all(node, &[path.to_path_buf()])?.remove(0))
}

/// The repository's shared Git directory, where removed folders wait.
fn common_dir(node: &dyn NodeLink, root: &Path) -> PathBuf {
    let call = Call::Repository {
        path: root.to_string_lossy().into_owned(),
    };
    call_as::<Option<RepositoryDirs>>(node, call, NODE_TIMEOUT)
        .ok()
        .flatten()
        .map_or_else(|| root.join(".git"), |repo| PathBuf::from(repo.common_dir))
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct CleanupSnapshot {
    pub id: u64,
    pub workspace_id: String,
    pub repository_root: String,
    pub phase: String,
    /// The local head of the default branch the review judged merges
    /// against; a confirmation refuses when it moved.
    pub main_head: Option<String>,
    pub rows: Vec<CleanupRow>,
    pub message: Option<String>,
    /// Why "in use" could not be read (no Herdr connection, a pane or port
    /// read failed). While it stands nothing may be chosen.
    pub usage_error: Option<String>,
    /// The in-use reads are in, so build caches and dependencies may be chosen
    /// while worktree eligibility is still being checked.
    pub usage_ready: bool,
    pub progress: Option<CleanupProgress>,
    pub cell_results: Vec<CellResult>,
    /// The project volume's free bytes when the review read them, before the
    /// removal started, and after the removed folders were deleted.
    pub free_bytes: Option<u64>,
    pub free_before: Option<u64>,
    pub free_after: Option<u64>,
    /// Heads GitHub answered merged into the default branch when the review
    /// asked by branch, past the pull request list the sidebar reads. The
    /// core keeps them so the sidebar's landed mark gives the same answer
    /// (PRD git-freshness B17); never sent to the shell.
    #[serde(skip)]
    pub merge_proofs: BTreeSet<String>,
    /// The default branch `merge_proofs` were judged against.
    #[serde(skip)]
    pub merge_base: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct CleanupProgress {
    pub done: usize,
    pub total: usize,
}

/// Why a checkout cannot be emptied while it is being worked in.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct InUse {
    /// `agent_working`, `agent_waiting` (a question, an approval or a blocked
    /// prompt), `descendant_busy` (an agent it delegated to is working or
    /// waiting, wherever that runs), `process`, `port`, or `unverified` for a
    /// checkout, or an agent of it, the runtime cannot vouch for, which is
    /// never read as idle.
    pub code: &'static str,
    pub name: Option<String>,
    pub port: Option<u16>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct CleanupRow {
    pub path: String,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub is_main: bool,
    /// The reason the worktree may not be removed, in words for the log
    /// only; the shell reads `exclusion_code`.
    #[serde(skip)]
    pub exclusion: Option<String>,
    pub exclusion_code: Option<&'static str>,
    pub exclusion_count: Option<u32>,
    pub in_use: Option<InUse>,
    /// How many live panes of the checkout close with it, which are the
    /// panes of finished agents and shells that nothing here is using.
    pub pane_count: u32,
    /// The panes a confirmed removal closes first, by Herdr's ids.
    #[serde(skip)]
    pub pane_ids: Vec<String>,
    /// `removed`, `skipped` (a recheck found it changed or busy) or `failed`
    /// (Git refused the removal), once a confirmation has settled the row.
    pub result: Option<String>,
    /// Why a `skipped` or `failed` row was not removed: an exclusion code,
    /// `changed`, `not_found`, `unverified`, `close_refused` (a pane would not
    /// close), or `remove_refused`.
    pub result_code: Option<&'static str>,
    /// The allocated bytes of the worktree this run removed.
    pub bytes: Option<u64>,
    /// The words behind `result_code`, for the log only.
    #[serde(skip)]
    pub message: Option<String>,
}

/// What became of one chosen build-cache or dependency cell.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CellResult {
    pub path: String,
    pub layer: &'static str,
    /// `removed`, `skipped` or `failed`.
    pub outcome: &'static str,
    pub bytes: u64,
    pub folders: usize,
    pub reason_code: Option<&'static str>,
    /// The words behind `reason_code`, for the log only.
    #[serde(skip)]
    pub reason: Option<String>,
}

/// What the runtime knows about one checkout, copied under the lock for the
/// worker's in-use read.
#[derive(Clone, Debug, Default)]
pub(crate) struct CheckoutFacts {
    pub path: PathBuf,
    /// Agents of the checkout whose activity is Working.
    pub agent_working: usize,
    /// Agents of the checkout holding a question, an approval or a blocked
    /// prompt: they wait for the operator and the work is not finished.
    pub agent_waiting: usize,
    /// Agents of the checkout whose activity Herdr does not report, which is
    /// never read as idle.
    pub agent_unknown: usize,
    /// Live descendants of the checkout's agents, running anywhere, that are
    /// working or waiting for the operator.
    pub descendants_busy: usize,
    /// Panes that are not an agent's; their foreground process is read.
    pub terminal_panes: Vec<String>,
    /// Every pane of the checkout, which a confirmed removal closes.
    pub panes: Vec<String>,
}

#[cfg(test)]
use crate::agent_state::AgentUse;

#[derive(Clone, Debug)]
pub(crate) struct ReviewInput {
    pub workspace_id: String,
    /// The project's path as its rows spell it: the key its repository facts
    /// are filed under, and the root every answer names, never respelled
    /// through `root`.
    pub repository_root: String,
    pub root: PathBuf,
    /// The checkout the operator is looking at, when it is in this project.
    pub current: Option<PathBuf>,
    pub checkouts: Vec<CheckoutFacts>,
    /// The folders each checkout's layer cells were measured from.
    pub folders: HashMap<PathBuf, Vec<LayerFolder>>,
    /// Each worktree's measured allocated bytes, for the result's totals.
    pub bytes: HashMap<PathBuf, u64>,
    pub merge: MergeBasis,
}

/// What a worktree's merge is judged against (PRD git-freshness D-06,
/// D-07), copied under the lock with the rest of the input.
#[derive(Clone, Debug, Default)]
pub(crate) struct MergeBasis {
    /// The repository's default branch as the worktree reader resolved it:
    /// the chosen base, GitHub's default, or `origin/HEAD`. `None` when
    /// none is known, and then nothing can be proven merged.
    pub base: Option<String>,
    /// The head commits of the pull requests GitHub merged into `base`,
    /// from the pull request list the sidebar reads too, a device's own
    /// list for a device project. `None` when this core has read no list
    /// for the project, which is then asked of GitHub one branch at a time.
    pub heads: Option<BTreeSet<String>>,
}

/// Where a cleanup runs: the machine whose disk holds the project, through
/// its Herdr (`api_connector`) and its node, with the runtime its answers go
/// to; and where `gh` reads a squash merge's proof (D-16), none for a
/// node's repository that names no GitHub one. A node's project is cleaned
/// up on that node, never on this machine's disk (D-29).
#[derive(Clone)]
pub(crate) struct CleanupPlace {
    runtime: Weak<Mutex<Runtime>>,
    notifier: ChangeNotifier,
    api_connector: Arc<dyn ApiConnector>,
    node: Arc<dyn NodeLink>,
    /// Where merge proofs are read, or why they cannot be.
    github: Result<GhPlace, String>,
}

impl CleanupPlace {
    /// This machine's project: its Herdr and the core's own node.
    pub(crate) fn local(context: super::LiveContext, github: Result<GhPlace, String>) -> Self {
        Self {
            runtime: context.runtime,
            notifier: context.notifier,
            api_connector: context.api_connector,
            node: context.node,
            github,
        }
    }

    /// A node's project: the node's Herdr and its link.
    pub(crate) fn node(
        control: &super::RemoteControlContext,
        node: Arc<dyn NodeLink>,
        github: Result<GhPlace, String>,
    ) -> Self {
        Self {
            runtime: control.runtime.clone(),
            notifier: control.notifier.clone(),
            api_connector: Arc::clone(&control.api_connector),
            node,
            github,
        }
    }
}

/// A build cache or dependency cell the operator chose.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CellChoice {
    pub path: String,
    pub layer: Layer,
}

/// How many terminal panes one review asks Herdr about. A project past it
/// cannot be judged, and says so instead of guessing.
const PANE_READ_LIMIT: usize = 256;

pub(crate) type InUseMap = HashMap<PathBuf, InUse>;

/// What the runtime and Herdr said about each checkout: whether it is in use,
/// and the panes that close with it.
#[derive(Clone, Debug, Default)]
struct Usage {
    in_use: InUseMap,
    panes: HashMap<PathBuf, Vec<String>>,
}

impl Usage {
    fn of(in_use: InUseMap, facts: &[CheckoutFacts]) -> Self {
        Self {
            in_use,
            panes: facts
                .iter()
                .map(|facts| (facts.path.clone(), facts.panes.clone()))
                .collect(),
        }
    }

    /// Writes what is known of the row's checkout onto it.
    fn mark(&self, row: &mut CleanupRow) {
        let path = Path::new(&row.path);
        row.in_use = self.in_use.get(path).cloned();
        row.pane_ids = self.panes.get(path).cloned().unwrap_or_default();
        row.pane_count = u32::try_from(row.pane_ids.len()).unwrap_or(u32::MAX);
    }
}

/// Which checkouts are being worked in: an agent is Working or waiting on the
/// operator, an agent it delegated to is, a program other than a shell holds
/// a terminal pane of the checkout, or a process started inside it listens on
/// a port. A finished or idle agent, and a shell, are not use. `process`
/// answers a pane's program, and any read that fails fails the whole answer:
/// an unread checkout is not idle.
pub(crate) fn read_in_use(
    node: &dyn NodeLink,
    checkouts: &[CheckoutFacts],
    ports: Result<Vec<ListeningPortSnapshot>, String>,
    mut process: impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<InUseMap, String> {
    let ports = ports?;
    let paths: Vec<PathBuf> = checkouts.iter().map(|facts| facts.path.clone()).collect();
    let real = canonical_all(node, &paths)?;
    let mut in_use = InUseMap::new();
    let mut reads = 0usize;
    for (checkout, path) in checkouts.iter().zip(real) {
        let agent = [
            (checkout.agent_working, "agent_working"),
            (checkout.agent_waiting, "agent_waiting"),
            (checkout.descendants_busy, "descendant_busy"),
            (checkout.agent_unknown, "unverified"),
        ]
        .into_iter()
        .find_map(|(count, code)| (count > 0).then_some(code));
        if let Some(code) = agent {
            in_use.insert(
                checkout.path.clone(),
                InUse {
                    code,
                    name: None,
                    port: None,
                },
            );
            continue;
        }
        if let Some(port) = ports
            .iter()
            .filter(|listener| Path::new(&listener.cwd).starts_with(&path))
            .map(|listener| listener.port)
            .min()
        {
            in_use.insert(
                checkout.path.clone(),
                InUse {
                    code: "port",
                    name: None,
                    port: Some(port),
                },
            );
            continue;
        }
        for pane in &checkout.terminal_panes {
            reads += 1;
            if reads > PANE_READ_LIMIT {
                return Err(format!(
                    "More than {PANE_READ_LIMIT} terminal panes are open, too many to check for running programs"
                ));
            }
            if let Some(name) = process(pane)? {
                in_use.insert(
                    checkout.path.clone(),
                    InUse {
                        code: "process",
                        name: Some(name),
                        port: None,
                    },
                );
                break;
            }
        }
    }
    Ok(in_use)
}

/// The in-use read against the live Herdr and the machine's listening ports.
fn live_in_use(context: &CleanupPlace, checkouts: &[CheckoutFacts]) -> Result<InUseMap, String> {
    let ports = crate::ports::read_now(context.node.as_ref());
    let ports = match ports.unavailable_reason {
        Some(reason) => Err(format!("Listening ports could not be read: {reason}")),
        None => Ok(ports.entries),
    };
    read_in_use(context.node.as_ref(), checkouts, ports, |pane| {
        let value = control_request(
            context.api_connector.as_ref(),
            "pane.process_info",
            crate::wire::pane_process_info_params(pane)?,
        )?;
        let group = crate::wire::pane_process_group(value)?;
        if group.foreground_pids.is_empty() || group.shell_holds_terminal() {
            return Ok(None);
        }
        Ok(Some(
            group
                .foreground_program()
                .or_else(|| group.foreground_names.first().map(String::as_str))
                .unwrap_or("a program")
                .to_owned(),
        ))
    })
}

fn unverified_use() -> InUse {
    InUse {
        code: "unverified",
        name: None,
        port: None,
    }
}

/// A registered worktree the runtime has no facts about cannot be shown to
/// be idle, so it reads as unverified and nothing in it is emptied.
fn mark_unverified(
    node: &dyn NodeLink,
    in_use: &mut InUseMap,
    facts: &[CheckoutFacts],
    registered: Vec<PathBuf>,
) -> Result<(), String> {
    let facts: Vec<PathBuf> = facts.iter().map(|facts| facts.path.clone()).collect();
    let known: std::collections::HashSet<PathBuf> =
        canonical_all(node, &facts)?.into_iter().collect();
    let real = canonical_all(node, &registered)?;
    for (path, real) in registered.into_iter().zip(real) {
        if !known.contains(&real) {
            in_use.entry(path).or_insert_with(unverified_use);
        }
    }
    Ok(())
}

/// Whether one checkout is in use right now, with the panes a removal would
/// close: the facts are copied again under the lock and the checkout's panes
/// and ports are read again, so a decision is never older than the move it
/// guards. A checkout without facts is unverified.
fn read_in_use_now(
    node: &dyn NodeLink,
    path: &Path,
    facts: impl FnOnce() -> Option<Vec<CheckoutFacts>>,
    read: impl FnOnce(&[CheckoutFacts]) -> Result<InUseMap, String>,
) -> Result<(Option<InUse>, Vec<String>), String> {
    let facts = facts().ok_or("The project is no longer available")?;
    let key = canonical(node, path)?;
    let paths: Vec<PathBuf> = facts.iter().map(|facts| facts.path.clone()).collect();
    let real = canonical_all(node, &paths)?;
    let Some(checkout) = facts
        .into_iter()
        .zip(real)
        .find_map(|(facts, real)| (real == key).then_some(facts))
    else {
        return Ok((Some(unverified_use()), Vec::new()));
    };
    let map = read(std::slice::from_ref(&checkout))?;
    Ok((map.get(&checkout.path).cloned(), checkout.panes))
}

fn fresh_in_use(
    context: &CleanupPlace,
    workspace_id: &str,
    path: &Path,
) -> Result<(Option<InUse>, Vec<String>), String> {
    read_in_use_now(
        context.node.as_ref(),
        path,
        || {
            let runtime = context.runtime.upgrade()?;
            let guard = runtime.lock().ok()?;
            guard.cleanup_facts(workspace_id)
        },
        |facts| live_in_use(context, facts),
    )
}

/// A worktree that may not be removed, and why.
struct Exclusion {
    code: &'static str,
    count: Option<u32>,
    message: String,
}

impl Exclusion {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            count: None,
            message: message.into(),
        }
    }
}

/// A Git or filesystem read that failed leaves the worktree unverified.
impl From<String> for Exclusion {
    fn from(message: String) -> Self {
        Self::new("unverified", message)
    }
}

/// The review's rows: Git's registration with the reason each one is not a
/// candidate.
fn listed(node: &dyn NodeLink, root: &Path) -> Result<Vec<CleanupRow>, String> {
    let call = Call::WorktreesRegistered {
        root: root.to_string_lossy().into_owned(),
    };
    let registered: Vec<Registered> =
        call_as(node, call, NODE_TIMEOUT).map_err(|error| error.to_string())?;
    Ok(registered
        .into_iter()
        .enumerate()
        .map(|(index, registered)| {
            let (code, message) = if index == 0 {
                (Some("main"), Some("Main checkout"))
            } else if registered.locked {
                (
                    Some("locked"),
                    Some("Worktree is locked. Unlock it separately before reviewing again."),
                )
            } else if registered.unavailable {
                (Some("unavailable"), Some("Worktree is unavailable"))
            } else {
                (None, None)
            };
            CleanupRow {
                exclusion: message.map(Into::into),
                exclusion_code: code,
                is_main: index == 0,
                path: registered.path,
                branch: registered.branch,
                head: registered.head,
                ..Default::default()
            }
        })
        .collect())
}

/// Each live pane's id with a folder it stands in, from Herdr.
type PanePaths = Vec<(String, PathBuf)>;

fn pane_paths(context: &CleanupPlace) -> Result<PanePaths, String> {
    let response = control_request(
        context.api_connector.as_ref(),
        "session.snapshot",
        serde_json::json!({}),
    )?;
    verified_pane_paths(
        context.node.as_ref(),
        crate::wire::cleanup_usage_paths(response).map_err(|e| e.message().to_owned())?,
    )
}

fn verified_pane_paths(
    node: &dyn NodeLink,
    paths: Vec<(String, Option<String>)>,
) -> Result<PanePaths, String> {
    let paths: PanePaths = paths
        .into_iter()
        .map(|(pane, cwd)| {
            let cwd = cwd.ok_or("A live pane's folder is unknown. Resolve it before cleanup.")?;
            let path = PathBuf::from(cwd);
            if !path.is_absolute()
                || path.components().any(|component| {
                    matches!(
                        component,
                        std::path::Component::CurDir | std::path::Component::ParentDir
                    )
                })
            {
                return Err(
                    "A live pane's folder is not an absolute normalized path. Resolve it before cleanup."
                        .into(),
                );
            }
            Ok((pane, path))
        })
        .collect::<Result<_, String>>()?;
    // A pane can outlive the worktree folder it launched in. Keep its
    // absolute path for component-wise ownership checks instead of letting
    // that one stale folder disable cleanup for every current worktree.
    // Existing paths still resolve aliases before matching.
    let folders: Vec<PathBuf> = paths.iter().map(|(_, path)| path.clone()).collect();
    let real = canonical_all(node, &folders)?;
    Ok(paths
        .into_iter()
        .zip(real)
        .map(|((pane, _), real)| (pane, real))
        .collect())
}

fn ref_commit(node: &dyn NodeLink, root: &Path, name: &str) -> Option<String> {
    git(
        node,
        root,
        GitCommand::RefCommit {
            name: name.to_owned(),
        },
    )
    .ok()
}

/// Whether the worktree at `head` is merged, by the judgment the sidebar
/// shares (`worktrees::merged`): its HEAD is in a commit `bases` names (the
/// default branch, locally or as last fetched), or GitHub merged a pull
/// request into that branch at exactly this HEAD. A squash or rebase merge
/// leaves the head out of the branch's history, so GitHub's answer is the
/// only proof then; it needs no merge commit in any local ref.
fn verified_merge(
    node: &dyn NodeLink,
    root: &Path,
    (base, bases): (&str, &[String]),
    branch: &str,
    head: &str,
    merged_heads: &mut impl FnMut(&str, &str) -> Result<BTreeSet<String>, String>,
) -> Result<(), Exclusion> {
    let mut in_base = false;
    for commit in bases {
        let count = git(
            node,
            root,
            GitCommand::CountCommits {
                base: commit.clone(),
                head: head.to_owned(),
            },
        )?;
        match count.trim().parse::<u64>() {
            Ok(0) => in_base = true,
            Err(_) => {
                return Err(Exclusion::new(
                    "merge_unverified",
                    "Merge status could not be verified",
                ));
            }
            Ok(_) => {}
        }
    }
    let heads = if in_base {
        None
    } else {
        Some(merged_heads(branch, head)?)
    };
    if crate::worktrees::merged(Some(in_base), Some(head), heads.as_ref()) == Some(true) {
        return Ok(());
    }
    Err(Exclusion::new(
        "not_merged",
        format!(
            "Not merged into {base}: HEAD is in neither local {base} nor origin/{base}, and no pull request merged into {base} exactly matches it"
        ),
    ))
}

/// The registered worktrees with the reason each may not be removed. `only`
/// limits the checks to those paths, so a confirmation rereads what it is
/// about to remove and nothing else.
fn inspect(
    context: &CleanupPlace,
    root: &Path,
    current: Option<&Path>,
    panes: Result<PanePaths, String>,
    usage: &Usage,
    only: Option<&[String]>,
    merge: &MergeBasis,
) -> Result<CleanupSnapshot, String> {
    let node = context.node.as_ref();
    let base = merge.base.as_deref();
    let mut proved = BTreeSet::new();
    let mut snapshot = inspect_with_merge_proofs(
        node,
        root,
        current,
        panes,
        usage,
        only,
        base,
        |branch, head| {
            let heads = merge_proof(merge.heads.as_ref(), head, || {
                let github = context
                    .github
                    .as_ref()
                    .map_err(|why| format!("GitHub merge proof is unavailable: {why}"))?;
                let base = base.ok_or("No default branch is known to judge a merge against")?;
                Ok(heads_into(
                    github::merged_pull_request_proofs(
                        github.node.as_ref(),
                        &github.place,
                        branch,
                    )?,
                    base,
                ))
            })?;
            if heads.contains(head) && merge.heads.as_ref().is_none_or(|list| !list.contains(head))
            {
                proved.insert(head.to_owned());
            }
            Ok(heads)
        },
    )?;
    snapshot.merge_proofs = proved;
    snapshot.merge_base = merge.base.clone();
    Ok(snapshot)
}

/// GitHub's merged heads for one worktree. The pull request list the sidebar
/// reads (`window`) answers when it holds this HEAD; it holds only the newest
/// pull requests, so a HEAD it does not hold is asked of GitHub by its branch,
/// and an older squash merge is still found. With no list the branch is asked
/// alone. A lookup that failed leaves the worktree unverified, never not
/// merged: GitHub did not say.
fn merge_proof(
    window: Option<&BTreeSet<String>>,
    head: &str,
    by_branch: impl FnOnce() -> Result<BTreeSet<String>, String>,
) -> Result<BTreeSet<String>, String> {
    match window {
        Some(heads) if heads.contains(head) => Ok(heads.clone()),
        None => by_branch(),
        Some(heads) => Ok(heads.iter().cloned().chain(by_branch()?).collect()),
    }
}

/// The heads of the pull requests that merged into `base`; one merged into
/// a stacked parent branch proves nothing about `base`.
fn heads_into(proofs: Vec<github::MergedPullRequestProof>, base: &str) -> BTreeSet<String> {
    proofs
        .into_iter()
        .filter(|proof| proof.base_ref == base)
        .map(|proof| proof.head_oid)
        .collect()
}

// The test seam of `inspect`: its merge proofs come from the caller.
#[allow(clippy::too_many_arguments)]
fn inspect_with_merge_proofs(
    node: &dyn NodeLink,
    root: &Path,
    current: Option<&Path>,
    panes: Result<PanePaths, String>,
    usage: &Usage,
    only: Option<&[String]>,
    base: Option<&str>,
    mut merged_heads: impl FnMut(&str, &str) -> Result<BTreeSet<String>, String>,
) -> Result<CleanupSnapshot, String> {
    let mut rows = listed(node, root)?;
    // Review never fetches, and a local base that nothing keeps current is
    // one of two answers, not the only one: the remote-tracking copy of the
    // last fetch is the other.
    let local_main = base.and_then(|base| ref_commit(node, root, &format!("refs/heads/{base}")));
    let mains: Vec<String> = [
        local_main.clone(),
        base.and_then(|base| ref_commit(node, root, &format!("refs/remotes/origin/{base}"))),
    ]
    .into_iter()
    .flatten()
    .collect();
    let canonical_current = match current {
        Some(current) => match real_paths(node, &[current.to_path_buf()])?.remove(0) {
            PathState::Real { path } => Some(PathBuf::from(path)),
            PathState::Missing | PathState::Unreadable { .. } => None,
        },
        None => None,
    };
    let paths: Vec<PathBuf> = rows.iter().map(|r| PathBuf::from(&r.path)).collect();
    for row in &mut rows {
        usage.mark(row);
        if row.exclusion.is_some() || only.is_some_and(|only| !only.contains(&row.path)) {
            continue;
        }
        let checked = (|| -> Result<(), Exclusion> {
            let path = Path::new(&row.path);
            let canonical = match real_paths(node, &[path.to_path_buf()])?.remove(0) {
                PathState::Real { path } => PathBuf::from(path),
                PathState::Missing | PathState::Unreadable { .. } => {
                    return Err(Exclusion::new("missing", "Folder is missing or unreadable"));
                }
            };
            if canonical != path {
                return Err(Exclusion::new(
                    "alias",
                    "Folder is an alias. Refresh authoritative worktree paths.",
                ));
            }
            if canonical_current.as_ref() == Some(&canonical) {
                return Err(Exclusion::new("current", "Current checkout"));
            }
            // Removing an ancestor would also remove another checkout's files.
            if paths
                .iter()
                .any(|other| other != path && other.starts_with(path))
            {
                return Err(Exclusion::new(
                    "contains_worktree",
                    "Contains another registered worktree",
                ));
            }
            if row.in_use.is_some() {
                return Err(Exclusion::new(
                    "in_use",
                    "In use: an agent is working or waiting, or a program is running here.",
                ));
            }
            // The checkout's own panes close with it; a pane that stands in
            // the folder without being one of them would be left behind.
            let panes = panes.as_ref().map_err(Clone::clone)?;
            if panes
                .iter()
                .any(|(id, cwd)| cwd.starts_with(&canonical) && !row.pane_ids.contains(id))
            {
                return Err(Exclusion::new(
                    "pane_open",
                    "A live pane that is not this checkout's own stands in it. Close or move it, then review again.",
                ));
            }
            let Some(base) = base else {
                return Err(Exclusion::new(
                    "main_unavailable",
                    "The repository's default branch is unknown. Choose a base branch in Project Settings, then review again.",
                ));
            };
            if mains.is_empty() {
                return Err(Exclusion::new(
                    "main_unavailable",
                    format!(
                        "Neither local {base} nor origin/{base} exists, so no base can establish eligibility."
                    ),
                ));
            }
            let head = row
                .head
                .as_ref()
                .ok_or_else(|| Exclusion::new("unverified", "Worktree HEAD is unknown"))?;
            if row.branch.is_none() {
                return Err(Exclusion::new(
                    "detached",
                    "Detached HEAD. Review this worktree separately.",
                ));
            }
            let changed = git(node, path, GitCommand::Status)?.lines().count();
            if changed > 0 {
                return Err(Exclusion {
                    code: "dirty",
                    count: Some(u32::try_from(changed).unwrap_or(u32::MAX)),
                    message:
                        "Uncommitted or untracked files. Commit or move them, then review again."
                            .into(),
                });
            }
            verified_merge(
                node,
                root,
                (base, &mains),
                row.branch.as_deref().expect("branch checked above"),
                head,
                &mut merged_heads,
            )?;
            // Removing the worktree deletes its ignored folders too, and a
            // repository cloned into one is lost with it.
            let call = Call::IgnoredRepository {
                worktree: path.to_string_lossy().into_owned(),
            };
            let ignored: Option<String> =
                call_as(node, call, NODE_TIMEOUT).map_err(|error| error.to_string())?;
            match ignored {
                Some(folder) => Err(Exclusion::new(
                    "nested_repository",
                    format!("The ignored folder {folder} holds its own Git repository"),
                )),
                None => Ok(()),
            }
        })();
        if let Err(exclusion) = checked {
            row.exclusion = Some(exclusion.message);
            row.exclusion_code = Some(exclusion.code);
            row.exclusion_count = exclusion.count;
        }
    }
    Ok(CleanupSnapshot {
        main_head: local_main,
        rows,
        phase: "review".into(),
        usage_ready: true,
        ..Default::default()
    })
}

/// Why a chosen worktree was not removed, as a code the shell reads and the
/// words for the log.
struct Refusal {
    outcome: &'static str,
    code: &'static str,
    message: String,
}

impl Refusal {
    fn failed(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            outcome: "failed",
            code,
            message: message.into(),
        }
    }

    fn skipped(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            outcome: "skipped",
            code,
            message: message.into(),
        }
    }
}

/// Removes each chosen worktree that a fresh look (`refresh`, made for that
/// one worktree just before its removal) still finds eligible and idle. Its
/// panes, as that look listed them, are closed first (`close`); a pane that
/// will not close keeps the worktree. Each row ends `removed`, `skipped` (a
/// recheck found it changed, busy or gone) or `failed` (a pane would not
/// close, or Git refused the removal).
fn confirm(
    review: &CleanupSnapshot,
    selected: &[String],
    mut refresh: impl FnMut(&str) -> Result<CleanupSnapshot, String>,
    mut close: impl FnMut(&str, &[String]) -> Result<(), Refusal>,
    mut remove: impl FnMut(&str) -> Result<String, String>,
    mut settled: impl FnMut(&CleanupRow),
    mut present: impl FnMut(&str) -> Result<bool, String>,
) -> CleanupSnapshot {
    let mut result = review.clone();
    result.phase = "complete".into();
    for row in &mut result.rows {
        if !selected.contains(&row.path) || row.result.as_deref() == Some("removed") {
            continue;
        }
        let validation = (|| -> Result<Vec<String>, Refusal> {
            if let Some(reason) = &row.exclusion {
                return Err(Refusal::skipped(
                    row.exclusion_code.unwrap_or("changed"),
                    format!("Excluded: {reason}"),
                ));
            }
            let fresh =
                refresh(&row.path).map_err(|message| Refusal::skipped("unverified", message))?;
            let Some(now) = fresh.rows.iter().find(|r| r.path == row.path) else {
                if !present(&row.path).map_err(|message| Refusal::skipped("unverified", message))? {
                    return Err(Refusal::skipped(
                        "not_found",
                        "Worktree was already removed. Branch and Git history stay.",
                    ));
                }
                return Err(Refusal::skipped(
                    "changed",
                    "Registration changed but the folder remains. Review again; no files were removed.",
                ));
            };
            if fresh.main_head != review.main_head
                || now.head != row.head
                || now.branch != row.branch
                || now.exclusion.is_some()
            {
                return Err(Refusal::skipped(
                    now.exclusion_code.unwrap_or("changed"),
                    format!(
                        "State changed. {} Review again before removing.",
                        now.exclusion
                            .as_deref()
                            .unwrap_or("The reviewed branch or the default branch's HEAD moved.")
                    ),
                ));
            }
            Ok(now.pane_ids.clone())
        })();
        let done = validation.and_then(|panes| {
            if !panes.is_empty() {
                close(&row.path, &panes)?;
            }
            Ok(remove(&row.path))
        });
        match done {
            Ok(Ok(message)) => {
                row.result = Some("removed".into());
                row.message = Some(message);
            }
            Ok(Err(message)) => {
                row.result = Some("failed".into());
                row.result_code = Some("remove_refused");
                row.message = Some(message);
            }
            Err(refusal) => {
                row.result = Some(refusal.outcome.into());
                row.result_code = Some(refusal.code);
                row.message = Some(refusal.message);
            }
        }
        settled(row);
    }
    result
}

/// Hands a snapshot to the runtime, which keeps it only while its id is the
/// live one. A runtime that cannot be locked is logged, never skipped quietly.
fn publish(context: &CleanupPlace, snapshot: CleanupSnapshot) {
    let id = snapshot.id;
    if let Some(runtime) = context.runtime.upgrade() {
        match runtime.lock() {
            Ok(mut guard) => {
                guard.ingest_cleanup(snapshot);
            }
            Err(_) => crate::diagnostic!(serde_json::json!({
                "component": "cleanup",
                "kind": "publish.lock_poisoned",
                "cleanup_id": id,
            })),
        }
        context.notifier.notify();
    }
}

/// Marks the worker done, so the lane is free again.
fn worker_finished(context: &CleanupPlace, id: u64) {
    if let Some(runtime) = context.runtime.upgrade() {
        match runtime.lock() {
            Ok(mut guard) => guard.cleanup_worker_finished(id),
            Err(_) => crate::diagnostic!(serde_json::json!({
                "component": "cleanup",
                "kind": "worker_finished.lock_poisoned",
                "cleanup_id": id,
            })),
        }
    }
}

/// Owns a worker's place in the lane: while it lives the daemon accepts no
/// other review or dismissal, and however the worker ends - it returns or it
/// panics - the lane is freed, and a worker that did not reach its own final
/// snapshot leaves a terminal one behind instead of a review stuck loading.
struct WorkerGuard {
    publish: Box<dyn Fn(CleanupSnapshot) + Send>,
    finished: Box<dyn Fn() + Send>,
    last: CleanupSnapshot,
    terminal_phase: &'static str,
    armed: bool,
}

impl WorkerGuard {
    fn new(
        publish: Box<dyn Fn(CleanupSnapshot) + Send>,
        finished: Box<dyn Fn() + Send>,
        start: CleanupSnapshot,
        terminal_phase: &'static str,
    ) -> Self {
        Self {
            publish,
            finished,
            last: start,
            terminal_phase,
            armed: true,
        }
    }

    fn for_context(
        context: &CleanupPlace,
        start: CleanupSnapshot,
        terminal_phase: &'static str,
    ) -> Self {
        let (a, b, id) = (context.clone(), context.clone(), start.id);
        Self::new(
            Box::new(move |snapshot| publish(&a, snapshot)),
            Box::new(move || worker_finished(&b, id)),
            start,
            terminal_phase,
        )
    }

    /// A step of the run, remembered so an unwind can say how far it got.
    fn publish(&mut self, snapshot: CleanupSnapshot) {
        self.last = snapshot.clone();
        (self.publish)(snapshot);
    }

    /// The run's own final snapshot.
    fn finish(mut self, snapshot: CleanupSnapshot) {
        self.armed = false;
        self.last = snapshot.clone();
        (self.publish)(snapshot);
    }
}

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        if self.armed {
            let mut last = self.last.clone();
            last.phase = self.terminal_phase.into();
            last.message = Some("The cleanup worker stopped before it finished".into());
            crate::diagnostic!(serde_json::json!({
                "component": "cleanup",
                "kind": "worker.stopped_early",
                "cleanup_id": last.id,
                "phase": self.terminal_phase,
            }));
            (self.publish)(last);
        }
        (self.finished)();
    }
}

/// The review worker: reads what is in use and publishes the rows with it at
/// once, so build caches can be chosen, then checks each worktree's
/// eligibility (Git, and GitHub for a squash merge) and publishes the answer.
pub fn spawn_review(context: CleanupPlace, id: u64, input: ReviewInput) -> Result<(), String> {
    std::thread::Builder::new()
        .name("hide-worktree-cleanup".into())
        .spawn(move || {
            let base = CleanupSnapshot {
                id,
                workspace_id: input.workspace_id.clone(),
                repository_root: input.repository_root.clone(),
                phase: "loading".into(),
                free_bytes: disk::volume_free_bytes(context.node.as_ref(), &input.root),
                ..Default::default()
            };
            let mut guard = WorkerGuard::for_context(&context, base.clone(), "failed");
            let usage = live_in_use(&context, &input.checkouts);
            let rows = listed(context.node.as_ref(), &input.root);
            let (in_use, usage_error) = match (usage, &rows) {
                (Ok(mut map), Ok(rows)) => match mark_unverified(
                    context.node.as_ref(),
                    &mut map,
                    &input.checkouts,
                    rows.iter().map(|row| PathBuf::from(&row.path)).collect(),
                ) {
                    Ok(()) => (map, None),
                    Err(message) => (InUseMap::new(), Some(message)),
                },
                (Ok(map), Err(_)) => (map, None),
                (Err(message), _) => (InUseMap::new(), Some(message)),
            };
            match rows {
                Ok(mut rows) => {
                    let usage = Usage::of(in_use.clone(), &input.checkouts);
                    for row in &mut rows {
                        usage.mark(row);
                    }
                    guard.publish(CleanupSnapshot {
                        rows,
                        usage_ready: usage_error.is_none(),
                        usage_error: usage_error.clone(),
                        ..base.clone()
                    });
                }
                Err(message) => {
                    guard.finish(CleanupSnapshot {
                        phase: "failed".into(),
                        message: Some(message),
                        ..base
                    });
                    return;
                }
            }
            // Without the in-use reads a worktree's eligibility cannot be
            // decided, so each one is unverified and none can be chosen.
            let panes = match &usage_error {
                None => pane_paths(&context),
                Some(message) => Err(message.clone()),
            };
            let usage = Usage::of(in_use, &input.checkouts);
            let answer = match inspect(
                &context,
                &input.root,
                input.current.as_deref(),
                panes,
                &usage,
                None,
                &input.merge,
            ) {
                Ok(value) => CleanupSnapshot {
                    id,
                    workspace_id: input.workspace_id.clone(),
                    repository_root: input.repository_root.clone(),
                    usage_error,
                    free_bytes: base.free_bytes,
                    ..value
                },
                Err(message) => CleanupSnapshot {
                    phase: "failed".into(),
                    message: Some(message),
                    usage_error,
                    ..base
                },
            };
            guard.finish(answer);
        })
        .map(|_| ())
        .map_err(|e| format!("Cleanup worker could not start: {e}"))
}

/// What emptying cells needs besides the cells themselves.
struct CellRun<'a> {
    node: &'a dyn NodeLink,
    id: u64,
    workspace_id: &'a str,
    /// The repository's shared Git directory, where removed folders wait.
    common: &'a Path,
    folders: &'a HashMap<PathBuf, Vec<LayerFolder>>,
}

/// A folder that cannot be moved, and the code the shell reads for it.
struct Kept {
    code: &'static str,
    reason: String,
}

fn cell_result(
    path: &str,
    layer: Layer,
    outcome: &'static str,
    bytes: u64,
    folders: usize,
    kept: Option<Kept>,
) -> CellResult {
    CellResult {
        path: path.to_owned(),
        layer: layer.code(),
        outcome,
        bytes,
        folders,
        reason_code: kept.as_ref().map(|kept| kept.code),
        reason: kept.map(|kept| kept.reason),
    }
}

/// Empties the chosen cells: right before each move the folder is judged
/// again from the files, and nothing moves unless it is still ignored, still
/// vouched for as its layer, reached through no link, holding no repository
/// and no file Git tracks (one `git ls-files` per checkout), and the
/// checkout is read as idle again (`in_use_now`) after all of that. A moved
/// folder leaves its checkout at once and is deleted afterwards; nothing here
/// deletes. Every outcome is one result and one diagnostic, and the entries
/// this run put in the trash are added to `moved`.
fn empty_cells(
    run: &CellRun,
    cells: &[CellChoice],
    mut in_use_now: impl FnMut(&Path) -> Result<Option<InUse>, String>,
    moved: &mut std::collections::BTreeSet<PathBuf>,
    mut settled: impl FnMut(&CellResult),
) -> Vec<CellResult> {
    let mut results = Vec::new();
    let mut checkouts: Vec<&str> = Vec::new();
    for cell in cells {
        if !checkouts.contains(&cell.path.as_str()) {
            checkouts.push(&cell.path);
        }
    }
    // One look through folders for repositories for the whole run.
    let walk_until = Instant::now() + RUN_WALK_TIME;
    let mut walk_entries = RUN_WALK_ENTRIES;
    let mut record = |result: CellResult| {
        crate::diagnostic!(serde_json::json!({
            "component": "cleanup",
            "kind": format!("cell.{}", result.outcome),
            "cleanup_id": run.id,
            "workspace_id": run.workspace_id,
            "checkout": result.path,
            "layer": result.layer,
            "bytes": result.bytes,
            "folders": result.folders,
            "reason_code": result.reason_code,
            "reason": result.reason,
        }));
        settled(&result);
        results.push(result);
    };
    for checkout in checkouts {
        let root = PathBuf::from(checkout);
        let chosen: Vec<Layer> = cells
            .iter()
            .filter(|cell| cell.path == checkout)
            .map(|cell| cell.layer)
            .collect();
        // Judge every folder of every chosen layer on the node, then ask Git
        // once.
        let folders: Vec<&LayerFolder> = chosen
            .iter()
            .flat_map(|layer| {
                run.folders
                    .get(&root)
                    .into_iter()
                    .flatten()
                    .filter(move |folder| folder.layer == *layer)
            })
            .collect();
        let verdicts = judge_folders(run.node, &root, &folders, &mut walk_entries, walk_until);
        let mut judged: Vec<(Layer, &LayerFolder, Option<Kept>)> = folders
            .into_iter()
            .zip(verdicts)
            .map(|(folder, verdict)| {
                let kept = match verdict {
                    Ok(FolderVerdict::Fits) => None,
                    Ok(FolderVerdict::Refused { refusal }) => Some(Kept {
                        code: refusal.code(),
                        reason: format!(
                            "The folder is no longer what was measured ({})",
                            refusal.code()
                        ),
                    }),
                    Ok(FolderVerdict::NestedRepository) => Some(Kept {
                        code: "nested_repository",
                        reason: "The folder holds another Git repository".into(),
                    }),
                    Ok(FolderVerdict::Unverified { reason }) | Err(reason) => Some(Kept {
                        code: "unverified",
                        reason,
                    }),
                };
                (folder.layer, folder, kept)
            })
            .collect();
        let tracked = tracked_folders(
            run.node,
            &root,
            judged
                .iter()
                .filter(|(_, _, kept)| kept.is_none())
                .map(|(_, folder, _)| folder.path.as_path()),
        );
        for (_, folder, kept) in &mut judged {
            if kept.is_some() {
                continue;
            }
            match &tracked {
                Err(message) => {
                    *kept = Some(Kept {
                        code: "unverified",
                        reason: message.clone(),
                    });
                }
                Ok(tracked) if tracked.contains(&folder.path) => {
                    *kept = Some(Kept {
                        code: "tracked_files",
                        reason: "Git tracks a file inside the folder".into(),
                    });
                }
                Ok(_) => {}
            }
        }
        // The last look, right before the moves.
        let busy = match in_use_now(&root) {
            Err(message) => Some(Kept {
                code: "unverified",
                reason: message,
            }),
            Ok(Some(use_)) if use_.code == "unverified" => Some(Kept {
                code: "unverified",
                reason: "Nothing is known about the checkout's panes".into(),
            }),
            Ok(Some(_)) => Some(Kept {
                code: "in_use",
                reason: "The checkout became busy after the review".into(),
            }),
            Ok(None) => None,
        };
        if let Some(busy) = busy {
            for layer in chosen {
                record(cell_result(
                    checkout,
                    layer,
                    "skipped",
                    0,
                    0,
                    Some(Kept {
                        code: busy.code,
                        reason: busy.reason.clone(),
                    }),
                ));
            }
            continue;
        }
        for layer in chosen {
            let (mut moved_count, mut bytes, mut failed) = (0usize, 0u64, None);
            let mut first_kept: Option<Kept> = None;
            let mut total = 0usize;
            for (_, folder, kept) in judged.iter_mut().filter(|(l, _, _)| *l == layer) {
                total += 1;
                if let Some(kept) = kept.take() {
                    first_kept.get_or_insert(kept);
                    continue;
                }
                let call = Call::SetAsideFolder {
                    common: run.common.to_string_lossy().into_owned(),
                    folder: folder.path.to_string_lossy().into_owned(),
                };
                match call_as::<PathBuf>(run.node, call, NODE_TIMEOUT) {
                    Ok(entry) => {
                        moved.insert(entry);
                        moved_count += 1;
                        bytes = bytes.saturating_add(folder.bytes);
                    }
                    Err(error) => {
                        failed.get_or_insert(Kept {
                            code: "io",
                            reason: error.to_string(),
                        });
                    }
                }
            }
            record(match (moved_count, failed, first_kept, total) {
                (_, _, _, 0) => cell_result(
                    checkout,
                    layer,
                    "skipped",
                    0,
                    0,
                    Some(Kept {
                        code: "changed",
                        reason: "No measured folder is left in this cell".into(),
                    }),
                ),
                (0, Some(failure), _, _) => {
                    cell_result(checkout, layer, "failed", 0, 0, Some(failure))
                }
                (0, None, kept, _) => cell_result(checkout, layer, "skipped", 0, 0, kept),
                (moved, failure, kept, _) => {
                    cell_result(checkout, layer, "removed", bytes, moved, failure.or(kept))
                }
            });
        }
    }
    results
}

/// Each folder's verdict from the checkout's node, in order, spending what is
/// left of the run's look through folders; a node that cannot be asked
/// leaves every folder unverified.
fn judge_folders(
    node: &dyn NodeLink,
    root: &Path,
    folders: &[&LayerFolder],
    walk_entries: &mut u64,
    walk_until: Instant,
) -> Vec<Result<FolderVerdict, String>> {
    if folders.is_empty() {
        return Vec::new();
    }
    let call = Call::JudgeFolders {
        root: root.to_string_lossy().into_owned(),
        folders: folders
            .iter()
            .map(|folder| FolderToJudge {
                layer: folder.layer,
                path: folder.path.to_string_lossy().into_owned(),
            })
            .collect(),
        walk: WalkAllowance {
            entries: *walk_entries,
            millis: u64::try_from(
                walk_until
                    .saturating_duration_since(Instant::now())
                    .as_millis(),
            )
            .unwrap_or(u64::MAX),
        },
    };
    let unverified = |reason: String| folders.iter().map(|_| Err(reason.clone())).collect();
    match call_as::<FolderJudgments>(node, call, RUN_WALK_TIME + NODE_TIMEOUT) {
        Ok(judged) if judged.verdicts.len() == folders.len() => {
            *walk_entries = judged.walk_left.entries;
            judged.verdicts.into_iter().map(Ok).collect()
        }
        Ok(_) => unverified("The node judged another number of folders than it was asked".into()),
        Err(error) => unverified(format!("The folders could not be judged: {error}")),
    }
}

/// Spelled the way a comparison may not tell apart: canonically composed
/// (macOS hands out decomposed Hangul, Git prints what the index holds) and
/// case-folded. Folding more than the file system does only skips more.
fn folded(text: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    text.nfc()
        .collect::<String>()
        .to_lowercase()
        .nfc()
        .collect()
}

/// The folders among `folders` that hold a file Git tracks, from one
/// `git ls-files -z` for the checkout. The whole index is listed and compared
/// by folded spelling, so a folder that the file system spells differently
/// from the index is still found, and no pathspec list can grow past what an
/// argument list may hold.
fn tracked_folders<'a>(
    node: &dyn NodeLink,
    root: &Path,
    folders: impl Iterator<Item = &'a Path>,
) -> Result<Vec<PathBuf>, String> {
    let folders: Vec<&Path> = folders.collect();
    if folders.is_empty() {
        return Ok(Vec::new());
    }
    let listed = git(node, root, GitCommand::ListFiles)?;
    let mut index: Vec<String> = listed
        .split('\0')
        .filter(|entry| !entry.is_empty())
        .map(folded)
        .collect();
    index.sort_unstable();
    Ok(folders
        .into_iter()
        .filter(|folder| {
            let Ok(relative) = folder.strip_prefix(root) else {
                // Not under the checkout: cannot be shown untracked.
                return true;
            };
            // Git lists the index in the wire's relative spelling, `/`
            // between names on every system.
            let relative = hide_platform::path::RelPath::from_native(relative)
                .map(hide_platform::path::RelPath::into_string)
                .unwrap_or_else(|_| relative.to_string_lossy().into_owned());
            let below = format!("{}/", folded(&relative));
            let at = index.partition_point(|entry| entry.as_str() < below.as_str());
            index.get(at).is_some_and(|entry| entry.starts_with(&below))
        })
        .map(Path::to_path_buf)
        .collect())
}

/// How long the removal waits for the background deletion of what it moved
/// before it reports what it has: a folder that will not go stays in the
/// trash for the next removal's sweep.
const DELETE_WAIT: Duration = Duration::from_secs(600);

/// Closes a chosen worktree's panes as the last step before its removal. What
/// was judged idle was judged a few checks ago, so the checkout, its panes and
/// the folder's live panes are read once more: a checkout that became busy, a
/// pane that appeared, or one that is not the checkout's own keeps the
/// worktree, and a pane that is already gone needs no closing.
fn close_checkout(
    context: &CleanupPlace,
    workspace_id: &str,
    id: u64,
    path: &str,
    reviewed: &[String],
) -> Result<(), Refusal> {
    let unverified = |message: String| Refusal::skipped("unverified", message);
    let (in_use, facts) =
        fresh_in_use(context, workspace_id, Path::new(path)).map_err(unverified)?;
    if in_use.is_some() {
        return Err(Refusal::skipped(
            "in_use",
            "The checkout became busy after the review. No panes were closed.",
        ));
    }
    let live = pane_paths(context).map_err(unverified)?;
    let folder = canonical(context.node.as_ref(), Path::new(path)).map_err(unverified)?;
    let to_close = panes_to_close(&folder, reviewed, &facts, &live)?;
    crate::diagnostic!(serde_json::json!({
        "component": "cleanup",
        "kind": "worktree.panes_closing",
        "cleanup_id": id,
        "workspace_id": workspace_id,
        "checkout": path,
        "pane_count": to_close.len(),
    }));
    if to_close.is_empty() {
        return Ok(());
    }
    super::close_checkout_panes(
        context.api_connector.as_ref(),
        context.node.as_ref(),
        &[path.to_owned()],
        &to_close,
        super::ProcessWait::for_folder_removal(true),
        super::CONFIRM_TIMEOUT,
    )
    .map_err(|message| Refusal::failed("close_refused", message))
}

/// The panes of a checkout that still exist, when they are exactly the ones
/// the review listed and nothing else stands in the folder. Anything else is a
/// change the operator has not seen.
fn panes_to_close(
    folder: &Path,
    reviewed: &[String],
    facts: &[String],
    live: &PanePaths,
) -> Result<Vec<String>, Refusal> {
    let sorted = |panes: &[String]| {
        let mut panes = panes.to_vec();
        panes.sort();
        panes
    };
    if sorted(reviewed) != sorted(facts) {
        return Err(Refusal::skipped(
            "changed",
            "The checkout's panes changed after the review. No panes were closed.",
        ));
    }
    if live
        .iter()
        .any(|(pane, cwd)| cwd.starts_with(folder) && !facts.contains(pane))
    {
        return Err(Refusal::skipped(
            "pane_open",
            "A pane that is not this checkout's own stands in it. No panes were closed.",
        ));
    }
    Ok(facts
        .iter()
        .filter(|pane| live.iter().any(|(live, _)| live == *pane))
        .cloned()
        .collect())
}

pub fn spawn_confirm(
    context: CleanupPlace,
    review: CleanupSnapshot,
    input: ReviewInput,
    selected: Vec<String>,
    cells: Vec<CellChoice>,
) -> Result<(), String> {
    std::thread::Builder::new()
        .name("hide-worktree-cleanup".into())
        .spawn(move || {
            let root = PathBuf::from(&review.repository_root);
            let id = review.id;
            let total = selected.len() + cells.len();
            let mut done = 0usize;
            let mut answer = CleanupSnapshot {
                phase: "removing".into(),
                free_before: disk::volume_free_bytes(context.node.as_ref(), &root),
                progress: Some(CleanupProgress { done, total }),
                ..review.clone()
            };
            let mut guard = WorkerGuard::for_context(&context, answer.clone(), "complete");
            guard.publish(answer.clone());
            let node = context.node.as_ref();
            let common = common_dir(node, &root);
            let mut ours = std::collections::BTreeSet::new();
            // Worktrees first: each is rechecked against what is on disk and
            // in Herdr right now, just before its own removal, and removed
            // without force.
            let refresh = |path: &str| {
                let (in_use, panes) =
                    fresh_in_use(&context, &review.workspace_id, Path::new(path))?;
                let usage = Usage {
                    in_use: in_use
                        .map(|use_| InUseMap::from([(PathBuf::from(path), use_)]))
                        .unwrap_or_default(),
                    panes: HashMap::from([(PathBuf::from(path), panes)]),
                };
                inspect(
                    &context,
                    &root,
                    input.current.as_deref(),
                    pane_paths(&context),
                    &usage,
                    Some(&[path.to_owned()]),
                    &input.merge,
                )
            };
            let confirmed = confirm(
                &review,
                &selected,
                refresh,
                |path, panes| close_checkout(&context, &review.workspace_id, id, path, panes),
                |path| {
                    let call = Call::WorktreeRemoveClean {
                        root: root.to_string_lossy().into_owned(),
                        checkout: path.to_owned(),
                        common: common.to_string_lossy().into_owned(),
                    };
                    let removal: CleanRemoval =
                        call_as(node, call, NODE_TIMEOUT).map_err(|error| error.to_string())?;
                    ours.extend(removal.trashed.into_iter().map(PathBuf::from));
                    if removal.outcome.removed {
                        Ok(removal.outcome.message)
                    } else {
                        Err(removal.outcome.message)
                    }
                },
                |row| {
                    let bytes = (row.result.as_deref() == Some("removed"))
                        .then(|| input.bytes.get(Path::new(&row.path)).copied())
                        .flatten();
                    crate::diagnostic!(serde_json::json!({
                        "component": "cleanup",
                        "kind": format!("worktree.{}", row.result.as_deref().unwrap_or("unknown")),
                        "cleanup_id": id,
                        "workspace_id": review.workspace_id,
                        "checkout": row.path,
                        "layer": "worktree",
                        "bytes": bytes,
                        "reason_code": row.result_code,
                        "reason": row.message,
                    }));
                    done += 1;
                },
                |path| match real_paths(node, &[PathBuf::from(path)])?.remove(0) {
                    PathState::Real { .. } => Ok(true),
                    PathState::Missing => Ok(false),
                    PathState::Unreadable { reason } => Err(reason),
                },
            );
            answer.rows = confirmed.rows;
            for row in &mut answer.rows {
                if row.result.as_deref() == Some("removed") && row.bytes.is_none() {
                    row.bytes = input.bytes.get(Path::new(&row.path)).copied();
                }
            }
            answer.progress = Some(CleanupProgress { done, total });
            guard.publish(answer.clone());
            let run = CellRun {
                node,
                id,
                workspace_id: &review.workspace_id,
                common: &common,
                folders: &input.folders,
            };
            empty_cells(
                &run,
                &cells,
                |path| Ok(fresh_in_use(&context, &review.workspace_id, path)?.0),
                &mut ours,
                |result| {
                    answer.cell_results.push(result.clone());
                    done += 1;
                    answer.progress = Some(CleanupProgress { done, total });
                    guard.publish(answer.clone());
                },
            );
            // The folders left their checkouts at once; the volume only has
            // its space back once they are deleted.
            let call = Call::DrainTrash {
                common: common.to_string_lossy().into_owned(),
                ours: ours
                    .iter()
                    .map(|entry| entry.to_string_lossy().into_owned())
                    .collect(),
                wait_ms: u64::try_from(DELETE_WAIT.as_millis()).unwrap_or(u64::MAX),
            };
            let remaining = match call_as::<usize>(node, call, DELETE_WAIT + NODE_TIMEOUT) {
                Ok(remaining) => remaining,
                Err(error) => {
                    crate::diagnostic!(serde_json::json!({
                        "component": "cleanup",
                        "kind": "trash.drain_failed",
                        "cleanup_id": id,
                        "error": error.to_string(),
                    }));
                    ours.len()
                }
            };
            if remaining > 0 {
                answer.message = Some(format!(
                    "{remaining} removed folders are still being deleted in the background"
                ));
            }
            answer.phase = "complete".into();
            answer.progress = Some(CleanupProgress { done: total, total });
            answer.free_after = disk::volume_free_bytes(context.node.as_ref(), &root);
            answer.id = id;
            guard.finish(answer);
        })
        .map(|_| ())
        .map_err(|e| format!("Cleanup worker could not start: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hide_host::worktrees::{ConfirmedRemoval, RemovalBranch, RemovalStop, remove_confirmed};

    /// Git run in the fixture, as the test sets it up.
    fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
        hide_host::worktrees::git(dir, args)
    }

    /// The machine the fixtures are on, as the core reaches it.
    fn node() -> hide_node::Local {
        hide_node::Local::of_process()
    }
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: PathBuf,
        main: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            hide_host::worktrees::record_git_calls();
            let root = std::env::temp_dir().join(format!(
                "hide-cleanup-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(root.join("main")).unwrap();
            let root = std::fs::canonicalize(root).unwrap();
            let main = root.join("main");
            git(&main, &["init", "-b", "main"]).unwrap();
            std::fs::write(main.join("tracked"), "keep").unwrap();
            // Build output is ignored, as in this repository, so a checkout
            // that has been built still counts as clean and removable.
            std::fs::write(main.join(".gitignore"), "target/\n").unwrap();
            git(&main, &["add", "."]).unwrap();
            git(
                &main,
                &[
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "commit",
                    "-m",
                    "Initial",
                ],
            )
            .unwrap();
            Self { root, main }
        }
        fn add(&self, name: &str) -> PathBuf {
            let path = self.root.join(name);
            git(
                &self.main,
                &[
                    "worktree",
                    "add",
                    "-b",
                    name,
                    path.to_str().unwrap(),
                    "main",
                ],
            )
            .unwrap();
            path
        }
        fn review(&self, current: &Path, usage: Result<PanePaths, String>) -> CleanupSnapshot {
            inspect_with_merge_proofs(
                &node(),
                &self.main,
                Some(current),
                usage,
                &Usage::default(),
                None,
                Some("main"),
                |_, _| Ok(BTreeSet::new()),
            )
            .unwrap()
        }
        /// A review judged against `base` as the repository's default
        /// branch, with GitHub's merged pull requests `proofs`.
        fn review_against(
            &self,
            base: Option<&str>,
            proofs: Vec<github::MergedPullRequestProof>,
        ) -> CleanupSnapshot {
            inspect_with_merge_proofs(
                &node(),
                &self.main,
                Some(&self.main),
                Ok(Vec::new()),
                &Usage::default(),
                None,
                base,
                |_, _| Ok(heads_into(proofs.clone(), base.unwrap())),
            )
            .unwrap()
        }
        fn review_with_proofs(
            &self,
            current: &Path,
            proofs: Vec<github::MergedPullRequestProof>,
        ) -> CleanupSnapshot {
            inspect_with_merge_proofs(
                &node(),
                &self.main,
                Some(current),
                Ok(Vec::new()),
                &Usage::default(),
                None,
                Some("main"),
                |_, _| Ok(heads_into(proofs.clone(), "main")),
            )
            .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn commit(path: &Path, message: &str) {
        git(
            path,
            &[
                "-c",
                "commit.gpgsign=false",
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "--allow-empty",
                "-m",
                message,
            ],
        )
        .unwrap();
    }

    fn confirmed(f: &Fixture, worktree: &Path, delete_branch: bool) -> ConfirmedRemoval {
        ConfirmedRemoval {
            repository_root: f.main.to_string_lossy().into_owned(),
            checkout_path: worktree.to_string_lossy().into_owned(),
            expected_head_sha: Some(
                git(worktree, &["rev-parse", "HEAD"])
                    .unwrap()
                    .trim()
                    .to_owned(),
            ),
            expected_branch: worktree
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            protected_base_branch: Some("main".into()),
            delete_branch: delete_branch
                .then(|| worktree.file_name().unwrap().to_string_lossy().into_owned()),
            force_delete_branch: false,
            discard_changes: false,
            expected_ignored_repositories: Vec::new(),
        }
    }

    fn branch_exists(f: &Fixture, branch: &str) -> bool {
        !git(&f.main, &["branch", "--list", branch])
            .unwrap()
            .trim()
            .is_empty()
    }

    #[test]
    fn a_confirmed_removal_keeps_a_dirty_worktree_and_removes_it_once_clean() {
        let f = Fixture::new();
        let worktree = f.add("linked");
        let request = confirmed(&f, &worktree, false);
        std::fs::write(worktree.join("tracked"), "dirty").unwrap();

        let refused = remove_confirmed(&request).unwrap_err();
        assert_eq!(refused.stop, RemovalStop::Changed { files: 1 });
        assert!(
            refused.message.contains("panes already closed stay closed"),
            "{}",
            refused.message
        );
        assert!(worktree.exists());

        std::fs::write(worktree.join("tracked"), "keep").unwrap();
        let removed = remove_confirmed(&request).unwrap();
        assert_eq!(removed.branch, RemovalBranch::Kept);
        assert!(!worktree.exists());
        assert!(branch_exists(&f, "linked"));
    }

    /// The operator ticked both boxes: the dirty folder goes with `--force`
    /// and the unmerged branch with `-D`, exactly as chosen.
    #[test]
    fn an_accepted_discard_removes_a_dirty_worktree_and_force_deletes_its_unmerged_branch() {
        let f = Fixture::new();
        let worktree = f.add("linked");
        commit(&worktree, "not merged anywhere");
        let mut request = confirmed(&f, &worktree, true);
        request.discard_changes = true;
        request.force_delete_branch = true;
        std::fs::write(worktree.join("tracked"), "dirty").unwrap();
        std::fs::write(worktree.join("untracked"), "new").unwrap();
        std::fs::create_dir_all(worktree.join("target/clone")).unwrap();
        git(&worktree.join("target/clone"), &["init", "-q"]).unwrap();
        request.expected_ignored_repositories = vec!["target/clone".to_owned()];

        let removed = remove_confirmed(&request).unwrap();
        assert_eq!(removed.branch, RemovalBranch::Deleted);
        assert!(!worktree.exists());
        assert!(!branch_exists(&f, "linked"));
    }

    /// Discard accepted but the branch deletion left safe: the folder goes,
    /// and the unmerged branch survives `-d` with Git's reason.
    #[test]
    fn an_accepted_discard_still_keeps_an_unmerged_branch_under_safe_deletion() {
        let f = Fixture::new();
        let worktree = f.add("linked");
        commit(&worktree, "not merged anywhere");
        let mut request = confirmed(&f, &worktree, true);
        request.discard_changes = true;
        std::fs::write(worktree.join("tracked"), "dirty").unwrap();

        let removed = remove_confirmed(&request).unwrap();
        assert_eq!(removed.branch, RemovalBranch::Remains);
        assert!(
            removed.message.contains("branch linked remains"),
            "{}",
            removed.message
        );
        assert!(!worktree.exists());
        assert!(branch_exists(&f, "linked"));
    }

    /// A worktree inside the one being deleted is lost with it, so only an
    /// accepted discard lets the removal through.
    #[test]
    fn a_worktree_inside_is_removed_only_with_an_accepted_discard() {
        let f = Fixture::new();
        let outer = f.add("outer");
        let inner = outer.join("inner");
        git(
            &f.main,
            &["worktree", "add", "--detach", inner.to_str().unwrap()],
        )
        .unwrap();
        let mut request = confirmed(&f, &outer, false);
        assert_eq!(
            remove_confirmed(&request).unwrap_err().stop,
            RemovalStop::Nested
        );
        assert!(inner.exists());

        request.discard_changes = true;
        remove_confirmed(&request).unwrap();
        assert!(!outer.exists());
    }

    #[test]
    fn a_confirmed_removal_of_a_missing_folder_clears_the_registration_and_keeps_the_branch() {
        let f = Fixture::new();
        let worktree = f.add("linked");
        let request = confirmed(&f, &worktree, false);
        std::fs::remove_dir_all(&worktree).unwrap();

        remove_confirmed(&request).unwrap();
        assert!(
            !git(&f.main, &["worktree", "list", "--porcelain"])
                .unwrap()
                .contains(&*worktree.to_string_lossy())
        );
        assert!(branch_exists(&f, "linked"));
    }

    #[test]
    fn a_confirmed_removal_deletes_a_merged_branch_only_with_safe_deletion() {
        let f = Fixture::new();
        let merged = f.add("merged");
        assert_eq!(
            remove_confirmed(&confirmed(&f, &merged, true))
                .unwrap()
                .branch,
            RemovalBranch::Deleted
        );
        assert!(!branch_exists(&f, "merged"));

        let ahead = f.add("ahead");
        commit(&ahead, "not merged anywhere");
        let removed = remove_confirmed(&confirmed(&f, &ahead, true)).unwrap();
        assert_eq!(removed.branch, RemovalBranch::Remains);
        assert!(!ahead.exists());
        assert!(branch_exists(&f, "ahead"));
    }

    #[test]
    fn a_confirmed_removal_stops_when_the_worktree_changed_after_confirmation() {
        let f = Fixture::new();
        let worktree = f.add("linked");
        let request = confirmed(&f, &worktree, true);
        commit(&worktree, "moved after confirmation");

        assert_eq!(
            remove_confirmed(&request).unwrap_err().stop,
            RemovalStop::IdentityChanged
        );
        assert!(worktree.exists());
        assert!(branch_exists(&f, "linked"));
    }

    #[test]
    fn a_confirmed_removal_refuses_the_main_checkout_and_the_protected_base() {
        let f = Fixture::new();
        let mut main = confirmed(&f, &f.main, false);
        main.expected_branch = Some("main".into());
        assert_eq!(remove_confirmed(&main).unwrap_err().stop, RemovalStop::Main);

        let worktree = f.add("base");
        let mut base = confirmed(&f, &worktree, false);
        base.protected_base_branch = Some("base".into());
        assert_eq!(
            remove_confirmed(&base).unwrap_err().stop,
            RemovalStop::HoldsBase
        );
        assert!(worktree.exists());

        let nested = f.root.join("base").join("inner");
        git(
            &f.main,
            &[
                "worktree",
                "add",
                "-b",
                "inner",
                nested.to_str().unwrap(),
                "main",
            ],
        )
        .unwrap();
        assert_eq!(
            remove_confirmed(&confirmed(&f, &worktree, false))
                .unwrap_err()
                .stop,
            RemovalStop::Nested
        );
        assert!(nested.exists());
    }

    #[test]
    fn a_stale_missing_pane_folder_does_not_block_unrelated_worktrees() {
        let f = Fixture::new();
        let clean = f.add("eligible");
        let stale = f.root.join("removed-worktree").join("pane");
        assert!(!stale.exists());

        let unrelated = f.review(
            &f.main,
            verified_pane_paths(
                &node(),
                vec![("p-stale".into(), Some(stale.to_string_lossy().into_owned()))],
            ),
        );
        assert_eq!(
            unrelated
                .rows
                .iter()
                .find(|row| Path::new(&row.path) == clean)
                .unwrap()
                .exclusion,
            None
        );

        let matching_stale = clean.join("deleted-pane-cwd");
        assert!(!matching_stale.exists());
        let in_use = f.review(
            &f.main,
            verified_pane_paths(
                &node(),
                vec![(
                    "p-stale".into(),
                    Some(matching_stale.to_string_lossy().into_owned()),
                )],
            ),
        );
        assert!(
            in_use
                .rows
                .iter()
                .find(|row| Path::new(&row.path) == clean)
                .unwrap()
                .exclusion
                .as_deref()
                .unwrap()
                .contains("not this checkout's own")
        );
    }

    fn code_of<'a>(review: &'a CleanupSnapshot, path: &Path) -> Option<&'a str> {
        review
            .rows
            .iter()
            .find(|row| Path::new(&row.path) == path)
            .unwrap()
            .exclusion_code
    }

    fn head_of(path: &Path) -> String {
        git(path, &["rev-parse", "HEAD"]).unwrap().trim().to_owned()
    }

    /// What `git fetch` leaves behind for a remote main, without a network.
    fn set_origin_main(f: &Fixture, oid: &str) {
        git(&f.main, &["update-ref", "refs/remotes/origin/main", oid]).unwrap();
    }

    fn proof(head_oid: &str, base_ref: &str) -> github::MergedPullRequestProof {
        github::MergedPullRequestProof {
            head_oid: head_oid.into(),
            base_ref: base_ref.into(),
        }
    }

    #[test]
    fn a_head_in_origin_main_is_chosen_while_local_main_lags_behind() {
        let f = Fixture::new();
        let landed = f.add("landed");
        let unlanded = f.add("unlanded");
        commit(&landed, "Landed on origin");
        commit(&unlanded, "Never landed");
        set_origin_main(&f, &head_of(&landed));

        let review = f.review(&f.main, Ok(Vec::new()));

        assert_eq!(code_of(&review, &landed), None);
        assert_eq!(code_of(&review, &unlanded), Some("not_merged"));
        assert_eq!(review.main_head, Some(head_of(&f.main)));
    }

    #[test]
    fn origin_main_decides_when_local_main_is_gone() {
        let f = Fixture::new();
        let landed = f.add("landed");
        let unlanded = f.add("unlanded");
        commit(&landed, "Landed on origin");
        commit(&unlanded, "Never landed");
        set_origin_main(&f, &head_of(&landed));
        git(&f.main, &["branch", "-m", "main", "trunk"]).unwrap();

        let review = f.review(&f.main, Ok(Vec::new()));

        assert_eq!(code_of(&review, &landed), None);
        assert_eq!(code_of(&review, &unlanded), Some("not_merged"));
    }

    #[test]
    fn with_neither_local_main_nor_origin_main_nothing_can_be_judged() {
        let f = Fixture::new();
        let worktree = f.add("worktree");
        commit(&worktree, "Merged on GitHub");
        git(&f.main, &["branch", "-m", "main", "trunk"]).unwrap();

        // Even a matching pull request cannot stand in for a base.
        let review = f.review_with_proofs(&f.main, vec![proof(&head_of(&worktree), "main")]);

        assert_eq!(code_of(&review, &worktree), Some("main_unavailable"));
        assert_eq!(review.main_head, None);
    }

    #[test]
    fn a_head_past_the_pull_request_list_is_asked_of_github_by_its_branch() {
        let window: BTreeSet<String> = ["recent".to_owned()].into();
        let asked = std::cell::Cell::new(0);
        let lookup = |answer: Result<BTreeSet<String>, String>| {
            let asked = &asked;
            move || {
                asked.set(asked.get() + 1);
                answer
            }
        };
        // In the list: no lookup.
        assert!(
            merge_proof(Some(&window), "recent", lookup(Ok(BTreeSet::new())))
                .unwrap()
                .contains("recent")
        );
        assert_eq!(asked.get(), 0);
        // Older than the list: the branch's own pull requests prove it.
        let old: BTreeSet<String> = ["old".to_owned()].into();
        assert!(
            merge_proof(Some(&window), "old", lookup(Ok(old)))
                .unwrap()
                .contains("old")
        );
        // A lookup that fails is no answer, so the worktree is unverified
        // rather than judged not merged.
        assert!(merge_proof(Some(&window), "old", lookup(Err("offline".into()))).is_err());
        // No list: the branch alone, and its failure stays a failure.
        assert!(merge_proof(None, "old", lookup(Err("offline".into()))).is_err());
        assert_eq!(asked.get(), 3);
    }

    #[test]
    fn a_repository_whose_default_branch_is_not_main_is_judged_against_that_branch() {
        let f = Fixture::new();
        let landed = f.add("landed");
        let unlanded = f.add("unlanded");
        let squashed = f.add("squashed");
        commit(&landed, "Landed on trunk");
        commit(&unlanded, "Never landed");
        commit(&squashed, "Squashed into trunk");
        git(&f.main, &["merge", "--ff-only", "landed"]).unwrap();
        git(&f.main, &["branch", "-m", "main", "trunk"]).unwrap();
        let proofs = vec![
            proof(&head_of(&squashed), "trunk"),
            proof(&head_of(&unlanded), "main"),
        ];

        let review = f.review_against(Some("trunk"), proofs.clone());
        assert_eq!(code_of(&review, &landed), None);
        assert_eq!(code_of(&review, &squashed), None);
        // A pull request into a branch that is not the default proves nothing.
        assert_eq!(code_of(&review, &unlanded), Some("not_merged"));
        assert_eq!(review.main_head, Some(head_of(&f.main)));

        // With no default branch known, nothing is judged merged, not even
        // a worktree GitHub answered for.
        let unknown = inspect_with_merge_proofs(
            &node(),
            &f.main,
            Some(&f.main),
            Ok(Vec::new()),
            &Usage::default(),
            None,
            None,
            |_, _| Ok(heads_into(proofs.clone(), "trunk")),
        )
        .unwrap();
        assert_eq!(code_of(&unknown, &landed), Some("main_unavailable"));
        assert_eq!(code_of(&unknown, &squashed), Some("main_unavailable"));
    }

    #[test]
    fn a_squash_merge_is_proved_by_a_main_pull_request_with_exactly_this_head() {
        let f = Fixture::new();
        let squashed = f.add("squashed");
        commit(&squashed, "Feature");
        let feature_head = head_of(&squashed);

        // The squash commit is in no local ref, and the proof names none.
        let matching = f.review_with_proofs(&f.main, vec![proof(&feature_head, "main")]);
        assert_eq!(code_of(&matching, &squashed), None);

        let reused_branch = f.review_with_proofs(&f.main, vec![proof("a-different-head", "main")]);
        assert_eq!(code_of(&reused_branch, &squashed), Some("not_merged"));

        let no_proof = f.review_with_proofs(&f.main, Vec::new());
        assert_eq!(code_of(&no_proof, &squashed), Some("not_merged"));

        let unavailable = inspect_with_merge_proofs(
            &node(),
            &f.main,
            Some(&f.main),
            Ok(Vec::new()),
            &Usage::default(),
            None,
            Some("main"),
            |_, _| Err("GitHub merge proof is unavailable".into()),
        )
        .unwrap();
        assert_eq!(code_of(&unavailable, &squashed), Some("unverified"));
    }

    #[test]
    fn a_pull_request_merged_into_another_branch_is_no_proof_until_that_branch_lands() {
        let f = Fixture::new();
        let stacked = f.add("stacked");
        commit(&stacked, "Stacked work");
        let stacked_head = head_of(&stacked);
        let proofs = vec![proof(&stacked_head, "ci/platform-completion")];

        let before = f.review_with_proofs(&f.main, proofs.clone());
        assert_eq!(code_of(&before, &stacked), Some("not_merged"));

        set_origin_main(&f, &stacked_head);
        let after = f.review_with_proofs(&f.main, proofs);
        assert_eq!(code_of(&after, &stacked), None);
    }

    #[test]
    fn removal_deletes_the_selected_worktree_with_its_build_output_and_keeps_the_branch() {
        let f = Fixture::new();
        let clean = f.add("with-cache");
        std::fs::create_dir_all(clean.join("target/debug")).unwrap();
        std::fs::write(clean.join("target/debug/artifact"), "regenerable").unwrap();
        let review = f.review(&f.main, Ok(Vec::new()));
        let selected = vec![clean.to_string_lossy().into_owned()];

        let result = confirm(
            &review,
            &selected,
            |_| Ok(f.review(&f.main, Ok(Vec::new()))),
            |_, _| panic!("a worktree with no pane closes none"),
            |path| hide_host::worktrees::remove_worktree(&f.main, Path::new(path), false),
            |_| {},
            |path| Ok(Path::new(path).exists()),
        );

        assert!(!clean.exists());
        assert_eq!(
            result
                .rows
                .iter()
                .find(|row| row.path == selected[0])
                .and_then(|row| row.result.as_deref()),
            Some("removed")
        );
        assert!(git(&f.main, &["show-ref", "--verify", "refs/heads/with-cache"]).is_ok());
    }

    #[test]
    fn review_and_cancel_preserve_files_and_explain_every_exclusion() {
        let f = Fixture::new();
        let clean = f.add("clean");
        let dirty = f.add("dirty");
        let untracked = f.add("untracked");
        let busy = f.add("busy");
        let current = f.add("current");
        let ahead = f.add("ahead");
        let locked = f.add("locked");
        std::fs::write(dirty.join("tracked"), "changed").unwrap();
        std::fs::write(untracked.join("new"), "keep").unwrap();
        std::fs::create_dir(busy.join("subdir")).unwrap();
        git(&f.main, &["worktree", "lock", locked.to_str().unwrap()]).unwrap();
        git(
            &ahead,
            &[
                "-c",
                "commit.gpgsign=false",
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "--allow-empty",
                "-m",
                "Unmerged",
            ],
        )
        .unwrap();
        let before = git(&f.main, &["worktree", "list", "--porcelain", "-z"]).unwrap();
        let review = f.review(&current, Ok(vec![("p-stray".into(), busy.join("subdir"))]));
        let row = |path: &Path| {
            review
                .rows
                .iter()
                .find(|r| Path::new(&r.path) == path)
                .unwrap()
        };
        assert!(row(&clean).exclusion.is_none());
        for (path, reason) in [
            (&f.main, "Main"),
            (&dirty, "Uncommitted"),
            (&untracked, "Uncommitted"),
            (&busy, "not this checkout's own"),
            (&current, "Current"),
            (&ahead, "Not merged"),
            (&locked, "locked"),
        ] {
            assert!(row(path).exclusion.as_ref().unwrap().contains(reason));
        }
        let cancelled = confirm(
            &review,
            &[],
            |_| panic!("cancel must not inspect"),
            |_, _| panic!("cancel must not close a pane"),
            |_| panic!("cancel must not delete"),
            |_| {},
            |path| Ok(Path::new(path).exists()),
        );
        assert!(cancelled.rows.iter().all(|r| r.result.is_none()));
        assert_eq!(
            git(&f.main, &["worktree", "list", "--porcelain", "-z"]).unwrap(),
            before
        );
        for path in [&clean, &dirty, &untracked, &busy, &current, &ahead, &locked] {
            assert!(path.join("tracked").exists());
        }
        let unknown = f.review(&current, Err("Usage unavailable".into()));
        assert!(unknown.rows.iter().all(|r| r.exclusion.is_some()));
    }

    #[test]
    fn confirm_rechecks_stale_state_and_converges_partial_success_without_repeating_removal() {
        let f = Fixture::new();
        let clean = f.add("merged");
        let changed = f.add("changed");
        let failed = f.add("failed");
        let review = f.review(&f.main, Ok(vec![]));
        let paths: Vec<_> = [&clean, &changed, &failed]
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        std::fs::write(changed.join("new"), "arrived after review").unwrap();
        let mut effects = Vec::new();
        let result = confirm(
            &review,
            &paths,
            |_| Ok(f.review(&f.main, Ok(vec![]))),
            |_, _| panic!("a worktree with no pane closes none"),
            |path| {
                effects.push(path.to_owned());
                if Path::new(path) == failed {
                    return Err("Git refused removal".into());
                }
                git(&f.main, &["worktree", "remove", "--", path])
                    .map(|_| "Worktree removed".to_owned())
            },
            |_| {},
            |path| Ok(Path::new(path).exists()),
        );
        assert!(!clean.exists());
        assert!(changed.join("new").exists());
        assert!(failed.exists());
        assert_eq!(effects.len(), 2, "stale target never reached remove");
        assert_eq!(
            result
                .rows
                .iter()
                .filter(|r| r.result.as_deref() == Some("removed"))
                .count(),
            1
        );
        assert_eq!(
            result
                .rows
                .iter()
                .filter(|r| r.result.as_deref() == Some("skipped"))
                .count(),
            1,
            "the worktree that changed after the review is skipped, by its code"
        );
        assert_eq!(
            result
                .rows
                .iter()
                .filter(|r| r.result.as_deref() == Some("failed"))
                .map(|r| r.result_code)
                .collect::<Vec<_>>(),
            [Some("remove_refused")],
            "a removal Git refuses is a failure, not a recheck"
        );
        let retry = confirm(
            &result,
            &paths,
            |_| Ok(f.review(&f.main, Ok(vec![]))),
            |_, _| panic!("a worktree with no pane closes none"),
            |path| {
                assert_ne!(Path::new(path), clean);
                git(&f.main, &["worktree", "remove", "--", path])
                    .map(|_| "Worktree removed".to_owned())
            },
            |_| {},
            |path| Ok(Path::new(path).exists()),
        );
        assert_eq!(
            retry
                .rows
                .iter()
                .filter(|r| r.result.as_deref() == Some("removed"))
                .count(),
            2
        );
        assert!(git(&f.main, &["show-ref", "--verify", "refs/heads/merged"]).is_ok());
        assert_eq!(
            std::fs::read_to_string(f.main.join("tracked")).unwrap(),
            "keep"
        );
    }

    fn facts(path: &str, working: usize, panes: &[&str]) -> CheckoutFacts {
        CheckoutFacts {
            path: PathBuf::from(path),
            agent_working: working,
            terminal_panes: panes.iter().map(|pane| (*pane).to_owned()).collect(),
            ..Default::default()
        }
    }

    fn listener(port: u16, cwd: &str) -> ListeningPortSnapshot {
        ListeningPortSnapshot {
            host: "127.0.0.1".into(),
            port,
            cwd: cwd.to_owned(),
        }
    }

    #[test]
    fn a_checkout_is_in_use_by_a_working_agent_a_running_program_or_its_own_server() {
        let checkouts = [
            facts("/fixture/agent", 1, &["p-agent"]),
            facts("/fixture/build", 0, &["p-idle", "p-cargo"]),
            facts("/fixture/server", 0, &[]),
            facts("/fixture/idle", 0, &["p-idle"]),
            // A neighbour whose name only starts the same way is not this checkout.
            facts("/fixture/idle-two", 0, &[]),
        ];
        let in_use = read_in_use(
            &node(),
            &checkouts,
            Ok(vec![
                listener(5173, "/fixture/server/web"),
                listener(8080, "/fixture/idle-two-staging"),
            ]),
            |pane| {
                assert_ne!(pane, "p-agent", "a checkout already known busy is not read");
                Ok((pane == "p-cargo").then(|| "cargo".to_owned()))
            },
        )
        .unwrap();
        assert_eq!(in_use[Path::new("/fixture/agent")].code, "agent_working");
        let build = &in_use[Path::new("/fixture/build")];
        assert_eq!(
            (build.code, build.name.as_deref()),
            ("process", Some("cargo"))
        );
        let server = &in_use[Path::new("/fixture/server")];
        assert_eq!((server.code, server.port), ("port", Some(5173)));
        assert!(!in_use.contains_key(Path::new("/fixture/idle")));
        assert!(!in_use.contains_key(Path::new("/fixture/idle-two")));
    }

    #[test]
    fn an_unreadable_pane_or_port_list_fails_the_whole_answer_instead_of_reading_idle() {
        let checkouts = [facts("/fixture/a", 0, &["p1"])];
        assert!(
            read_in_use(&node(), &checkouts, Err("lsof is missing".into()), |_| Ok(
                None
            ))
            .is_err()
        );
        assert!(
            read_in_use(&node(), &checkouts, Ok(Vec::new()), |_| Err(
                "pane.process_info failed".into()
            ))
            .is_err()
        );
        let many: Vec<_> = (0..=PANE_READ_LIMIT).map(|n| format!("p{n}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        let error = read_in_use(
            &node(),
            &[facts("/fixture/b", 0, &many)],
            Ok(Vec::new()),
            |_| Ok(None),
        )
        .unwrap_err();
        assert!(error.contains("too many"), "{error}");
    }

    fn waiting(path: &str) -> CheckoutFacts {
        CheckoutFacts {
            agent_waiting: 1,
            ..facts(path, 0, &[])
        }
    }

    #[test]
    fn an_agent_is_quiet_only_when_it_is_stopped_and_asks_for_nothing() {
        for (demand, blocked, activity, expected) in [
            ("none", false, "stopped", AgentUse::Quiet),
            ("none", false, "working", AgentUse::Working),
            ("question", false, "stopped", AgentUse::Waiting),
            ("approval", false, "working", AgentUse::Waiting),
            ("none", true, "stopped", AgentUse::Waiting),
            ("none", false, "unknown", AgentUse::Unknown),
        ] {
            assert_eq!(
                AgentUse::of(demand, blocked, activity),
                expected,
                "{demand} {blocked} {activity}"
            );
        }
    }

    #[test]
    fn finished_agents_and_shells_leave_a_checkout_free_and_waiting_work_does_not() {
        let checkouts = [
            // A finished agent counts in no field, and its pane is a pane to close.
            CheckoutFacts {
                panes: vec!["p-agent".into(), "p-shell".into()],
                ..facts("/fixture/finished", 0, &["p-shell"])
            },
            waiting("/fixture/blocked"),
            CheckoutFacts {
                descendants_busy: 1,
                ..facts("/fixture/parent", 0, &[])
            },
            CheckoutFacts {
                agent_unknown: 1,
                ..facts("/fixture/unknown", 0, &[])
            },
            facts("/fixture/editor", 0, &["p-vim"]),
        ];
        let in_use = read_in_use(&node(), &checkouts, Ok(Vec::new()), |pane| {
            Ok((pane == "p-vim").then(|| "vim".to_owned()))
        })
        .unwrap();
        assert!(!in_use.contains_key(Path::new("/fixture/finished")));
        for (path, code) in [
            ("/fixture/blocked", "agent_waiting"),
            ("/fixture/parent", "descendant_busy"),
            ("/fixture/unknown", "unverified"),
            ("/fixture/editor", "process"),
        ] {
            assert_eq!(in_use[Path::new(path)].code, code, "{path}");
        }
    }

    #[test]
    fn a_worktree_with_only_its_own_finished_panes_is_chosen_and_a_stranger_pane_is_not() {
        let f = Fixture::new();
        let finished = f.add("finished");
        let stranger = f.add("stranger");
        let usage = Usage::of(
            InUseMap::new(),
            &[
                CheckoutFacts {
                    panes: vec!["p-agent".into(), "p-shell".into()],
                    ..facts(finished.to_str().unwrap(), 0, &["p-shell"])
                },
                facts(stranger.to_str().unwrap(), 0, &[]),
            ],
        );
        let panes = Ok(vec![
            ("p-agent".to_owned(), finished.clone()),
            ("p-shell".to_owned(), finished.join("src")),
            ("p-elsewhere".to_owned(), stranger.join("src")),
        ]);
        let review = inspect_with_merge_proofs(
            &node(),
            &f.main,
            Some(&f.main),
            panes,
            &usage,
            None,
            Some("main"),
            |_, _| Ok(BTreeSet::new()),
        )
        .unwrap();
        let row = |path: &Path| {
            review
                .rows
                .iter()
                .find(|r| Path::new(&r.path) == path)
                .unwrap()
        };
        assert_eq!(row(&finished).exclusion_code, None);
        assert_eq!(row(&finished).pane_count, 2);
        assert_eq!(row(&finished).pane_ids, ["p-agent", "p-shell"]);
        assert_eq!(row(&stranger).exclusion_code, Some("pane_open"));
        assert_eq!(row(&stranger).pane_count, 0);
    }

    #[test]
    fn a_confirmed_worktree_closes_its_panes_before_it_is_removed_and_keeps_them_when_one_will_not_close()
     {
        let f = Fixture::new();
        let closes = f.add("closes");
        let stuck = f.add("stuck");
        let review = f.review(&f.main, Ok(Vec::new()));
        let fresh = |panes: &'static [&'static str]| {
            let mut snapshot = f.review(&f.main, Ok(Vec::new()));
            for row in &mut snapshot.rows {
                row.pane_ids = panes.iter().map(|id| (*id).to_owned()).collect();
                row.pane_count = u32::try_from(panes.len()).unwrap();
            }
            snapshot
        };
        let selected: Vec<_> = [&closes, &stuck]
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        let steps = std::cell::RefCell::new(Vec::new());
        let result = confirm(
            &review,
            &selected,
            |_| Ok(fresh(&["p1", "p2"])),
            |path, panes| {
                steps
                    .borrow_mut()
                    .push(format!("close {} {}", panes.len(), path));
                if Path::new(path) == stuck {
                    return Err(Refusal::failed(
                        "close_refused",
                        "Timed out waiting for Herdr to confirm closed panes",
                    ));
                }
                Ok(())
            },
            |path| {
                steps.borrow_mut().push(format!("remove {path}"));
                git(&f.main, &["worktree", "remove", "--", path]).map(|_| "Worktree removed".into())
            },
            |_| {},
            |path| Ok(Path::new(path).exists()),
        );
        let outcome = |path: &Path| {
            let row = result
                .rows
                .iter()
                .find(|r| Path::new(&r.path) == path)
                .unwrap();
            (row.result.as_deref(), row.result_code)
        };
        assert_eq!(outcome(&closes), (Some("removed"), None));
        assert_eq!(outcome(&stuck), (Some("failed"), Some("close_refused")));
        assert!(!closes.exists());
        assert!(stuck.exists(), "a worktree whose pane stays is not removed");
        let steps = steps.into_inner();
        let at = |what: String| steps.iter().position(|step| *step == what).unwrap();
        let closes_path = closes.to_string_lossy();
        assert!(
            at(format!("close 2 {closes_path}")) < at(format!("remove {closes_path}")),
            "panes close before the folder goes"
        );
        assert!(
            !steps
                .iter()
                .any(|step| step.starts_with("remove") && step.ends_with("stuck")),
            "{steps:?}"
        );
    }

    #[test]
    fn panes_close_only_when_they_are_the_reviewed_ones_and_nothing_else_stands_in_the_folder() {
        let folder = Path::new("/fixture/finished");
        let panes = |ids: &[&str]| ids.iter().map(|id| (*id).to_owned()).collect::<Vec<_>>();
        let live = |ids: &[(&str, &str)]| -> PanePaths {
            ids.iter()
                .map(|(id, cwd)| ((*id).to_owned(), PathBuf::from(cwd)))
                .collect()
        };
        let stands = live(&[("p1", "/fixture/finished"), ("p2", "/fixture/finished/src")]);
        assert_eq!(
            panes_to_close(
                folder,
                &panes(&["p2", "p1"]),
                &panes(&["p1", "p2"]),
                &stands
            )
            .ok(),
            Some(panes(&["p1", "p2"]))
        );
        // A pane the operator already closed needs no closing and is not a failure.
        assert_eq!(
            panes_to_close(
                folder,
                &panes(&["p1", "p2"]),
                &panes(&["p1", "p2"]),
                &live(&[("p1", "/fixture/finished")])
            )
            .ok(),
            Some(panes(&["p1"]))
        );
        let refused = |reviewed: &[&str], facts: &[&str], live: &PanePaths| {
            panes_to_close(folder, &panes(reviewed), &panes(facts), live)
                .err()
                .map(|refusal| (refusal.outcome, refusal.code))
        };
        // A pane opened since the review, in the core's view or only in Herdr's.
        assert_eq!(
            refused(&["p1"], &["p1", "p9"], &stands),
            Some(("skipped", "changed"))
        );
        assert_eq!(
            refused(
                &["p1", "p2"],
                &["p1", "p2"],
                &live(&[("p1", "/fixture/finished"), ("p3", "/fixture/finished/new")])
            ),
            Some(("skipped", "pane_open"))
        );
    }

    #[test]
    fn every_reason_a_worktree_cannot_be_removed_carries_a_code_and_a_count_where_one_helps() {
        let f = Fixture::new();
        let clean = f.add("clean");
        let dirty = f.add("dirty");
        let busy = f.add("busy");
        let nested = f.add("nested");
        let ahead = f.add("ahead");
        let locked = f.add("locked");
        let current = f.add("current");
        std::fs::write(dirty.join("tracked"), "changed").unwrap();
        std::fs::write(dirty.join("new"), "untracked").unwrap();
        std::fs::create_dir_all(nested.join("target/clone/.git")).unwrap();
        commit(&ahead, "Unmerged");
        git(&f.main, &["worktree", "lock", locked.to_str().unwrap()]).unwrap();
        let mut in_use = InUseMap::new();
        in_use.insert(
            busy.clone(),
            InUse {
                code: "process",
                name: Some("cargo".into()),
                port: None,
            },
        );
        let review = inspect_with_merge_proofs(
            &node(),
            &f.main,
            Some(&current),
            Ok(Vec::new()),
            &Usage::of(in_use, &[]),
            None,
            Some("main"),
            |_, _| Ok(BTreeSet::new()),
        )
        .unwrap();
        let row = |path: &Path| {
            review
                .rows
                .iter()
                .find(|r| Path::new(&r.path) == path)
                .unwrap()
        };
        assert_eq!(row(&clean).exclusion_code, None);
        assert!(row(&f.main).is_main);
        for (path, code) in [
            (&f.main, "main"),
            (&dirty, "dirty"),
            (&busy, "in_use"),
            (&nested, "nested_repository"),
            (&ahead, "not_merged"),
            (&locked, "locked"),
            (&current, "current"),
        ] {
            assert_eq!(row(path).exclusion_code, Some(code), "{}", path.display());
        }
        assert_eq!(row(&dirty).exclusion_count, Some(2));
        assert_eq!(
            row(&busy).in_use.as_ref().unwrap().name.as_deref(),
            Some("cargo")
        );
        assert_eq!(row(&f.main).in_use, None, "an idle main carries no reason");
        assert!(review.usage_ready);
    }

    /// A built checkout: a Rust and a Node package, each with folders a
    /// tool makes again, all ignored, next to tracked source.
    struct Built {
        f: Fixture,
        folders: HashMap<PathBuf, Vec<LayerFolder>>,
    }

    impl Built {
        fn new() -> Self {
            let f = Fixture::new();
            let main = f.main.clone();
            std::fs::write(main.join(".gitignore"), "target/\nnode_modules/\ndist/\n").unwrap();
            std::fs::write(main.join("Cargo.toml"), "[package]\n").unwrap();
            std::fs::write(main.join("package.json"), "{}\n").unwrap();
            std::fs::create_dir_all(main.join("target/debug")).unwrap();
            std::fs::write(main.join("target/debug/artifact"), vec![1u8; 20_000]).unwrap();
            std::fs::write(
                main.join("target/CACHEDIR.TAG"),
                "Signature: 8a477f597d28d172789f06886806bc55\n",
            )
            .unwrap();
            std::fs::create_dir_all(main.join("node_modules/dep")).unwrap();
            std::fs::write(main.join("node_modules/dep/index.js"), vec![2u8; 8_000]).unwrap();
            std::fs::create_dir_all(main.join("dist")).unwrap();
            std::fs::write(main.join("dist/app.js"), vec![3u8; 4_000]).unwrap();
            git(&main, &["add", ".gitignore", "Cargo.toml", "package.json"]).unwrap();
            commit(&main, "Manifests");
            let measured = disk::read(
                &hide_node::Local::of_process(),
                &disk::DiskRequest {
                    paths: vec![main.clone()],
                    ..Default::default()
                },
            );
            let folders = HashMap::from([(main, measured[0].folders.clone())]);
            Self { f, folders }
        }

        fn common(&self) -> PathBuf {
            self.f.main.join(".git")
        }

        fn run(&self, in_use: Result<&InUseMap, &str>, cells: &[(Layer,)]) -> Vec<CellResult> {
            self.run_with(
                |path| match in_use {
                    Err(message) => Err(message.to_owned()),
                    Ok(map) => Ok(map.get(path).cloned()),
                },
                cells,
            )
        }

        fn run_with(
            &self,
            in_use_now: impl FnMut(&Path) -> Result<Option<InUse>, String>,
            cells: &[(Layer,)],
        ) -> Vec<CellResult> {
            let node = node();
            let run = CellRun {
                node: &node,
                id: 1,
                workspace_id: "w1",
                common: &self.common(),
                folders: &self.folders,
            };
            let cells: Vec<_> = cells
                .iter()
                .map(|(layer,)| CellChoice {
                    path: self.f.main.to_string_lossy().into_owned(),
                    layer: *layer,
                })
                .collect();
            let mut moved = std::collections::BTreeSet::new();
            let results = empty_cells(&run, &cells, in_use_now, &mut moved, |_| {});
            assert!(moved.iter().all(|entry| entry.starts_with(self.common())));
            results
        }
    }

    /// What a forced add leaves: a file Git tracks inside an ignored folder.
    /// (Written without the force flag, which this file never spells.)
    fn track_in_ignored(main: &Path, file: &str) {
        let ignore = std::fs::read_to_string(main.join(".gitignore")).unwrap();
        std::fs::write(main.join(".gitignore"), "").unwrap();
        git(main, &["add", file]).unwrap();
        std::fs::write(main.join(".gitignore"), ignore).unwrap();
    }

    fn outcome(results: &[CellResult], layer: Layer) -> (&'static str, Option<&'static str>) {
        let result = results.iter().find(|r| r.layer == layer.code()).unwrap();
        (result.outcome, result.reason_code)
    }

    #[test]
    fn a_chosen_cell_leaves_its_checkout_at_once_and_the_trash_deletes_it() {
        let built = Built::new();
        let main = &built.f.main;
        let results = built.run(
            Ok(&InUseMap::new()),
            &[(Layer::BuildCache,), (Layer::Dependencies,)],
        );
        let cache = results.iter().find(|r| r.layer == "build_cache").unwrap();
        // `target` and `dist` are both build cache.
        assert_eq!((cache.outcome, cache.folders), ("removed", 2));
        assert!(cache.bytes >= 24_000);
        assert_eq!(outcome(&results, Layer::Dependencies), ("removed", None));
        for gone in ["target", "node_modules", "dist"] {
            assert!(!main.join(gone).exists(), "{gone} left the checkout");
        }
        for kept in ["tracked", "Cargo.toml", "package.json", ".gitignore"] {
            assert!(main.join(kept).exists(), "{kept} is source");
        }
        assert_eq!(
            hide_host::worktrees::drain_trash(
                &built.common(),
                &hide_host::worktrees::trash_entries(&built.common()),
                std::time::Duration::from_secs(30),
            ),
            0
        );
        assert_eq!(
            std::fs::read_dir(built.common().join("hide-removed"))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn a_checkout_that_became_busy_since_the_review_or_cannot_be_read_loses_nothing() {
        let built = Built::new();
        let busy = InUseMap::from([(
            built.f.main.clone(),
            InUse {
                code: "agent_working",
                name: None,
                port: None,
            },
        )]);
        let results = built.run(Ok(&busy), &[(Layer::BuildCache,)]);
        assert_eq!(
            outcome(&results, Layer::BuildCache),
            ("skipped", Some("in_use"))
        );
        let results = built.run(Err("pane.process_info failed"), &[(Layer::Dependencies,)]);
        assert_eq!(
            outcome(&results, Layer::Dependencies),
            ("skipped", Some("unverified"))
        );
        for kept in ["target", "node_modules", "dist"] {
            assert!(built.f.main.join(kept).exists());
        }
    }

    #[test]
    fn a_folder_that_changed_after_the_measurement_is_kept_and_named() {
        let built = Built::new();
        let main = &built.f.main;
        // A link swapped in for the folder: what it points at is never touched.
        let outside = built.f.root.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("precious"), "keep").unwrap();
        std::fs::remove_dir_all(main.join("node_modules")).unwrap();
        hide_platform::fs::link::create_link(&outside, &main.join("node_modules")).unwrap();
        // A file Git tracks appeared in an ignored folder.
        std::fs::write(main.join("dist/keep.txt"), "tracked on purpose").unwrap();
        track_in_ignored(main, "dist/keep.txt");
        // Another repository was cloned into the build folder.
        std::fs::create_dir_all(main.join("target/vendored/.git")).unwrap();

        let results = built.run(
            Ok(&InUseMap::new()),
            &[(Layer::BuildCache,), (Layer::Dependencies,)],
        );
        assert_eq!(
            outcome(&results, Layer::Dependencies),
            ("skipped", Some("symlink"))
        );
        // The build-cache cell had two folders: none could move.
        assert_eq!(outcome(&results, Layer::BuildCache).0, "skipped");
        assert!(matches!(
            outcome(&results, Layer::BuildCache).1,
            Some("tracked_files" | "nested_repository")
        ));
        assert_eq!(
            std::fs::read_to_string(outside.join("precious")).unwrap(),
            "keep"
        );
        assert!(main.join("dist/keep.txt").exists());
        assert!(main.join("target/vendored/.git").exists());
    }

    #[test]
    fn one_folder_kept_does_not_stop_its_cell_and_the_reason_stays_on_the_result() {
        let built = Built::new();
        let main = &built.f.main;
        std::fs::write(main.join("dist/keep.txt"), "tracked on purpose").unwrap();
        track_in_ignored(main, "dist/keep.txt");
        let results = built.run(Ok(&InUseMap::new()), &[(Layer::BuildCache,)]);
        let cache = &results[0];
        assert_eq!((cache.outcome, cache.folders), ("removed", 1));
        assert_eq!(cache.reason_code, Some("tracked_files"));
        assert!(!main.join("target").exists());
        assert!(main.join("dist/keep.txt").exists());
    }

    #[test]
    fn repeating_a_confirmed_cell_removes_nothing_a_second_time() {
        let built = Built::new();
        let first = built.run(Ok(&InUseMap::new()), &[(Layer::Dependencies,)]);
        assert_eq!(outcome(&first, Layer::Dependencies), ("removed", None));
        let again = built.run(Ok(&InUseMap::new()), &[(Layer::Dependencies,)]);
        assert_eq!(
            outcome(&again, Layer::Dependencies),
            ("skipped", Some("not_found"))
        );
        assert_eq!(again[0].bytes, 0);
    }

    #[test]
    fn a_checkouts_folders_are_asked_of_git_once() {
        let built = Built::new();
        built.run(
            Ok(&InUseMap::new()),
            &[(Layer::BuildCache,), (Layer::Dependencies,)],
        );
        let asked = hide_host::worktrees::git_calls(&built.f.main, "ls-files");
        assert_eq!(asked, 1, "three folders, one ls-files");
    }

    fn ls_files_asked(root: &Path) -> usize {
        hide_host::worktrees::git_calls(root, "ls-files")
    }

    #[test]
    fn a_registered_worktree_the_runtime_has_no_facts_about_reads_unverified_never_idle() {
        let known = [facts("/fixture/known", 0, &[])];
        let mut in_use = InUseMap::new();
        mark_unverified(
            &node(),
            &mut in_use,
            &known,
            vec![
                PathBuf::from("/fixture/known"),
                PathBuf::from("/fixture/unknown"),
            ],
        )
        .unwrap();
        assert!(!in_use.contains_key(Path::new("/fixture/known")));
        assert_eq!(in_use[Path::new("/fixture/unknown")].code, "unverified");

        // The fresh read behaves the same way, and a project that is gone is
        // an error, not an idle answer.
        let read = |_: &[CheckoutFacts]| Ok(InUseMap::new());
        let now = read_in_use_now(
            &node(),
            Path::new("/fixture/unknown"),
            || Some(vec![facts("/fixture/known", 0, &[])]),
            read,
        )
        .unwrap();
        assert_eq!(now.0.map(|use_| use_.code), Some("unverified"));
        let now = read_in_use_now(
            &node(),
            Path::new("/fixture/known"),
            || Some(vec![facts("/fixture/known", 0, &[])]),
            read,
        )
        .unwrap();
        assert_eq!(now.0, None);
        assert!(read_in_use_now(&node(), Path::new("/fixture/known"), || None, read).is_err());
    }

    #[test]
    fn a_checkout_that_turns_busy_after_the_folders_were_judged_loses_nothing() {
        let built = Built::new();
        let root = built.f.main.clone();
        let asked_before = ls_files_asked(&root);
        let mut looked = 0;
        let results = built.run_with(
            |_| {
                looked += 1;
                // Every judgement is done, Git has been asked, and nothing
                // has moved yet.
                assert_eq!(ls_files_asked(&root), asked_before + 1);
                for folder in ["target", "node_modules", "dist"] {
                    assert!(root.join(folder).exists());
                }
                Ok(Some(InUse {
                    code: "process",
                    name: Some("cargo".into()),
                    port: None,
                }))
            },
            &[(Layer::BuildCache,), (Layer::Dependencies,)],
        );
        assert_eq!(looked, 1, "one look per checkout, right before the moves");
        for layer in [Layer::BuildCache, Layer::Dependencies] {
            assert_eq!(outcome(&results, layer), ("skipped", Some("in_use")));
        }
        for folder in ["target", "node_modules", "dist"] {
            assert!(root.join(folder).exists(), "{folder} stayed");
        }
    }

    #[test]
    fn a_folder_the_index_spells_differently_is_still_found_tracked() {
        let f = Fixture::new();
        let main = &f.main;
        // The index holds `Dist/keep.txt` and `한글/keep.txt`; the file
        // system, and so the measurement, may hand back another spelling.
        std::fs::create_dir_all(main.join("Dist")).unwrap();
        std::fs::write(main.join("Dist/keep.txt"), "x").unwrap();
        let composed = "\u{d55c}\u{ae00}";
        std::fs::create_dir_all(main.join(composed)).unwrap();
        std::fs::write(main.join(composed).join("keep.txt"), "x").unwrap();
        git(main, &["add", "Dist/keep.txt"]).unwrap();
        git(main, &["add", "--", &format!("{composed}/keep.txt")]).unwrap();
        let decomposed = "\u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af}";
        let candidates = [
            main.join("dist"),
            main.join(decomposed),
            main.join("elsewhere"),
            main.join("Dist"),
        ];
        let tracked =
            tracked_folders(&node(), main, candidates.iter().map(PathBuf::as_path)).unwrap();
        assert_eq!(
            tracked,
            [main.join("dist"), main.join(decomposed), main.join("Dist")]
        );
    }

    #[test]
    fn thousands_of_folders_cost_one_ls_files_and_no_argument_list() {
        let f = Fixture::new();
        let main = &f.main;
        let folders: Vec<PathBuf> = (0..20_000)
            .map(|n| main.join(format!("package-{n}/node_modules")))
            .collect();
        let asked_before = ls_files_asked(main);
        let tracked = tracked_folders(&node(), main, folders.iter().map(PathBuf::as_path)).unwrap();
        assert!(tracked.is_empty());
        assert_eq!(ls_files_asked(main), asked_before + 1);
        // `tracked` is a file at the top of the fixture, not a folder below
        // a candidate that merely starts the same way.
        let near = [main.join("track"), main.join("tracked")];
        let tracked = tracked_folders(&node(), main, near.iter().map(PathBuf::as_path)).unwrap();
        assert!(tracked.is_empty());
    }

    type Published = std::sync::Arc<std::sync::Mutex<Vec<CleanupSnapshot>>>;

    fn guard(
        terminal: &'static str,
    ) -> (
        WorkerGuard,
        Published,
        std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        let published = Published::default();
        let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (sink, flag) = (published.clone(), finished.clone());
        let guard = WorkerGuard::new(
            Box::new(move |snapshot| sink.lock().unwrap().push(snapshot)),
            Box::new(move || flag.store(true, Ordering::SeqCst)),
            CleanupSnapshot {
                id: 4,
                phase: "removing".into(),
                ..Default::default()
            },
            terminal,
        );
        (guard, published, finished)
    }

    #[test]
    fn a_worker_that_panics_leaves_a_terminal_answer_and_frees_the_lane() {
        let (guard, published, finished) = guard("complete");
        let stopped = std::thread::spawn(move || {
            let mut guard = guard;
            guard.publish(CleanupSnapshot {
                id: 4,
                phase: "removing".into(),
                progress: Some(CleanupProgress { done: 1, total: 3 }),
                ..Default::default()
            });
            panic!("the worker died mid-run");
        })
        .join();
        assert!(stopped.is_err());
        let published = published.lock().unwrap();
        let last = published.last().unwrap();
        assert_eq!((last.id, last.phase.as_str()), (4, "complete"));
        assert_eq!(last.progress, Some(CleanupProgress { done: 1, total: 3 }));
        assert!(last.message.is_some());
        assert!(finished.load(Ordering::SeqCst));
    }

    #[test]
    fn a_worker_that_finishes_publishes_only_its_own_answer_and_frees_the_lane() {
        let (guard, published, finished) = guard("failed");
        guard.finish(CleanupSnapshot {
            id: 4,
            phase: "review".into(),
            ..Default::default()
        });
        let published = published.lock().unwrap();
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].phase, "review");
        assert!(finished.load(Ordering::SeqCst));
    }

    #[test]
    fn the_shell_reads_codes_and_sizes_and_never_the_words_meant_for_the_log() {
        let snapshot = CleanupSnapshot {
            rows: vec![CleanupRow {
                path: "/w/a".into(),
                exclusion: Some("Git says the branch is not merged".into()),
                exclusion_code: Some("not_merged"),
                result: Some("skipped".into()),
                result_code: Some("changed"),
                bytes: Some(4096),
                message: Some("The worktree changed after the review".into()),
                ..Default::default()
            }],
            cell_results: vec![CellResult {
                path: "/w/a".into(),
                layer: "build_cache",
                outcome: "skipped",
                bytes: 0,
                folders: 0,
                reason_code: Some("tracked_files"),
                reason: Some("Git tracks a file inside the folder".into()),
            }],
            ..Default::default()
        };
        let wire = serde_json::to_value(&snapshot).unwrap();
        let row = wire["rows"][0].as_object().unwrap();
        assert!(!row.contains_key("exclusion") && !row.contains_key("message"));
        assert_eq!(row["exclusion_code"], "not_merged");
        assert_eq!(
            (&row["result"], &row["result_code"]),
            (&"skipped".into(), &"changed".into())
        );
        assert_eq!(row["bytes"], 4096);
        let cell = wire["cell_results"][0].as_object().unwrap();
        assert!(!cell.contains_key("reason"));
        assert_eq!(cell["reason_code"], "tracked_files");
    }
}
