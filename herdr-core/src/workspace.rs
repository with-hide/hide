//! Persistent workspace and checkout discovery for the hide navigator.
//!
//! A workspace registration is metadata only, while checkout discovery is
//! rebuilt on launch and after a session update from what the core's own node
//! says about the paths involved (`PathIndex`). The core reads no folder: the
//! node answers from the repositories' own files, never by running git, and
//! the sync coordinator asks it before taking the runtime lock. Removing a
//! registration therefore cannot remove a checkout or terminate a remote
//! process.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use hide_node_link::catalog::{BranchNote, PATH_FACTS_LIMIT, PathFact, PathFacts, RepositoryPlace};
use hide_node_link::protocol::Call;
use hide_platform::path;

use crate::model::{
    CheckoutPurposeOrigin, CheckoutPurposeSnapshot, CheckoutSnapshot, DeviceSnapshot, TabSnapshot,
    WorkspaceRegistration, WorkspaceSnapshot, WorktreeCatalogSnapshot,
};
use crate::node::NodeId;
use crate::node_access::{NodeLink, call_as};

/// The row for the machine the core runs on. Its id is the node id; its
/// `kind` stays `local`, the role "the core's own machine", not a name; its
/// `label` is the name the operator registered it under before the core
/// moved to it (`UiStateSnapshot::machine_label`), which a window on another
/// machine names it by, and none otherwise, since a window on this machine
/// names it as its system does from `host.platform`; and `machine_name` is
/// what that machine calls itself.
pub fn local_device(
    node: &NodeId,
    machine_name: Option<&str>,
    label: Option<&str>,
) -> DeviceSnapshot {
    DeviceSnapshot {
        agent_scope: Default::default(),
        id: node.to_string(),
        label: label.map(str::to_owned),
        kind: "local".to_owned(),
        state: "ready".to_owned(),
        message: None,
        problem: None,
        ssh_alias: None,
        wsl_distro: None,
        herdr_socket_path: None,
        agent_count: 0,
        test: None,
        host: crate::model::DeviceHostSnapshot {
            consent: "this_machine".to_owned(),
            state: "ready".to_owned(),
            platform: Some(format!(
                "{} {}",
                std::env::consts::OS,
                std::env::consts::ARCH
            )),
            ..Default::default()
        },
        kit: crate::model::KitSnapshot::default(),
        machine_name: machine_name.map(str::to_owned),
        dials_in: false,
        herdr_behind: None,
    }
}

/// This Mac, then each device the operator registered. A remote device
/// starts `unavailable`; `Runtime::refresh_device_snapshots` reads its state
/// off the remote status once the connection has reported.
pub fn devices(
    node: &NodeId,
    machine_name: Option<&str>,
    ui_state: &crate::model::UiStateSnapshot,
) -> Vec<DeviceSnapshot> {
    let registrations = &ui_state.device_registrations;
    let mut result = vec![local_device(
        node,
        machine_name,
        ui_state.machine_label.as_deref(),
    )];
    let mut seen = HashSet::from([node.to_string()]);

    for registration in registrations {
        if seen.insert(registration.id.clone()) {
            result.push(DeviceSnapshot {
                agent_scope: Default::default(),
                id: registration.id.clone(),
                label: Some(registration.label.clone()),
                kind: "remote".to_owned(),
                state: "unavailable".to_owned(),
                message: None,
                problem: None,
                ssh_alias: registration.origin.ssh_alias().map(str::to_owned),
                wsl_distro: registration.origin.wsl_distro().map(str::to_owned),
                herdr_socket_path: registration.herdr_socket_path.clone(),
                agent_count: 0,
                test: None,
                host: crate::model::DeviceHostSnapshot::default(),
                kit: crate::model::KitSnapshot::default(),
                machine_name: None,
                dials_in: registration.origin == crate::model::LinkOrigin::Inbound,
                herdr_behind: None,
            });
        }
    }
    result
}

/// A registration of `path` on `device_id`, the node id for this machine.
pub fn registration(
    path: &str,
    label: &str,
    device_id: &str,
) -> Result<WorkspaceRegistration, String> {
    if device_id.trim().is_empty() {
        return Err("workspace registration must name its device".to_owned());
    }
    // The path is the node's own spelling (a folder it created, resolved or
    // answered for); the core compares it by names and never resolves it.
    if path.trim().is_empty() {
        return Err("workspace path must not be empty".to_owned());
    }
    let path = PathBuf::from(comparison_by_names(Path::new(path)));
    let label = if label.trim().is_empty() {
        path.file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("Workspace")
            .to_owned()
    } else {
        label.trim().to_owned()
    };
    Ok(WorkspaceRegistration {
        primary_checkout_id: None,
        id: workspace_id_for_path(&path),
        label,
        path: path::to_wire_lossy(&path),
        device_id: device_id.to_owned(),
        pinned: false,
        home: false,
    })
}

pub fn workspace_id_for_path(path: &Path) -> String {
    format!(
        "workspace:{:016x}",
        fnv1a(path::to_wire_lossy(path).as_bytes())
    )
}

pub fn checkout_id_for_path(workspace_id: &str, path: &Path) -> String {
    format!(
        "{workspace_id}:checkout:{:016x}",
        fnv1a(path::to_wire_lossy(path).as_bytes())
    )
}

pub fn inspect_registered(
    registration: &WorkspaceRegistration,
    paths: &PathIndex,
) -> WorkspaceSnapshot {
    let mut workspace = inspect(
        &registration.id,
        &registration.label,
        &registration.path,
        &registration.device_id,
        (true, false),
        paths,
    );
    workspace.pinned = registration.pinned;
    workspace.is_home = registration.home;
    apply_primary_checkout(&mut workspace, registration.primary_checkout_id.as_deref());
    workspace
}

pub fn inspect_temporary(path: &str, device_id: &str, paths: &PathIndex) -> WorkspaceSnapshot {
    let label = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("Unregistered workspace");
    inspect(
        &workspace_id_for_path(Path::new(path)),
        label,
        path,
        device_id,
        (false, true),
        paths,
    )
}

/// One Herdr workspace and the working directories its panes occupy.
///
/// Herdr owns the workspace axis, so the navigator mirrors it rather than
/// inventing a second one. The checkouts under it come from where its panes
/// actually are; the checkout Herdr attached it to only places a pane whose
/// folder the node can no longer read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionSpace {
    pub id: String,
    pub label: String,
    pub cwds: Vec<String>,
    pub purpose: Option<String>,
    pub checkout: Option<HerdrCheckout>,
}

/// The checkout Herdr attached a workspace to (`workspace.worktree`) and the
/// repository Herdr reports it in, both in comparison form. Herdr recorded
/// it when the checkout was opened, so it still names the repository of a
/// checkout whose folder has since been removed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HerdrCheckout {
    pub path: String,
    pub repository: String,
}

/// The effective live purpose for one checkout.
///
/// `purpose: None` is meaningful: a later workspace with no token clears an
/// earlier workspace's token and lets the checkout fall back to Git metadata.
/// `has_shadowed_purpose` lets the Git mirror clear a description on its first
/// observation when that earlier token is still live in another workspace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EffectiveCheckoutPurpose<'a> {
    pub workspace_id: &'a str,
    pub purpose: Option<&'a str>,
    pub has_shadowed_purpose: bool,
}

/// The live purpose Herdr currently exposes for this checkout.
///
/// A repository can have more than one Herdr workspace. The later occupant
/// wins even when it has no token. Intersecting with the project's own
/// workspace ids prevents a pane in a nested repository from claiming its
/// ancestor checkout merely because its cwd sits below that path.
pub fn effective_checkout_purpose<'a>(
    spaces: &'a [SessionSpace],
    workspace: &WorkspaceSnapshot,
    checkout_path: &str,
) -> Option<EffectiveCheckoutPurpose<'a>> {
    let mut authority = None;
    let mut has_shadowed_purpose = false;
    for space in spaces {
        if !space_occupies_checkout(space, workspace, checkout_path) {
            continue;
        }
        if authority.is_some_and(|previous: &SessionSpace| previous.purpose.is_some()) {
            has_shadowed_purpose = true;
        }
        authority = Some(space);
    }
    authority.map(|space| EffectiveCheckoutPurpose {
        workspace_id: &space.id,
        purpose: space.purpose.as_deref(),
        has_shadowed_purpose,
    })
}

/// The Herdr workspace a Set purpose operation mutates.
///
/// Mutation authority stays separate from effective-value projection so a
/// later policy cannot silently make Save target a different workspace.
pub fn authoritative_session_space<'a>(
    spaces: &'a [SessionSpace],
    workspace: &WorkspaceSnapshot,
    checkout_path: &str,
) -> Option<&'a SessionSpace> {
    spaces
        .iter()
        .rev()
        .find(|space| space_occupies_checkout(space, workspace, checkout_path))
}

fn space_occupies_checkout(
    space: &SessionSpace,
    workspace: &WorkspaceSnapshot,
    checkout_path: &str,
) -> bool {
    if !workspace.session_workspace_ids.contains(&space.id) {
        return false;
    }
    let checkout = PathBuf::from(comparison_by_names(Path::new(checkout_path)));
    space
        .cwds
        .iter()
        .any(|cwd| Path::new(cwd).starts_with(&checkout))
}

/// What the core's own node said about the paths of one catalog rebuild
/// (`Call::PathFacts`), by the string each was asked as and by its comparison
/// form. Asked by the sync coordinator before it takes the runtime lock, so
/// the catalog and the reconcile that run on every publish compare names and
/// never read a folder. A path it does not carry is read by its names alone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PathIndex {
    paths: BTreeMap<String, PathFact>,
    notes: BTreeMap<String, Result<BTreeMap<String, BranchNote>, String>>,
    /// Each repository's remotes, by main root (`PathFacts::remotes`).
    remotes: BTreeMap<String, Vec<String>>,
    /// The asked paths whose Project the node could not read, with why.
    unread: BTreeMap<String, String>,
}

/// How long the core's own node may take to read the paths of one rebuild.
const PATH_FACTS_TIMEOUT: Duration = Duration::from_secs(10);

/// The Projects one node could not read, as its path reader last logged
/// them: that reader asks the same paths again and again, so only a change
/// reaches the log. It keeps only the latest answer's, so it is as large as
/// one request.
#[derive(Debug, Default)]
pub(crate) struct UnreadProjects(BTreeMap<String, String>);

impl UnreadProjects {
    /// Logs each path of `index` whose Project became unreadable, or
    /// unreadable for another reason, and forgets every path `index` does
    /// not carry as unread.
    pub(crate) fn record(&mut self, index: &PathIndex) {
        for (path, reason) in &index.unread {
            if self.0.get(path) != Some(reason) {
                crate::diagnostic!(serde_json::json!({
                    "component": "workspace_catalog",
                    "kind": "project_facts.read_failed",
                    "path": path,
                    "message": reason,
                }));
            }
        }
        self.0.clone_from(&index.unread);
    }
}

impl PathIndex {
    /// An index that knows no path: every path is read by its names.
    pub const NONE: Self = Self {
        paths: BTreeMap::new(),
        notes: BTreeMap::new(),
        remotes: BTreeMap::new(),
        unread: BTreeMap::new(),
    };

    pub fn new(facts: PathFacts) -> Self {
        let unread = facts
            .paths
            .iter()
            .filter_map(|(path, fact)| Some((path.clone(), fact.project_unread.clone()?)))
            .collect();
        let mut paths = facts.paths;
        let aliases = paths
            .values()
            .filter(|fact| !paths.contains_key(&fact.comparison))
            .map(|fact| (fact.comparison.clone(), fact.clone()))
            .collect::<Vec<_>>();
        // The working tree a path is in is that tree's own place, so a row
        // made for the root reads the same facts without asking again.
        let roots = paths
            .values()
            .filter_map(|fact| {
                fact.repository
                    .as_ref()
                    .map(|place| (place, fact.project.clone()))
            })
            .map(|(place, project)| {
                (
                    place.root.clone(),
                    PathFact {
                        comparison: place.root.clone(),
                        exists: true,
                        project,
                        project_unread: None,
                        repository: Some(place.clone()),
                    },
                )
            })
            .collect::<Vec<_>>();
        for (comparison, fact) in aliases.into_iter().chain(roots) {
            paths.entry(comparison).or_insert(fact);
        }
        for (repository, error) in facts
            .repositories
            .iter()
            .filter_map(|(repository, notes)| Some((repository, notes.as_ref().err()?)))
        {
            crate::diagnostic!(serde_json::json!({
                "component": "workspace_catalog",
                "kind": "branch_notes.read_failed",
                "repository": repository,
                "message": error,
            }));
        }
        Self {
            paths,
            notes: facts.repositories,
            remotes: facts.remotes,
            unread,
        }
    }

    /// Asks `node` what `paths` are, in parts of at most
    /// `PATH_FACTS_LIMIT`.
    pub fn ask(node: &dyn NodeLink, paths: BTreeSet<String>) -> Result<Self, String> {
        Self::ask_while(node, paths, || true)
    }

    /// Check owner cancellation between bounded node calls, never accept a
    /// partially read index as a complete answer.
    pub(crate) fn ask_while(
        node: &dyn NodeLink,
        paths: BTreeSet<String>,
        current: impl Fn() -> bool,
    ) -> Result<Self, String> {
        let paths = paths.into_iter().collect::<Vec<_>>();
        let mut facts = PathFacts::default();
        for part in paths.chunks(PATH_FACTS_LIMIT) {
            if !current() {
                return Err("catalog owner ended during path read".to_owned());
            }
            let answer: PathFacts = call_as(
                node,
                Call::PathFacts {
                    paths: part.to_vec(),
                },
                PATH_FACTS_TIMEOUT,
            )
            .map_err(|error| error.to_string())?;
            facts.paths.extend(answer.paths);
            facts.repositories.extend(answer.repositories);
            facts.remotes.extend(answer.remotes);
        }
        Ok(Self::new(facts))
    }

    /// Whether the node answered for `path`.
    pub fn knows(&self, path: &str) -> bool {
        self.paths.contains_key(path)
    }

    /// The native Project facts prepared by the path's own node.
    pub(crate) fn project(&self, path: &str) -> Option<&hide_project::ProjectFacts> {
        self.paths.get(path)?.project.as_ref()
    }

    /// `path` in comparison form: the node's, else its names.
    pub fn comparison(&self, path: &str) -> String {
        self.paths
            .get(path)
            .map(|fact| fact.comparison.clone())
            .unwrap_or_else(|| comparison_by_names(Path::new(path)))
    }

    /// The repositories the remotes of the repository `path` is in name,
    /// as its node read them; `None` when the node did not say.
    pub fn remotes(&self, path: &str) -> Option<&[String]> {
        self.remotes
            .get(&self.place(path)?.main_root)
            .map(Vec::as_slice)
    }

    /// The repository `path` is in, as its node read it.
    pub fn place(&self, path: &str) -> Option<&RepositoryPlace> {
        self.paths
            .get(path)
            .and_then(|fact| fact.repository.as_ref())
    }

    /// Whether the node found `path`; a path it was not asked about is not
    /// known to exist.
    pub fn exists(&self, path: &str) -> bool {
        self.paths.get(path).is_some_and(|fact| fact.exists)
    }

    /// Whether the node answered for `path` and found nothing there.
    pub fn gone(&self, path: &str) -> bool {
        self.paths.get(path).is_some_and(|fact| !fact.exists)
    }

    /// The working tree holding `path`, else `path` itself, in comparison
    /// form.
    pub fn root(&self, path: &str) -> String {
        self.place(path)
            .map(|place| place.root.clone())
            .unwrap_or_else(|| self.comparison(path))
    }

    /// What the repository at `main_root` says of `branch`. A config that
    /// could not be read was reported when the answer arrived and is taken as
    /// no note.
    fn note(&self, main_root: &str, branch: &str) -> Option<&BranchNote> {
        self.notes.get(main_root)?.as_ref().ok()?.get(branch)
    }

    /// Every branch note of the repository holding `path`; none when the
    /// node did not place it or could not read its config.
    pub(crate) fn branch_notes(&self, path: &str) -> Option<&BTreeMap<String, BranchNote>> {
        self.notes.get(&self.place(path)?.main_root)?.as_ref().ok()
    }

    /// Every path a catalog of `registrations`, `spaces` and `worktrees`
    /// reads, for one `PathIndex::ask`.
    pub fn wanted(
        node: &NodeId,
        registrations: &[WorkspaceRegistration],
        spaces: &[SessionSpace],
        worktrees: &WorktreeCatalogSnapshot,
    ) -> BTreeSet<String> {
        let mut paths = BTreeSet::new();
        for registration in registrations
            .iter()
            .filter(|registration| *node == registration.device_id)
        {
            paths.insert(registration.path.clone());
        }
        for space in spaces {
            paths.extend(space.cwds.iter().cloned());
            paths.extend(
                space
                    .checkout
                    .iter()
                    .flat_map(|checkout| [checkout.path.clone(), checkout.repository.clone()]),
            );
        }
        for project in &worktrees.projects {
            paths.insert(project.root_path.clone());
            paths.extend(
                project
                    .worktrees
                    .iter()
                    .map(|worktree| worktree.path.clone()),
            );
        }
        paths
    }
}

pub fn build_catalog(
    node: &NodeId,
    registrations: &[WorkspaceRegistration],
    spaces: &[SessionSpace],
    worktrees: &WorktreeCatalogSnapshot,
    paths: &PathIndex,
) -> Vec<WorkspaceSnapshot> {
    // A project is a repository: its main worktree is the identity and every
    // worktree with a pane in it is a checkout under it. A Herdr workspace
    // whose panes sit in two repositories contributes to two projects, and
    // two Herdr workspaces in one repository are one project with both Herdr
    // ids attached. Herdr closing a workspace with its last pane therefore
    // never changes which row the user is looking at.
    let places = Places {
        paths,
        spaces,
        worktrees,
    };
    let mut result: Vec<WorkspaceSnapshot> = Vec::new();
    for space in spaces {
        for projected in inspect_space(node, space, &places) {
            match result
                .iter_mut()
                .find(|existing| existing.id == projected.id)
            {
                Some(existing) => merge_space(existing, projected),
                None => result.push(projected),
            }
        }
    }

    // A registration Herdr has no workspace for is somewhere the user can
    // still start work, so it stays listed. One Herdr already occupies keeps
    // the registration's identity and label with Herdr's workspace attached,
    // so the row survives Herdr closing that workspace.
    // A registration on another device names a path on that machine; this
    // machine's filesystem says nothing about it, so it is never inspected
    // here (PRD S5.5 B7). That device's catalog is its helper's to answer.
    // A repository's own registration names its project before a
    // registration of one of its checkouts could adopt the row.
    let mut local = registrations
        .iter()
        .filter(|registration| *node == registration.device_id)
        .collect::<Vec<_>>();
    local.sort_by_key(|registration| {
        places.project_root(&registration.path) != paths.comparison(&registration.path)
    });
    for registration in local {
        let comparison = places.project_root(&registration.path);
        let occupied = result
            .iter()
            .position(|workspace| comparison_by_names(Path::new(&workspace.path)) == comparison);
        match occupied {
            // A second registration for a repository that already carries
            // one is not a second row, and it must not rename the first.
            Some(index) if result[index].registered => continue,
            Some(index) => adopt_registration(&mut result[index], registration),
            None => result.push(inspect_registered(registration, paths)),
        }
    }

    for project in &mut result {
        apply_session_purposes(project, spaces);
        apply_worktrees(project, worktrees, paths);
        let primary = registrations
            .iter()
            .find(|row| row.id == project.id)
            .and_then(|row| row.primary_checkout_id.as_deref());
        apply_primary_checkout(project, primary);
    }

    result.sort_by(|left, right| {
        left.temporary
            .cmp(&right.temporary)
            .then_with(|| left.label.to_lowercase().cmp(&right.label.to_lowercase()))
            .then_with(|| left.path.cmp(&right.path))
    });
    result
}

/// Projects the registration's choice without reading the filesystem. A vanished
/// choice stays stored, so temporarily unavailable worktrees do not lose it.
pub(crate) fn apply_primary_checkout(project: &mut WorkspaceSnapshot, primary_id: Option<&str>) {
    for checkout in &mut project.checkouts {
        checkout.is_primary = project.is_git
            && match primary_id {
                Some(id) => checkout.id == id,
                None => !checkout.is_worktree,
            };
    }
}

/// Which repository each path of one catalog belongs to, so a pane's
/// directory and a registration reach the same project by the same rule.
///
/// The node's answer comes first. A path it cannot place, such as a checkout
/// whose folder was removed, is placed by the deepest checkout holding it
/// that Git's worktree list or Herdr's attached checkout still names; only a
/// path none of them places is a folder of its own. Nothing is placed by the
/// shape of its name.
struct Places<'a> {
    paths: &'a PathIndex,
    spaces: &'a [SessionSpace],
    worktrees: &'a WorktreeCatalogSnapshot,
}

impl Places<'_> {
    /// The repository `path` is in, and the working tree holding it.
    fn place(&self, path: &str) -> Option<Cow<'_, RepositoryPlace>> {
        if let Some(place) = self.paths.place(path) {
            return Some(Cow::Borrowed(place));
        }
        let path = self.paths.comparison(path);
        let listed = self.worktrees.projects.iter().flat_map(|project| {
            project.worktrees.iter().map(|worktree| RepositoryPlace {
                root: self.paths.comparison(&worktree.path),
                main_root: self.paths.comparison(&project.root_path),
                branch: worktree.branch.clone(),
                head_oid: worktree.head_sha.clone(),
            })
        });
        let attached = self
            .spaces
            .iter()
            .filter_map(|space| space.checkout.as_ref())
            .map(|checkout| RepositoryPlace {
                root: checkout.path.clone(),
                main_root: self
                    .paths
                    .place(&checkout.repository)
                    .map(|place| place.main_root.clone())
                    .unwrap_or_else(|| checkout.repository.clone()),
                branch: None,
                head_oid: None,
            });
        listed
            .chain(attached)
            .filter(|place| Path::new(&path).starts_with(&place.root))
            .fold(
                None,
                |deepest: Option<RepositoryPlace>, place| match deepest {
                    Some(deepest) if deepest.root.len() >= place.root.len() => Some(deepest),
                    _ => Some(place),
                },
            )
            .map(Cow::Owned)
    }

    /// The directory that identifies `path`'s project, in comparison form:
    /// the repository's main worktree for a git checkout, the folder itself
    /// otherwise.
    fn project_root(&self, path: &str) -> String {
        match self.place(path) {
            Some(place) => place.main_root.clone(),
            None => self.paths.comparison(path),
        }
    }
}

/// Folds a second Herdr workspace in the same repository into the project
/// that already represents it.
fn merge_space(existing: &mut WorkspaceSnapshot, incoming: WorkspaceSnapshot) {
    for id in incoming.session_workspace_ids {
        if !existing.session_workspace_ids.contains(&id) {
            existing.session_workspace_ids.push(id);
        }
    }
    for checkout in incoming.checkouts {
        let comparison = comparison_by_names(Path::new(&checkout.path));
        if !existing
            .checkouts
            .iter()
            .any(|known| comparison_by_names(Path::new(&known.path)) == comparison)
        {
            existing.checkouts.push(checkout);
        }
    }
}

/// Applies the same last-occupant decision used by the Git mirror. A missing
/// token deliberately leaves the branch-description fallback in place.
fn apply_session_purposes(project: &mut WorkspaceSnapshot, spaces: &[SessionSpace]) {
    for index in 0..project.checkouts.len() {
        let checkout_path = project.checkouts[index].path.clone();
        let purpose = effective_checkout_purpose(spaces, project, &checkout_path)
            .and_then(|effective| effective.purpose);
        if let Some(purpose) = purpose {
            project.checkouts[index].purpose = Some(CheckoutPurposeSnapshot {
                text: purpose.to_owned(),
                origin: CheckoutPurposeOrigin::Token,
            });
        }
    }
}

/// Gives a Herdr-occupied project the identity and label of the registration
/// that covers it. Checkout ids embed the project id, so they are re-keyed
/// too; persisted focus on a registered checkout then resolves whether or not
/// Herdr currently has a workspace there.
fn adopt_registration(workspace: &mut WorkspaceSnapshot, registration: &WorkspaceRegistration) {
    workspace.id = registration.id.clone();
    workspace.label = registration.label.clone();
    workspace.registered = true;
    workspace.temporary = false;
    workspace.pinned = registration.pinned;
    workspace.is_home = registration.home;
    workspace.device_id = registration.device_id.clone();
    workspace.remote_target_id = None;
    for checkout in &mut workspace.checkouts {
        checkout.id = checkout_id_for_path(&registration.id, Path::new(&checkout.path));
        checkout.workspace_id = registration.id.clone();
        checkout.temporary = false;
    }
}

/// Projects one Herdr workspace onto the repositories its panes sit in, one
/// project per repository with a checkout per directory a pane occupies.
///
/// This establishes the project rows and the checkouts Herdr can vouch for
/// without git. Every other worktree of the repository is added by
/// [`apply_worktrees`] from the worktree reader's answer, which is what makes
/// a worktree with no terminal a row the operator can select and start one in.
fn inspect_space(node: &NodeId, space: &SessionSpace, places: &Places) -> Vec<WorkspaceSnapshot> {
    let paths = places.paths;
    let mut projects: Vec<WorkspaceSnapshot> = Vec::new();
    for cwd in &space.cwds {
        // One answer per directory gives root, project and branch, read by
        // the node from Git's files.
        let repository = places.place(cwd);
        let repository = repository.as_deref();
        // A checkout only Git's list or Herdr's record places is one the
        // node could not read; it says whether the folder is gone.
        let recorded_only = repository.is_some() && paths.place(cwd).is_none();
        let root_comparison = repository
            .map(|place| place.root.clone())
            .unwrap_or_else(|| paths.comparison(cwd));
        let project_comparison = repository
            .map(|place| place.main_root.clone())
            .unwrap_or_else(|| root_comparison.clone());
        let root = PathBuf::from(&root_comparison);
        let project_path = PathBuf::from(&project_comparison);
        let workspace_id = workspace_id_for_path(&project_path);
        let index = match projects
            .iter()
            .position(|project| comparison_by_names(Path::new(&project.path)) == project_comparison)
        {
            Some(index) => index,
            None => {
                let name = project_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(&space.label)
                    .to_owned();
                projects.push(WorkspaceSnapshot {
                    agent_presence: Default::default(),
                    agent_scope: Default::default(),
                    home_issues: Default::default(),
                    pull_requests: Vec::new(),
                    tasks: Default::default(),
                    id: workspace_id.clone(),
                    label: name.clone(),
                    path: path::to_wire_lossy(&project_path),
                    remote_target_id: None,
                    expanded: true,
                    device_id: node.to_string(),
                    repo_name: name,
                    is_git: repository.is_some(),
                    default_branch: None,
                    branches: Vec::new(),
                    registered: false,
                    temporary: false,
                    session_workspace_ids: vec![space.id.clone()],
                    last_activity_unix_ms: None,
                    checkouts: Vec::new(),
                    pinned: false,
                    is_home: false,
                    inactive_checkouts: Default::default(),
                    session_folds: Default::default(),
                    removal: Default::default(),
                    disk: Default::default(),
                    cleanup: None,
                });
                projects.len() - 1
            }
        };
        if projects[index]
            .checkouts
            .iter()
            .any(|existing| comparison_by_names(Path::new(&existing.path)) == root_comparison)
        {
            continue;
        }
        let branch = repository.and_then(|place| place.branch.clone());
        let label = checkout_row_label(branch.as_deref(), &root);
        let is_worktree = root_comparison != project_comparison;
        if !is_worktree {
            projects[index].default_branch = branch.clone();
        }
        projects[index].checkouts.push(checkout(
            &workspace_id,
            &root,
            &label,
            (is_worktree, false),
            !(recorded_only && paths.gone(&root_comparison)),
            repository
                .map(|place| git_facts(&place.main_root, branch, place.head_oid.clone(), paths)),
        ));
    }
    // The main worktree leads, so the primary badge and the project path
    // agree even when a worktree's pane was reported first.
    for project in &mut projects {
        let project_comparison = comparison_by_names(Path::new(&project.path));
        if let Some(index) = project.checkouts.iter().position(|checkout| {
            comparison_by_names(Path::new(&checkout.path)) == project_comparison
        }) && index != 0
        {
            let main = project.checkouts.remove(index);
            project.checkouts.insert(0, main);
        }
    }
    projects
}

/// Adds every worktree git reports to the project, and carries each
/// worktree's counts onto the row that represents it.
///
/// A worktree with no pane becomes a row here; one that already has a row from
/// a pane keeps its identity and only gains the counts. The reader has not
/// answered for a project until it appears in the catalog, and a project with
/// no answer keeps exactly the rows it already had rather than losing them.
pub(crate) fn apply_worktrees(
    project: &mut WorkspaceSnapshot,
    worktrees: &WorktreeCatalogSnapshot,
    paths: &PathIndex,
) {
    let project_comparison = paths.comparison(&project.path);
    let Some(listed) = worktrees
        .projects
        .iter()
        .find(|listed| paths.comparison(&listed.root_path) == project_comparison)
    else {
        return;
    };
    project.default_branch = listed.default_branch.clone();
    project.branches = listed.branches.clone();

    // A listed project's path is its main worktree's, in comparison form.
    let main_root = project_comparison.clone();
    for worktree in &listed.worktrees {
        let comparison = paths.comparison(&worktree.path);
        let existing = project
            .checkouts
            .iter()
            .position(|known| paths.comparison(&known.path) == comparison);
        let index = match existing {
            Some(index) => index,
            None => {
                let path = PathBuf::from(&worktree.path);
                let label = checkout_row_label(worktree.branch.as_deref(), &path);
                // The row's `worktree` carries the reader's commit, which
                // `head_sha` prefers; no second copy is made here.
                let git = git_facts(&main_root, worktree.branch.clone(), None, paths);
                project.checkouts.push(checkout(
                    &project.id,
                    &path,
                    &label,
                    (!worktree.is_main, project.temporary),
                    !worktree.missing,
                    Some(git),
                ));
                project.checkouts.len() - 1
            }
        };
        let row = &mut project.checkouts[index];
        row.is_worktree = !worktree.is_main;
        row.label = checkout_row_label(worktree.branch.as_deref(), Path::new(&worktree.path));
        row.exists = !worktree.missing;
        row.dirty = worktree.dirty;
        row.changed_file_count = worktree.changed_file_count;
        row.base_branch = worktree.base_branch.clone();
        row.ahead = worktree.ahead;
        row.behind = worktree.behind;
        row.added_lines = worktree.added_lines;
        row.removed_lines = worktree.removed_lines;
        row.unpushed = worktree.unpushed.clone();
        row.worktree = Some(worktree.clone());
        row.branch = worktree.branch.clone();
        if !row
            .purpose
            .as_ref()
            .is_some_and(|purpose| purpose.origin == CheckoutPurposeOrigin::Token)
        {
            row.purpose = worktree
                .branch
                .as_deref()
                .and_then(|branch| branch_purpose(paths.note(&main_root, branch)));
        }
    }

    // The main worktree leads, so the primary badge and the project path
    // agree whichever order the rows were created in.
    if let Some(index) = project
        .checkouts
        .iter()
        .position(|checkout| comparison_by_names(Path::new(&checkout.path)) == project_comparison)
        && index != 0
    {
        let main = project.checkouts.remove(index);
        project.checkouts.insert(0, main);
    }
}

pub(crate) fn checkout_row_label(branch: Option<&str>, path: &Path) -> String {
    branch.map(str::to_owned).unwrap_or_else(|| {
        path.file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .into_owned()
    })
}

/// What the node in this process says of `paths`, for a test that builds a
/// catalog by hand.
#[cfg(test)]
pub(crate) fn paths_here<I, P>(paths: I) -> PathIndex
where
    I: IntoIterator<Item = P>,
    P: Into<String>,
{
    PathIndex::ask(
        &hide_node::Local::of_process(),
        paths.into_iter().map(Into::into).collect(),
    )
    .expect("the node in this process answers")
}

/// The paths a catalog of `registrations`, `spaces` and `worktrees` reads, as
/// the node in this process answers them.
#[cfg(test)]
pub(crate) fn catalog_paths_here(
    registrations: &[WorkspaceRegistration],
    spaces: &[SessionSpace],
    worktrees: &WorktreeCatalogSnapshot,
) -> PathIndex {
    paths_here(PathIndex::wanted(
        &crate::node::test_node(),
        registrations,
        spaces,
        worktrees,
    ))
}

/// The node in this process, counting what it is asked: a test that reads
/// `calls` around an ingest proves the ingest asked the node nothing under
/// the runtime lock.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct CountingNode {
    calls: std::sync::atomic::AtomicUsize,
}

#[cfg(test)]
impl CountingNode {
    pub(crate) fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(test)]
impl NodeLink for CountingNode {
    fn reader_features(&self) -> Option<&hide_node_link::sessions::ReaderFeatures> {
        static READERS: std::sync::LazyLock<hide_node_link::sessions::ReaderFeatures> =
            std::sync::LazyLock::new(hide_node_link::sessions::ReaderFeatures::implemented);
        Some(&READERS)
    }

    fn call(
        &self,
        call: Call,
        timeout: Duration,
    ) -> Result<hide_node_link::link::LinkAnswer, hide_node_link::link::LinkError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        hide_node::Local::of_process().call(call, timeout)
    }

    fn in_process(&self) -> bool {
        true
    }
}

/// A base directory for tests that assert what the catalog says about a
/// directory which is not a checkout.
///
/// `std::env::temp_dir()` is not always outside a repository. The verification
/// sandbox points `TMPDIR` inside this repository, and git discovery walks up
/// from a directory created there and finds it, so a test that means "a folder
/// with no repository" gets the enclosing repository instead. Picking the base
/// by asking git, rather than assuming, keeps those tests measuring the product
/// instead of the environment they happen to run in.
#[cfg(test)]
pub(crate) fn temp_base_outside_any_repository() -> &'static Path {
    use std::sync::OnceLock;

    static BASE: OnceLock<PathBuf> = OnceLock::new();
    BASE.get_or_init(|| {
        let preferred = std::env::temp_dir();
        for candidate in [preferred.clone(), PathBuf::from("/tmp")] {
            if candidate.is_dir() && hide_project::git::discover(&candidate).is_none() {
                return candidate;
            }
        }
        panic!(
            "these tests need a temporary directory outside every git repository; \
             both {} and /tmp are inside one",
            preferred.display()
        )
    })
    .as_path()
}

fn inspect(
    id: &str,
    label: &str,
    path: &str,
    device_id: &str,
    (registered, temporary): (bool, bool),
    paths: &PathIndex,
) -> WorkspaceSnapshot {
    let repository = paths.place(path);
    let normalized = paths.comparison(path);
    let is_git = repository.is_some();
    // One row for where this registration points. The repository's other
    // worktrees are added from the worktree reader's answer, so this stands
    // alone only for a plain folder and for the ticks before the first read.
    let branch = repository.and_then(|place| place.branch.clone());
    let checkouts = match repository {
        Some(place) => {
            let root = Path::new(&place.root);
            vec![checkout(
                id,
                root,
                &checkout_row_label(branch.as_deref(), root),
                (false, temporary),
                true,
                Some(git_facts(
                    &place.main_root,
                    branch.clone(),
                    place.head_oid.clone(),
                    paths,
                )),
            )]
        }
        None => vec![checkout(
            id,
            Path::new(&normalized),
            &checkout_row_label(None, Path::new(&normalized)),
            (false, temporary),
            paths.exists(path),
            None,
        )],
    };

    WorkspaceSnapshot {
        agent_presence: Default::default(),
        agent_scope: Default::default(),
        home_issues: Default::default(),
        pull_requests: Vec::new(),
        tasks: Default::default(),
        id: id.to_owned(),
        label: label.to_owned(),
        path: normalized.clone(),
        // Only this node's paths are read: its answer says nothing about
        // another machine.
        remote_target_id: None,
        expanded: true,
        device_id: device_id.to_owned(),
        repo_name: repository
            .and_then(|place| Path::new(&place.root).file_name())
            .and_then(|name| name.to_str())
            .unwrap_or(label)
            .to_owned(),
        is_git,
        default_branch: branch,
        branches: Vec::new(),
        registered,
        temporary,
        session_workspace_ids: Vec::new(),
        last_activity_unix_ms: None,
        pinned: false,
        is_home: false,
        checkouts,
        inactive_checkouts: Default::default(),
        session_folds: Default::default(),
        removal: Default::default(),
        disk: Default::default(),
        cleanup: None,
    }
}

/// What a checkout row knows from Git when it is made.
struct CheckoutGit {
    branch: Option<String>,
    head_oid: Option<String>,
    purpose: Option<CheckoutPurposeSnapshot>,
    issue: Option<String>,
}

/// A row's Git facts: its branch and commit, and what the repository at
/// `main_root` says of the branch.
fn git_facts(
    main_root: &str,
    branch: Option<String>,
    head_oid: Option<String>,
    paths: &PathIndex,
) -> CheckoutGit {
    let note = branch
        .as_deref()
        .and_then(|branch| paths.note(main_root, branch));
    CheckoutGit {
        purpose: branch_purpose(note),
        issue: note.and_then(|note| note.issue.clone()),
        branch,
        head_oid,
    }
}

/// A checkout row of `path`. `git` is `None` for a folder outside any
/// repository.
fn checkout(
    workspace_id: &str,
    path: &Path,
    label: &str,
    (is_worktree, temporary): (bool, bool),
    exists: bool,
    git: Option<CheckoutGit>,
) -> CheckoutSnapshot {
    let is_primary = git.is_some() && !is_worktree;
    let git = git.unwrap_or(CheckoutGit {
        branch: None,
        head_oid: None,
        purpose: None,
        issue: None,
    });
    CheckoutSnapshot {
        agent_scope: Default::default(),
        branch_issue: git.issue,
        head_oid: git.head_oid,
        // A checkout with no Herdr tabs yet: the first one the operator makes
        // here is Tab 1. Reconcile overwrites this the moment Herdr reports any.
        next_tab_label: crate::model::next_tab_label(std::iter::empty()),
        id: checkout_id_for_path(workspace_id, path),
        workspace_id: workspace_id.to_owned(),
        label: label.to_owned(),
        path: path::to_wire_lossy(path),
        branch: git.branch,
        purpose: git.purpose,
        is_worktree,
        is_primary,
        exists,
        temporary,
        tabs: Vec::<TabSnapshot>::new(),
        active_tab_id: None,
        strip: Vec::new(),
        // The git facts arrive from the worktree reader; the catalog only
        // decides which rows exist.
        ..CheckoutSnapshot::default()
    }
}

fn branch_purpose(note: Option<&BranchNote>) -> Option<CheckoutPurposeSnapshot> {
    note?
        .description
        .clone()
        .map(|text| CheckoutPurposeSnapshot {
            text,
            origin: CheckoutPurposeOrigin::BranchDescription,
        })
}

/// `path` compared by its names: spelled with `/` between names and without
/// a trailing separator. Links are the node's to resolve (`PathIndex`); the
/// core only compares what a node already spelled.
pub fn comparison_by_names(path: &Path) -> String {
    let wire = path::to_wire_lossy(path);
    match wire.trim_end_matches('/') {
        "" if wire.starts_with('/') => "/".to_owned(),
        trimmed => trimmed.to_owned(),
    }
}

pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ProjectWorktreesSnapshot, WorktreeSnapshot};
    use std::fs;
    use std::process::Command;

    #[test]
    fn a_project_its_node_could_not_read_reaches_the_cores_log_once_per_change() {
        let facts = |paths: &[(&str, Option<&str>)]| PathFacts {
            paths: paths
                .iter()
                .map(|(path, unread)| {
                    let fact = PathFact {
                        comparison: (*path).into(),
                        exists: true,
                        project: None,
                        project_unread: unread.map(str::to_owned),
                        repository: None,
                    };
                    ((*path).to_owned(), fact)
                })
                .collect(),
            ..PathFacts::default()
        };
        let logged = |log: &mut UnreadProjects, paths: &[(&str, Option<&str>)]| {
            let index = PathIndex::new(facts(paths));
            let (_, records) = crate::diagnostics::capture(|| log.record(&index));
            records
                .iter()
                .filter(|record| record["kind"] == "project_facts.read_failed")
                .map(|record| (record["path"].clone(), record["message"].clone()))
                .collect::<Vec<_>>()
        };
        let said = |reason: &str| vec![(serde_json::json!("/p"), serde_json::json!(reason))];
        let (mut one, mut other) = (UnreadProjects::default(), UnreadProjects::default());
        assert_eq!(
            logged(
                &mut one,
                &[("/p", Some("project_path_not_utf8")), ("/q", None)]
            ),
            said("project_path_not_utf8")
        );
        assert_eq!(
            logged(&mut one, &[("/p", Some("project_path_not_utf8"))]),
            [],
            "the same answer again"
        );
        assert_eq!(
            logged(&mut other, &[("/p", None)]),
            [],
            "another node reads its own /p"
        );
        assert_eq!(
            logged(&mut one, &[("/p", Some("project_path_not_utf8"))]),
            [],
            "and leaves this node's record as it was"
        );
        assert_eq!(
            logged(&mut one, &[("/p", Some("project_git_link_invalid"))]),
            said("project_git_link_invalid")
        );
        assert_eq!(logged(&mut one, &[("/q", None)]), []);
        assert_eq!(
            logged(&mut one, &[("/p", Some("project_git_link_invalid"))]),
            said("project_git_link_invalid"),
            "a path no longer asked is forgotten, so asking it again is a change"
        );
    }

    /// `build_catalog` with the paths the node in this process answers.
    fn catalog(
        registrations: &[WorkspaceRegistration],
        spaces: &[SessionSpace],
        worktrees: &WorktreeCatalogSnapshot,
    ) -> Vec<WorkspaceSnapshot> {
        let paths = catalog_paths_here(registrations, spaces, worktrees);
        build_catalog(
            &crate::node::test_node(),
            registrations,
            spaces,
            worktrees,
            &paths,
        )
    }

    /// The catalog before the worktree reader has answered. Every case that
    /// is not about worktree rows uses this, so those tests still assert what
    /// the pane-derived catalog alone produces.
    fn no_worktrees() -> WorktreeCatalogSnapshot {
        WorktreeCatalogSnapshot::default()
    }

    /// A folder named `name` inside a new scratch folder outside every
    /// repository. The scratch folder goes when the first value is dropped,
    /// with everything a test makes beside `name` (a linked worktree, a
    /// second checkout), so nothing is left behind however the test ends.
    fn temp_dir(name: &str) -> (tempfile::TempDir, PathBuf) {
        let scratch = tempfile::Builder::new()
            .prefix(&format!("hide-workspace-{name}-"))
            .tempdir_in(temp_base_outside_any_repository())
            .expect("a scratch folder");
        let path = scratch.path().join(name);
        fs::create_dir(&path).expect("temp directory");
        (scratch, path)
    }

    /// A repository with one commit on `main` and a linked worktree on
    /// `feature`, made with git itself so the catalog is measured against
    /// what git would say.
    fn repository_with_worktree(name: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let (scratch, root) = temp_dir(name);
        let run = |dir: &Path, args: &[&str]| {
            let status = Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .status()
                .expect("git runs");
            assert!(status.success(), "git {args:?} in {}", dir.display());
        };
        run(&root, &["init", "-b", "main"]);
        fs::write(root.join("README.md"), "fixture\n").expect("fixture file");
        run(&root, &["add", "."]);
        run(
            &root,
            &[
                "-c",
                "commit.gpgsign=false",
                "-c",
                "user.email=hide@example.invalid",
                "-c",
                "user.name=hide-test",
                "commit",
                "-m",
                "fixture",
            ],
        );
        let worktree = root.with_file_name(format!(
            "{}-worktree",
            root.file_name().unwrap().to_string_lossy()
        ));
        run(
            &root,
            &[
                "worktree",
                "add",
                "-b",
                "feature",
                worktree.to_str().unwrap(),
            ],
        );
        (scratch, root, worktree)
    }

    // The catalog is rebuilt on the session-sync coordinator, the thread that
    // applies Herdr's events, from what the node said of its paths; the node
    // reads the repositories' own files (2026-09-10 audit: one git per fact
    // held Herdr's events for seconds).
    #[test]
    fn the_catalog_says_what_git_says_from_the_nodes_answer() {
        let (_scratch, root, worktree) = repository_with_worktree("no-spawn");
        let nested = worktree.join("src");
        fs::create_dir_all(&nested).expect("nested directory");
        let (_plain_scratch, folder) = temp_dir("no-spawn-plain");
        let demo = registration(root.to_str().unwrap(), "Demo", crate::node::TEST_NODE)
            .expect("registration");
        let plain = registration(folder.to_str().unwrap(), "Plain", crate::node::TEST_NODE)
            .expect("registration");
        let spaces = vec![
            SessionSpace {
                checkout: None,
                id: "w1".to_owned(),
                label: "one".to_owned(),
                purpose: None,
                cwds: vec![
                    root.to_string_lossy().into_owned(),
                    nested.to_string_lossy().into_owned(),
                ],
            },
            SessionSpace {
                checkout: None,
                id: "w2".to_owned(),
                label: "two".to_owned(),
                purpose: None,
                cwds: vec![folder.to_string_lossy().into_owned()],
            },
        ];

        let registrations = [demo, plain];
        let paths = catalog_paths_here(&registrations, &spaces, &no_worktrees());
        let catalog = build_catalog(
            &crate::node::test_node(),
            &registrations,
            &spaces,
            &no_worktrees(),
            &paths,
        );

        // The worktree's pane is a checkout row under the repository it
        // belongs to, on its own branch.
        let project = catalog
            .iter()
            .find(|project| project.label == "Demo")
            .expect("the repository is a project");
        assert!(project.is_git);
        assert_eq!(project.default_branch.as_deref(), Some("main"));
        assert_eq!(project.session_workspace_ids, vec!["w1".to_owned()]);
        assert_eq!(project.checkouts.len(), 2);
        assert!(!project.checkouts[0].is_worktree);
        let linked = &project.checkouts[1];
        assert!(linked.is_worktree);
        assert_eq!(linked.branch.as_deref(), Some("feature"));
        assert_eq!(
            linked.path,
            path::to_wire_lossy(&fs::canonicalize(&worktree).unwrap())
        );
        let plain = catalog
            .iter()
            .find(|project| project.label == "Plain")
            .expect("the folder is a project");
        assert!(!plain.is_git);
        assert_eq!(plain.session_workspace_ids, vec!["w2".to_owned()]);
        assert_eq!(
            paths.root(nested.to_str().unwrap()),
            path::to_wire_lossy(&fs::canonicalize(&worktree).unwrap())
        );
    }

    #[test]
    fn discovers_git_default_branch_and_worktrees_without_mutating_the_repository() {
        let (_scratch, root) = temp_dir("git");
        let status = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["init", "-b", "main"])
            .status()
            .expect("git init");
        assert!(status.success());
        fs::write(root.join("README.md"), "fixture\n").expect("fixture file");
        let add = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["add", "."])
            .status()
            .expect("git add");
        assert!(add.success());
        let commit = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "user.email=hide@example.invalid",
                "-c",
                "user.name=hide-test",
                "commit",
                "-m",
                "fixture",
            ])
            .status()
            .expect("git commit");
        assert!(commit.success());
        let checkout_path = root.with_file_name(format!(
            "{}-worktree",
            root.file_name().unwrap().to_string_lossy()
        ));
        let worktree = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args([
                "worktree",
                "add",
                "-b",
                "feature",
                checkout_path.to_str().unwrap(),
            ])
            .status()
            .expect("git worktree add");
        assert!(worktree.success());

        let registration = registration(root.to_str().unwrap(), "Demo", crate::node::TEST_NODE)
            .expect("registration");
        let snapshot = inspect_registered(&registration, &paths_here([registration.path.clone()]));
        // A registration is one row for where it is. The `feature` worktree
        // exists on disk but has no pane, so it is not a checkout row.
        assert!(snapshot.is_git);
        assert_eq!(snapshot.default_branch.as_deref(), Some("main"));
        assert_eq!(snapshot.checkouts.len(), 1);
        assert_eq!(snapshot.checkouts[0].branch.as_deref(), Some("main"));

        // It becomes one once a Herdr workspace has a pane in it.
        let occupied = catalog(
            &[],
            &[SessionSpace {
                checkout: None,
                id: "w1".to_owned(),
                label: "Demo".to_owned(),
                purpose: None,
                cwds: vec![
                    root.to_string_lossy().into_owned(),
                    checkout_path.to_string_lossy().into_owned(),
                ],
            }],
            &no_worktrees(),
        );
        assert_eq!(occupied.len(), 1);
        assert_eq!(occupied[0].checkouts.len(), 2);
        assert!(
            occupied[0]
                .checkouts
                .iter()
                .any(|checkout| checkout.is_worktree
                    && checkout.branch.as_deref() == Some("feature"))
        );
        assert!(root.join("README.md").exists());
    }

    #[test]
    fn flat_folder_is_visible_without_implicit_git_init() {
        let (_scratch, root) = temp_dir("flat");
        let registration = registration(root.to_str().unwrap(), "Flat", crate::node::TEST_NODE)
            .expect("registration");
        let snapshot = inspect_registered(&registration, &paths_here([registration.path.clone()]));
        assert!(!snapshot.is_git);
        assert_eq!(snapshot.checkouts.len(), 1);
        assert_eq!(
            snapshot.checkouts[0].label,
            root.file_name().unwrap().to_string_lossy()
        );
        assert!(!root.join(".git").exists());
    }

    #[test]
    fn a_space_occupying_a_registered_directory_does_not_duplicate_it() {
        let (_scratch, root) = temp_dir("temporary");
        let registration =
            registration(root.to_str().unwrap(), "Registered", crate::node::TEST_NODE)
                .expect("registration");
        let spaces = [SessionSpace {
            checkout: None,
            id: "w1".to_owned(),
            label: "Registered".to_owned(),
            purpose: None,
            cwds: vec![root.to_string_lossy().into_owned()],
        }];

        let catalog = catalog(
            std::slice::from_ref(&registration),
            &spaces,
            &no_worktrees(),
        );

        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].id, registration.id);
        assert_eq!(catalog[0].label, "Registered");
        assert!(catalog[0].registered);
        assert_eq!(catalog[0].session_workspace_ids, vec!["w1".to_owned()]);
    }

    /// Herdr closes a workspace together with its last pane. The project and
    /// checkout the user had selected must be the same rows afterwards, so a
    /// persisted focus keeps resolving and the checkout offers to start a new
    /// terminal instead of the app falling back to "no workspace".
    #[test]
    fn a_project_keeps_its_identity_when_herdr_closes_its_workspace() {
        let (_scratch, root) = temp_dir("identity");
        let registration = registration(root.to_str().unwrap(), "Identity", crate::node::TEST_NODE)
            .expect("registration");
        let space = SessionSpace {
            checkout: None,
            id: "w7".to_owned(),
            label: "hide main".to_owned(),
            purpose: None,
            cwds: vec![root.to_string_lossy().into_owned()],
        };

        let occupied = catalog(
            std::slice::from_ref(&registration),
            &[space],
            &no_worktrees(),
        );
        let released = catalog(std::slice::from_ref(&registration), &[], &no_worktrees());

        assert_eq!(occupied[0].id, released[0].id);
        assert_eq!(occupied[0].label, released[0].label);
        assert_eq!(occupied[0].checkouts[0].id, released[0].checkouts[0].id);
        assert_eq!(occupied[0].session_workspace_ids, vec!["w7".to_owned()]);
        assert!(released[0].session_workspace_ids.is_empty());
    }

    /// A Herdr workspace with panes in two repositories is two projects, and
    /// each registration lands on its own repository.
    #[test]
    fn a_space_spanning_two_repositories_is_two_projects() {
        let (_first_scratch, first) = temp_dir("first-repo");
        let (_second_scratch, second) = temp_dir("second-repo");
        let space = SessionSpace {
            checkout: None,
            id: "w9".to_owned(),
            label: "hide main".to_owned(),
            purpose: None,
            cwds: vec![
                first.to_string_lossy().into_owned(),
                second.to_string_lossy().into_owned(),
            ],
        };
        let registrations = [
            registration(first.to_str().unwrap(), "First", crate::node::TEST_NODE).expect("first"),
            registration(second.to_str().unwrap(), "Second", crate::node::TEST_NODE)
                .expect("second"),
        ];

        let unregistered = catalog(&[], std::slice::from_ref(&space), &no_worktrees());
        let catalog = catalog(&registrations, &[space], &no_worktrees());

        assert_eq!(unregistered.len(), 2);
        assert!(
            unregistered
                .iter()
                .all(|project| project.label != "hide main")
        );
        assert_eq!(catalog.len(), 2);
        let labels = catalog.iter().map(|p| p.label.as_str()).collect::<Vec<_>>();
        assert_eq!(labels, vec!["First", "Second"]);
        assert!(catalog.iter().all(|project| {
            project.registered
                && project.checkouts.len() == 1
                && project.session_workspace_ids == vec!["w9".to_owned()]
        }));
    }

    #[test]
    fn an_unregistered_space_is_keyed_by_its_repository_path() {
        let (_scratch, root) = temp_dir("unregistered-space");
        let space = SessionSpace {
            checkout: None,
            id: "w8".to_owned(),
            label: "scratch".to_owned(),
            purpose: None,
            cwds: vec![root.to_string_lossy().into_owned()],
        };

        let catalog = catalog(&[], &[space], &no_worktrees());

        assert_eq!(catalog.len(), 1);
        let canonical = fs::canonicalize(&root).expect("canonical root");
        assert_eq!(catalog[0].id, workspace_id_for_path(&canonical));
        assert_eq!(catalog[0].session_workspace_ids, vec!["w8".to_owned()]);
        assert!(!catalog[0].registered);
    }

    #[test]
    fn later_space_without_a_purpose_clears_the_earlier_token_projection() {
        let (_scratch, root) = temp_dir("shared-root");
        let cwd = root.to_string_lossy().into_owned();
        let spaces = [
            SessionSpace {
                checkout: None,
                id: "w1".to_owned(),
                label: "first".to_owned(),
                purpose: Some("Earlier purpose".to_owned()),
                cwds: vec![cwd.clone()],
            },
            SessionSpace {
                checkout: None,
                id: "w2".to_owned(),
                label: "second".to_owned(),
                purpose: None,
                cwds: vec![cwd.clone()],
            },
        ];

        let catalog = catalog(&[], &spaces, &no_worktrees());

        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].checkouts.len(), 1);
        assert_eq!(
            catalog[0].session_workspace_ids,
            vec!["w1".to_owned(), "w2".to_owned()]
        );
        assert_eq!(catalog[0].checkouts[0].purpose, None);
        assert_eq!(
            effective_checkout_purpose(&spaces, &catalog[0], &cwd),
            Some(EffectiveCheckoutPurpose {
                workspace_id: "w2",
                purpose: None,
                has_shadowed_purpose: true,
            })
        );
    }

    #[test]
    fn purpose_authority_is_the_last_project_occupant_not_a_nested_repository() {
        let spaces = vec![
            SessionSpace {
                checkout: None,
                id: "outer-first".to_owned(),
                label: "Outer first".to_owned(),
                purpose: Some("First".to_owned()),
                cwds: vec!["/fixture/outer/worktree".to_owned()],
            },
            SessionSpace {
                checkout: None,
                id: "nested".to_owned(),
                label: "Nested".to_owned(),
                purpose: Some("Wrong repository".to_owned()),
                cwds: vec!["/fixture/outer/worktree/nested".to_owned()],
            },
            SessionSpace {
                checkout: None,
                id: "outer-last".to_owned(),
                label: "Outer last".to_owned(),
                purpose: Some("Last".to_owned()),
                cwds: vec!["/fixture/outer/worktree".to_owned()],
            },
        ];
        let project = WorkspaceSnapshot {
            agent_presence: Default::default(),
            agent_scope: Default::default(),
            home_issues: Default::default(),
            pull_requests: Vec::new(),
            tasks: Default::default(),
            id: "outer".to_owned(),
            label: "Outer".to_owned(),
            path: "/fixture/outer".to_owned(),
            remote_target_id: None,
            expanded: true,
            device_id: "local".to_owned(),
            repo_name: "outer".to_owned(),
            is_git: true,
            default_branch: Some("main".to_owned()),
            branches: vec!["main".to_owned()],
            registered: true,
            temporary: false,
            session_workspace_ids: vec!["outer-first".to_owned(), "outer-last".to_owned()],
            last_activity_unix_ms: None,
            checkouts: Vec::new(),
            pinned: false,
            is_home: false,
            inactive_checkouts: Default::default(),
            session_folds: Default::default(),
            removal: Default::default(),
            disk: Default::default(),
            cleanup: None,
        };

        let authority = authoritative_session_space(&spaces, &project, "/fixture/outer/worktree")
            .expect("outer checkout authority");

        assert_eq!(authority.id, "outer-last");
        assert_eq!(authority.purpose.as_deref(), Some("Last"));
    }

    #[test]
    fn purpose_authority_normalizes_symlink_and_dot_segment_cwds() {
        let (_scratch, root) = temp_dir("purpose-authority-alias");
        let checkout = root.join("checkout");
        let nested = checkout.join("nested");
        let alias = root.join("checkout-alias");
        fs::create_dir_all(&nested).expect("checkout fixture");
        hide_platform::fs::link::create_link(&checkout, &alias).expect("checkout symlink");
        // The spaces carry their cwds as the node reads them, which is where
        // the link and the `..` are resolved.
        let alias = alias.to_string_lossy().into_owned();
        let dotted = nested.join("..").to_string_lossy().into_owned();
        let paths = paths_here([alias.clone(), dotted.clone()]);
        let spaces = vec![
            SessionSpace {
                checkout: None,
                id: "symlink".to_owned(),
                label: "Symlink".to_owned(),
                purpose: Some("Earlier".to_owned()),
                cwds: vec![paths.comparison(&alias)],
            },
            SessionSpace {
                checkout: None,
                id: "dot-segment".to_owned(),
                label: "Dot segment".to_owned(),
                purpose: None,
                cwds: vec![paths.comparison(&dotted)],
            },
        ];
        let checkout = fs::canonicalize(&checkout).expect("the checkout's real path");
        let project = WorkspaceSnapshot {
            agent_presence: Default::default(),
            agent_scope: Default::default(),
            home_issues: Default::default(),
            pull_requests: Vec::new(),
            tasks: Default::default(),
            id: "fixture".to_owned(),
            label: "Fixture".to_owned(),
            path: root.to_string_lossy().into_owned(),
            remote_target_id: None,
            expanded: true,
            device_id: crate::node::TEST_NODE.to_owned(),
            repo_name: "fixture".to_owned(),
            is_git: false,
            default_branch: None,
            branches: Vec::new(),
            registered: true,
            temporary: false,
            session_workspace_ids: vec!["symlink".to_owned(), "dot-segment".to_owned()],
            last_activity_unix_ms: None,
            checkouts: Vec::new(),
            pinned: false,
            is_home: false,
            inactive_checkouts: Default::default(),
            session_folds: Default::default(),
            removal: Default::default(),
            disk: Default::default(),
            cleanup: None,
        };

        let authority =
            authoritative_session_space(&spaces, &project, checkout.to_string_lossy().as_ref())
                .expect("normalized checkout authority");
        let effective =
            effective_checkout_purpose(&spaces, &project, checkout.to_string_lossy().as_ref())
                .expect("normalized effective purpose");

        assert_eq!(authority.id, "dot-segment");
        assert_eq!(effective.workspace_id, "dot-segment");
        assert_eq!(effective.purpose, None);
        assert!(effective.has_shadowed_purpose);
    }

    fn listed_worktree(path: &str, branch: &str, is_main: bool) -> WorktreeSnapshot {
        WorktreeSnapshot {
            path: path.to_owned(),
            branch: Some(branch.to_owned()),
            is_main,
            ..WorktreeSnapshot::default()
        }
    }

    /// The rule R1 replaces: a worktree is a row because git lists it, not
    /// because Herdr has a pane in it. The worktree that has a pane keeps its
    /// identity and gains the counts; the ones without become rows that can
    /// be selected and started in.
    #[test]
    fn every_worktree_is_a_row_whether_or_not_a_pane_sits_in_it() {
        let (_scratch, root) = temp_dir("all-worktrees");
        let idle = root.with_file_name(format!(
            "{}-idle",
            root.file_name().unwrap().to_string_lossy()
        ));
        let second = root.with_file_name(format!(
            "{}-second",
            root.file_name().unwrap().to_string_lossy()
        ));
        fs::create_dir_all(&idle).expect("idle worktree");
        fs::create_dir_all(&second).expect("second worktree");
        let registration = registration(root.to_str().unwrap(), "Project", crate::node::TEST_NODE)
            .expect("registration");
        let worktrees = WorktreeCatalogSnapshot {
            projects: vec![ProjectWorktreesSnapshot {
                root_path: root.to_string_lossy().into_owned(),
                default_branch: Some("main".to_owned()),
                worktrees: vec![
                    WorktreeSnapshot {
                        dirty: true,
                        changed_file_count: 3,
                        ahead: 2,
                        behind: 1,
                        added_lines: 42,
                        removed_lines: 7,
                        base_branch: Some("release".to_owned()),
                        unpushed: Some(crate::model::UnpushedSnapshot {
                            remote: "origin".to_owned(),
                            count: 1,
                        }),
                        ..listed_worktree(root.to_str().unwrap(), "main", true)
                    },
                    listed_worktree(idle.to_str().unwrap(), "idle", false),
                    listed_worktree(second.to_str().unwrap(), "second", false),
                ],
                unavailable_reason: None,
                ..ProjectWorktreesSnapshot::default()
            }],
        };

        let catalog = catalog(
            &[registration],
            &[SessionSpace {
                checkout: None,
                id: "w1".to_owned(),
                label: "Project".to_owned(),
                purpose: None,
                cwds: vec![root.to_string_lossy().into_owned()],
            }],
            &worktrees,
        );

        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].default_branch.as_deref(), Some("main"));
        let rows = &catalog[0].checkouts;
        assert_eq!(rows.len(), 3, "one row per worktree, none duplicated");
        // The worktree Herdr already had a row for keeps that one row and
        // gains the counts, rather than becoming a second row beside it.
        let occupied = &rows[0];
        assert_eq!(
            occupied.path,
            path::to_wire_lossy(&fs::canonicalize(&root).unwrap()),
            "the main worktree leads"
        );
        assert!(!occupied.is_worktree);
        assert!(occupied.dirty);
        assert_eq!(occupied.changed_file_count, 3);
        assert_eq!((occupied.ahead, occupied.behind), (2, 1));
        assert_eq!((occupied.added_lines, occupied.removed_lines), (42, 7));
        assert_eq!(occupied.base_branch.as_deref(), Some("release"));
        assert_eq!(
            occupied.unpushed.as_ref().map(|unpushed| unpushed.count),
            Some(1)
        );

        let idle_row = rows.iter().find(|row| row.label == "idle").expect("idle");
        assert!(idle_row.is_worktree, "a linked worktree is marked as one");
        assert!(idle_row.exists);
        assert!(
            idle_row.tabs.is_empty(),
            "it has no pane, and that is the point"
        );
        assert!(rows.iter().any(|row| row.label == "second"));
        // Every row is keyed under the project, so a persisted selection on a
        // worktree with no pane still resolves.
        assert!(rows.iter().all(|row| row.workspace_id == catalog[0].id));
    }

    /// A worktree git lists but disk does not have keeps its row and reports
    /// that the path is gone, so the operator can see what to clean up.
    #[test]
    fn a_missing_worktree_is_a_row_that_says_it_is_missing() {
        let (_scratch, root) = temp_dir("missing-worktree");
        let registration = registration(root.to_str().unwrap(), "Project", crate::node::TEST_NODE)
            .expect("registration");
        let worktrees = WorktreeCatalogSnapshot {
            projects: vec![ProjectWorktreesSnapshot {
                root_path: root.to_string_lossy().into_owned(),
                default_branch: Some("main".to_owned()),
                worktrees: vec![
                    listed_worktree(root.to_str().unwrap(), "main", true),
                    WorktreeSnapshot {
                        missing: true,
                        ..listed_worktree("/definitely/not/here/hide-test", "gone", false)
                    },
                ],
                unavailable_reason: None,
                ..ProjectWorktreesSnapshot::default()
            }],
        };

        let catalog = catalog(&[registration], &[], &worktrees);

        let gone = catalog[0]
            .checkouts
            .iter()
            .find(|row| row.label == "gone")
            .expect("the missing worktree is still a row");
        assert!(!gone.exists);
    }

    /// Before the reader answers, and for a project it has no answer for, the
    /// rows the pane-derived catalog produced stay exactly as they were.
    #[test]
    fn a_project_with_no_worktree_answer_keeps_the_rows_it_had() {
        let (_scratch, root) = temp_dir("no-answer");
        let registration = registration(root.to_str().unwrap(), "Project", crate::node::TEST_NODE)
            .expect("registration");

        let before = catalog(std::slice::from_ref(&registration), &[], &no_worktrees());
        let unrelated = catalog(
            &[registration],
            &[],
            &WorktreeCatalogSnapshot {
                projects: vec![ProjectWorktreesSnapshot {
                    root_path: "/some/other/repository".to_owned(),
                    ..ProjectWorktreesSnapshot::default()
                }],
            },
        );

        assert_eq!(before, unrelated);
        assert_eq!(before[0].checkouts.len(), 1);
    }

    /// What a node says when each of `repositories` is the main worktree of
    /// a repository and each of `gone` is a folder it found nothing at.
    fn answered(repositories: &[&str], gone: &[&str]) -> PathIndex {
        let fact = |path: &str, repository: bool| {
            let fact = PathFact {
                comparison: path.into(),
                exists: repository,
                project: None,
                project_unread: None,
                repository: repository.then(|| RepositoryPlace {
                    root: path.into(),
                    main_root: path.into(),
                    branch: Some("main".into()),
                    head_oid: None,
                }),
            };
            (path.to_owned(), fact)
        };
        PathIndex::new(PathFacts {
            paths: repositories
                .iter()
                .map(|path| fact(path, true))
                .chain(gone.iter().map(|path| fact(path, false)))
                .collect(),
            ..PathFacts::default()
        })
    }

    /// A repository at `/repo` and a linked worktree of it whose folder was
    /// removed while a Herdr workspace attached to it kept a pane there.
    fn gone_checkout() -> (PathIndex, SessionSpace) {
        let paths = answered(&["/repo"], &["/worktrees/topic", "/worktrees/topic/src"]);
        let space = SessionSpace {
            id: "w2".to_owned(),
            label: "topic".to_owned(),
            cwds: vec!["/worktrees/topic/src".to_owned()],
            purpose: None,
            checkout: Some(HerdrCheckout {
                path: "/worktrees/topic".to_owned(),
                repository: "/repo".to_owned(),
            }),
        };
        (paths, space)
    }

    #[test]
    fn a_pane_in_a_removed_worktree_is_a_missing_checkout_of_its_repository() {
        let (paths, space) = gone_checkout();

        let catalog = build_catalog(
            &crate::node::test_node(),
            &[],
            &[space],
            &no_worktrees(),
            &paths,
        );

        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].path, "/repo");
        assert!(catalog[0].is_git);
        let rows = catalog[0]
            .checkouts
            .iter()
            .map(|row| (row.path.as_str(), row.is_worktree, row.exists))
            .collect::<Vec<_>>();
        assert_eq!(rows, [("/worktrees/topic", true, false)]);
    }

    #[test]
    fn a_registration_of_a_removed_worktree_adds_no_project() {
        let (paths, space) = gone_checkout();
        let main = space_in("w1", "/repo");
        let registrations = [
            registration("/worktrees/topic", "topic", crate::node::TEST_NODE).expect("worktree"),
            registration("/repo", "Repo", crate::node::TEST_NODE).expect("repository"),
        ];

        let catalog = build_catalog(
            &crate::node::test_node(),
            &registrations,
            &[main, space],
            &no_worktrees(),
            &paths,
        );

        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].id, registrations[1].id);
        assert_eq!(catalog[0].label, "Repo");
        let rows = catalog[0]
            .checkouts
            .iter()
            .map(|row| (row.path.as_str(), row.exists))
            .collect::<Vec<_>>();
        assert_eq!(rows, [("/repo", true), ("/worktrees/topic", false)]);
        assert_eq!(catalog[0].session_workspace_ids, ["w1", "w2"]);
    }

    #[test]
    fn a_removed_worktree_git_still_lists_stays_under_its_repository() {
        let paths = answered(&["/repo"], &["/worktrees/topic"]);
        let worktrees = WorktreeCatalogSnapshot {
            projects: vec![ProjectWorktreesSnapshot {
                root_path: "/repo".to_owned(),
                worktrees: vec![
                    listed_worktree("/repo", "main", true),
                    WorktreeSnapshot {
                        missing: true,
                        ..listed_worktree("/worktrees/topic", "topic", false)
                    },
                ],
                ..ProjectWorktreesSnapshot::default()
            }],
        };

        let catalog = build_catalog(
            &crate::node::test_node(),
            &[],
            &[space_in("w2", "/worktrees/topic")],
            &worktrees,
            &paths,
        );

        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].path, "/repo");
        let gone = catalog[0]
            .checkouts
            .iter()
            .find(|row| row.path == "/worktrees/topic")
            .expect("the removed worktree is a checkout of the repository");
        assert!(gone.is_worktree && !gone.exists);
        assert_eq!(gone.branch.as_deref(), Some("topic"));
    }

    #[test]
    fn a_removed_folder_nothing_places_stays_a_folder_of_its_own() {
        let paths = answered(&["/repo"], &["/scratch/notes"]);

        let catalog = build_catalog(
            &crate::node::test_node(),
            &[],
            &[space_in("w1", "/repo"), space_in("w3", "/scratch/notes")],
            &no_worktrees(),
            &paths,
        );

        let folder = catalog
            .iter()
            .find(|project| project.path == "/scratch/notes")
            .expect("the folder is its own row");
        assert!(!folder.is_git);
        assert_eq!(folder.checkouts.len(), 1);
        assert_eq!(catalog.len(), 2);
    }

    /// A Herdr workspace with one pane at `cwd` and no attached checkout.
    fn space_in(id: &str, cwd: &str) -> SessionSpace {
        SessionSpace {
            id: id.to_owned(),
            label: id.to_owned(),
            cwds: vec![cwd.to_owned()],
            purpose: None,
            checkout: None,
        }
    }

    #[test]
    fn a_registration_no_space_occupies_stays_listed() {
        let (_scratch, root) = temp_dir("unopened");
        let registration = registration(root.to_str().unwrap(), "Unopened", crate::node::TEST_NODE)
            .expect("registration");
        let registration_id = registration.id.clone();

        let catalog = catalog(&[registration], &[], &no_worktrees());

        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].id, registration_id);
    }
}
