//! A device's projects (PRD S5.5 B1-B4, D-03).
//!
//! The session projection (`session_sync/replica.rs`) turns each Herdr
//! workspace on a device into one row with one checkout. This module groups
//! those rows the way this machine's catalog groups its own (`workspace.rs`):
//! a project is a repository, identified by its main worktree on that device,
//! and every checkout a tab sits in is a row under it. The facts come from the
//! device's helper (`Call::Project`), never from this machine's filesystem: a
//! path on another machine means nothing here.
//!
//! A directory the helper has not answered for keeps its Herdr workspace as a
//! row of its own and the catalog says why (`DeviceCatalogSnapshot`), so an
//! unconfirmed grouping is never presented as a confirmed one. A directory
//! the helper answered it cannot read, such as a worktree whose folder was
//! removed, is placed by the device's Git worktree list or by the repository
//! Herdr recorded for the checkout (`recorded_place`), as on this machine;
//! only a folder none of them places is a row of its own.
//!
//! Ids stay scoped to the device. A project is
//! `remote:<device>:project:<sha256(device, root)>`, so the same path or the
//! same repository on two devices is two projects. A checkout is keyed by its
//! folder like this machine's (`remote:<device>:checkout:<hash of its root>`,
//! PRD checkout-workspace-binding D-10): every Herdr workspace whose tabs sit
//! in one folder is one row, and the id names no Herdr workspace. What a
//! command needs from the host is carried beside it instead: the checkout's
//! owner workspace (`checkout_owner`), and each tab's own id.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use hide_project::{ProjectFacts, ProjectKind};

use crate::model::{
    CheckoutSnapshot, DeviceCatalogRefusal, DeviceCatalogSnapshot, RemoteSessionSnapshot,
    StripTabKind, TabSnapshot, WorkspaceSnapshot,
};

/// What the device's helper said about one directory.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Fact {
    Known(ProjectFacts),
    /// The helper answered and refused: the folder is missing or unreadable.
    Refused(String),
}

/// The helper's answers for one device. A new connection asks about every
/// directory again, and an answer replaces what was known only when it comes.
#[derive(Debug, Default)]
pub(crate) struct DeviceFacts {
    pub(crate) facts: BTreeMap<String, Fact>,
    /// The helper generation a request is running against, if one is.
    pub(crate) in_flight: Option<u64>,
    /// Why the helper cannot be asked now, if it cannot.
    pub(crate) unavailable: Option<String>,
    /// The next request asks about the directories already answered too,
    /// because a new connection has not confirmed them.
    pub(crate) reread: bool,
}

impl DeviceFacts {
    fn known(&self, path: &str) -> Option<&ProjectFacts> {
        match self.facts.get(path) {
            Some(Fact::Known(facts)) => Some(facts),
            _ => None,
        }
    }
}

/// The directory a tab stands for: its first pane's, else its checkout's.
fn tab_context<'a>(tab: &'a TabSnapshot, checkout: &'a CheckoutSnapshot) -> &'a str {
    tab.panes
        .first()
        .map(|pane| pane.cwd.as_str())
        .filter(|cwd| !cwd.trim().is_empty())
        .unwrap_or(checkout.path.as_str())
}

/// Every directory the grouping needs the helper's facts for.
pub(crate) fn needed_paths(raw: &RemoteSessionSnapshot) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    for checkout in raw
        .workspaces
        .iter()
        .flat_map(|workspace| &workspace.checkouts)
    {
        if !checkout.path.trim().is_empty() {
            paths.insert(checkout.path.clone());
        }
        paths.extend(checkout.herdr_repo_root.iter().cloned());
        for tab in &checkout.tabs {
            let context = tab_context(tab, checkout);
            if !context.trim().is_empty() {
                paths.insert(context.to_owned());
            }
        }
    }
    paths
}

pub(crate) fn catalog_state(
    raw: &RemoteSessionSnapshot,
    facts: &DeviceFacts,
) -> DeviceCatalogSnapshot {
    let needed = needed_paths(raw);
    let refused = needed
        .iter()
        .filter_map(|path| match facts.facts.get(path) {
            Some(Fact::Refused(message)) => Some(DeviceCatalogRefusal {
                path: path.clone(),
                message: message.clone(),
            }),
            _ => None,
        })
        .collect::<Vec<_>>();
    let missing = needed.iter().any(|path| !facts.facts.contains_key(path));
    let (state, message) = match (missing, &facts.unavailable) {
        (false, _) => ("ready", None),
        (true, Some(reason)) => ("unavailable", Some(reason.clone())),
        (true, None) => ("resolving", None),
    };
    DeviceCatalogSnapshot {
        state: state.to_owned(),
        message,
        refused,
    }
}

pub(crate) fn project_id(target: &str, root: &Path) -> String {
    format!("remote:{target}:{}", hide_project::project_id(target, root))
}

/// The id of the checkout at `path` on `target`: its folder, never a Herdr
/// workspace, so it survives every workspace that opens or closes there.
pub(crate) fn checkout_id(target: &str, path: &str) -> String {
    format!(
        "remote:{target}:checkout:{:016x}",
        crate::workspace::fnv1a(path.trim_end_matches('/').as_bytes())
    )
}

/// The Herdr workspace behind `raw` that owns the checkout at `root`: a
/// worktree workspace Herdr binds to that folder, or a workspace carrying
/// Hide's mark for it.
fn raw_owner(
    target: &str,
    raw_workspace: &WorkspaceSnapshot,
    raw: &CheckoutSnapshot,
    root: &str,
) -> Option<String> {
    let facts = crate::checkout_owner::WorkspaceFacts {
        workspace_id: raw_workspace.session_workspace_ids.first()?,
        bound_path: raw.owner_workspace_id.as_ref().map(|_| raw.path.as_str()),
        mark: raw.owner_mark.as_deref(),
    };
    let git = crate::checkout_owner::owner_of(target, root, true, [facts]);
    let folder = crate::checkout_owner::owner_of(target, root, false, [facts]);
    git.or(folder).map(str::to_owned)
}

/// One checkout's part from one Herdr workspace, folded into the row its
/// folder already has in `project`, or added as that row.
fn merge_checkout(project: &mut WorkspaceSnapshot, part: CheckoutSnapshot) {
    let Some(row) = project
        .checkouts
        .iter_mut()
        .find(|checkout| checkout.id == part.id)
    else {
        project.checkouts.push(part);
        return;
    };
    row.tabs.extend(part.tabs);
    row.strip.extend(part.strip);
    row.has_panes |= part.has_panes;
    if row.owner_workspace_id.is_none() {
        row.owner_workspace_id = part.owner_workspace_id;
    }
    if row.purpose.is_none() {
        row.purpose = part.purpose;
    }
}

/// Where a checkout the helper answered it cannot read belongs: the deepest
/// worktree of the device's Git worktree list holding it, else the
/// repository Herdr recorded for it, by the helper's answer for that
/// repository when it has one. A folder the helper has not answered for yet
/// is not placed, so nothing is grouped before the device has spoken.
fn recorded_place(
    raw: &CheckoutSnapshot,
    facts: &DeviceFacts,
    worktrees: Option<&DeviceWorktrees>,
) -> Option<ProjectFacts> {
    if !matches!(facts.facts.get(&raw.path), Some(Fact::Refused(_))) {
        return None;
    }
    let path = Path::new(&raw.path);
    let listed = worktrees
        .into_iter()
        .flat_map(|worktrees| &worktrees.projects)
        .flat_map(|(root, project)| {
            project
                .worktrees
                .iter()
                .map(move |worktree| (root, worktree))
        })
        .filter(|(_, worktree)| path.starts_with(&worktree.path))
        .max_by_key(|(_, worktree)| worktree.path.len())
        .map(|(root, worktree)| ProjectFacts {
            root: PathBuf::from(root),
            checkout_root: PathBuf::from(&worktree.path),
            kind: ProjectKind::Git,
            branch: worktree.branch.clone(),
            linked_worktree: !worktree.is_main,
        });
    listed.or_else(|| {
        let repository = raw.herdr_repo_root.as_deref()?;
        let root = match facts.facts.get(repository)? {
            Fact::Known(known) => known.root.clone(),
            Fact::Refused(_) => PathBuf::from(repository),
        };
        Some(ProjectFacts {
            root,
            checkout_root: path.to_path_buf(),
            kind: ProjectKind::Git,
            branch: None,
            linked_worktree: raw.is_worktree,
        })
    })
}

/// The session as the device's facts group it. See the module comment.
pub(crate) fn group(
    target: &str,
    raw: &RemoteSessionSnapshot,
    facts: &DeviceFacts,
    worktrees: Option<&DeviceWorktrees>,
) -> RemoteSessionSnapshot {
    let mut projects: Vec<WorkspaceSnapshot> = Vec::new();
    // Each checkout's candidates for the tab it brings forward: the active
    // tab of every Herdr workspace with a tab there, by that workspace.
    let mut actives: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let mut grouped_ids = BTreeMap::new();
    for raw_workspace in &raw.workspaces {
        let Some(raw_checkout) = raw_workspace.checkouts.first() else {
            projects.push(raw_workspace.clone());
            continue;
        };
        let herdr_workspace = raw_workspace
            .session_workspace_ids
            .first()
            .cloned()
            .unwrap_or_default();
        let raw_active = raw.active_tab_ids.get(&raw_checkout.id);
        let recorded = recorded_place(raw_checkout, facts, worktrees);
        let Some(primary) = facts.known(&raw_checkout.path).or(recorded.as_ref()) else {
            // Unconfirmed: the folder is a row of its own, named by the
            // folder, never grouped by a guess.
            let id = checkout_id(target, &raw_checkout.path);
            grouped_ids.insert(raw_checkout.id.clone(), id.clone());
            if let Some(tab) = raw_active {
                actives
                    .entry(id.clone())
                    .or_default()
                    .push((herdr_workspace.clone(), tab.clone()));
            }
            let name = crate::workspace::checkout_row_label(None, Path::new(&raw_checkout.path));
            let part = CheckoutSnapshot {
                agent_scope: Default::default(),
                id: id.clone(),
                label: name.clone(),
                owner_workspace_id: raw_owner(
                    target,
                    raw_workspace,
                    raw_checkout,
                    &raw_checkout.path,
                ),
                owner_mark: None,
                unconfirmed: true,
                tabs: raw_checkout
                    .tabs
                    .iter()
                    .map(|tab| TabSnapshot {
                        checkout_id: Some(id.clone()),
                        ..tab.clone()
                    })
                    .collect(),
                ..raw_checkout.clone()
            };
            match projects
                .iter_mut()
                .find(|project| project.checkouts.iter().any(|checkout| checkout.id == id))
            {
                Some(project) => {
                    let part = CheckoutSnapshot {
                        agent_scope: Default::default(),
                        workspace_id: project.id.clone(),
                        tabs: part
                            .tabs
                            .into_iter()
                            .map(|tab| TabSnapshot {
                                workspace_id: Some(project.id.clone()),
                                ..tab
                            })
                            .collect(),
                        ..part
                    };
                    if !project.session_workspace_ids.contains(&herdr_workspace) {
                        project.session_workspace_ids.push(herdr_workspace.clone());
                    }
                    merge_checkout(project, part);
                }
                None => projects.push(WorkspaceSnapshot {
                    agent_scope: Default::default(),
                    label: name.clone(),
                    repo_name: name,
                    checkouts: vec![part],
                    ..raw_workspace.clone()
                }),
            }
            continue;
        };
        // Tabs split by the checkout their directory is in; a tab whose
        // directory has no answer stays with the workspace's own checkout.
        let mut parts: Vec<(&ProjectFacts, Vec<&TabSnapshot>)> = vec![(primary, Vec::new())];
        for tab in &raw_checkout.tabs {
            let facts = facts
                .known(tab_context(tab, raw_checkout))
                .unwrap_or(primary);
            match parts
                .iter_mut()
                .find(|(known, _)| known.checkout_root == facts.checkout_root)
            {
                Some((_, tabs)) => tabs.push(tab),
                None => parts.push((facts, vec![tab])),
            }
        }
        for (index, (facts, tabs)) in parts.into_iter().enumerate() {
            let project_id = project_id(target, &facts.root);
            let checkout_root = facts.checkout_root.to_string_lossy().into_owned();
            let checkout_id = checkout_id(target, &checkout_root);
            if index == 0 {
                grouped_ids.insert(raw_checkout.id.clone(), checkout_id.clone());
            }
            let tabs = tabs
                .into_iter()
                .map(|tab| TabSnapshot {
                    workspace_id: Some(project_id.clone()),
                    checkout_id: Some(checkout_id.clone()),
                    ..tab.clone()
                })
                .collect::<Vec<_>>();
            if let Some(tab) =
                raw_active.filter(|active| tabs.iter().any(|tab| tab.id.as_ref() == Some(*active)))
            {
                actives
                    .entry(checkout_id.clone())
                    .or_default()
                    .push((herdr_workspace.clone(), tab.clone()));
            }
            // The Herdr entries of this checkout's tabs; the runtime adds the
            // file tabs (`place_device_strips`).
            let strip = raw_checkout
                .strip
                .iter()
                .filter(|entry| {
                    entry.kind == StripTabKind::Herdr
                        && tabs
                            .iter()
                            .any(|tab| tab.id.as_ref() == Some(&entry.source_id))
                })
                .cloned()
                .collect::<Vec<_>>();
            let checkout = CheckoutSnapshot {
                agent_scope: Default::default(),
                id: checkout_id,
                workspace_id: project_id.clone(),
                label: crate::workspace::checkout_row_label(
                    facts.branch.as_deref(),
                    &facts.checkout_root,
                ),
                path: checkout_root.clone(),
                branch: facts.branch.clone(),
                is_worktree: facts.linked_worktree,
                is_primary: !facts.linked_worktree && facts.kind == ProjectKind::Git,
                // A checkout only the worktree list or Herdr placed is one
                // the helper could not read.
                exists: index != 0 || recorded.is_none(),
                has_panes: !tabs.is_empty(),
                purpose: if index == 0 {
                    raw_checkout.purpose.clone()
                } else {
                    None
                },
                owner_workspace_id: if index == 0 {
                    raw_owner(target, raw_workspace, raw_checkout, &checkout_root)
                } else {
                    None
                },
                owner_mark: None,
                active_tab_id: None,
                strip,
                tabs,
                ..raw_checkout.clone()
            };
            let root = facts.root.to_string_lossy().into_owned();
            let project = match projects.iter().position(|project| project.id == project_id) {
                Some(position) => &mut projects[position],
                None => {
                    let name = facts
                        .root
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| root.clone());
                    projects.push(WorkspaceSnapshot {
                        agent_scope: Default::default(),
                        id: project_id.clone(),
                        label: name.clone(),
                        path: root.clone(),
                        repo_name: name,
                        is_git: facts.kind == ProjectKind::Git,
                        default_branch: None,
                        session_workspace_ids: Vec::new(),
                        checkouts: Vec::new(),
                        last_activity_unix_ms: None,
                        ..raw_workspace.clone()
                    });
                    projects.last_mut().expect("just pushed")
                }
            };
            for id in &raw_workspace.session_workspace_ids {
                if !project.session_workspace_ids.contains(id) {
                    project.session_workspace_ids.push(id.clone());
                }
            }
            if facts.checkout_root == facts.root {
                project.default_branch = facts.branch.clone();
            }
            merge_checkout(project, checkout);
        }
    }
    // The tab a merged checkout brings forward when the runtime remembers
    // none for it (`bring_recent_device_tabs`, D-13): Herdr's focused tab
    // when it is there, else its owner's active tab, else the first
    // workspace's.
    let mut active_tab_ids = BTreeMap::new();
    for project in &mut projects {
        for checkout in &mut project.checkouts {
            let candidates = actives.get(&checkout.id).map_or(&[][..], Vec::as_slice);
            let focused = raw.focused_tab_id.as_ref().filter(|focused| {
                checkout
                    .tabs
                    .iter()
                    .any(|tab| tab.id.as_ref() == Some(*focused))
            });
            let owner = checkout.owner_workspace_id.as_ref().and_then(|owner| {
                candidates
                    .iter()
                    .find(|(workspace, _)| workspace == owner)
                    .map(|(_, tab)| tab)
            });
            let chosen = focused
                .or(owner)
                .or_else(|| candidates.first().map(|(_, tab)| tab))
                .cloned();
            checkout.active_tab_id = chosen.clone();
            if let Some(tab) =
                chosen.or_else(|| checkout.tabs.first().and_then(|tab| tab.id.clone()))
            {
                active_tab_ids.insert(checkout.id.clone(), tab);
            }
        }
    }
    // The main worktree leads, as on this machine.
    for project in &mut projects {
        if let Some(index) = project
            .checkouts
            .iter()
            .position(|checkout| checkout.path == project.path)
            && index != 0
        {
            let main = project.checkouts.remove(index);
            project.checkouts.insert(0, main);
        }
    }
    crate::agent_state::sync_checkout_agent_summaries(&mut projects, &raw.agents);
    crate::project_context::sort_projects(&mut projects, &raw.agents);

    let find_tab = |tab_id: &str| {
        projects.iter().find_map(|project| {
            project.checkouts.iter().find_map(|checkout| {
                checkout
                    .tabs
                    .iter()
                    .any(|tab| tab.id.as_deref() == Some(tab_id))
                    .then(|| (project.id.clone(), checkout.id.clone()))
            })
        })
    };
    let find_checkout = |checkout_id: &str| {
        projects.iter().find_map(|project| {
            project
                .checkouts
                .iter()
                .any(|checkout| checkout.id == checkout_id)
                .then(|| (project.id.clone(), checkout_id.to_owned()))
        })
    };
    let focused = raw
        .focused_tab_id
        .as_deref()
        .and_then(find_tab)
        .or_else(|| {
            raw.focused_checkout_id
                .as_ref()
                .and_then(|raw_id| grouped_ids.get(raw_id))
                .and_then(|id| find_checkout(id))
        });
    RemoteSessionSnapshot {
        workspaces: projects,
        agents: raw.agents.clone(),
        active_tab_ids,
        focused_workspace_id: focused.as_ref().map(|(project, _)| project.clone()),
        focused_checkout_id: focused.map(|(_, checkout)| checkout),
        focused_tab_id: raw.focused_tab_id.clone(),
        focused_pane_id: raw.focused_pane_id.clone(),
        pane_layouts: raw.pane_layouts.clone(),
        pane_hook_tokens: raw.pane_hook_tokens.clone(),
        issue_tokens: raw.issue_tokens.clone(),
        issue_links: BTreeMap::new(),
        herdr_version: raw.herdr_version.clone(),
    }
}

/// Device checkout ids saved before a checkout was keyed by its folder
/// (`remote:<device>:checkout:<workspace>`, `…:<workspace>#<hash>`, and a
/// registration's `<project>#registered`), each mapped to the folder-keyed id
/// that now names the same checkout, so a fold saved under the old id
/// survives the change (PRD checkout-workspace-binding B11). An
/// old id names a Herdr workspace, so it maps only while that workspace is
/// in the device's session; the second form carries the folder's hash.
pub(crate) fn legacy_checkout_ids<'a>(
    target: &str,
    raw: &RemoteSessionSnapshot,
    grouped: &RemoteSessionSnapshot,
    registrations: &[crate::model::WorkspaceRegistration],
    saved: impl IntoIterator<Item = &'a String>,
) -> BTreeMap<String, String> {
    let prefix = format!("remote:{target}:checkout:");
    let grouped_holding = |tab_id: &str| {
        grouped
            .workspaces
            .iter()
            .flat_map(|project| project.checkouts.iter())
            .find(|checkout| {
                checkout
                    .tabs
                    .iter()
                    .any(|tab| tab.id.as_deref() == Some(tab_id))
            })
            .map(|checkout| checkout.id.clone())
    };
    let mut renamed = BTreeMap::new();
    for id in saved {
        let new = if let Some(project) = id.strip_suffix("#registered") {
            registrations
                .iter()
                .find(|registration| registration.device_id == target && registration.id == project)
                .map(|registration| checkout_id(target, &registration.path))
        } else if let Some(rest) = id.strip_prefix(&prefix) {
            match rest.split_once('#') {
                Some((_, hash)) => Some(format!("{prefix}{hash}")),
                None => raw
                    .workspaces
                    .iter()
                    .flat_map(|workspace| workspace.checkouts.iter())
                    .find(|checkout| &checkout.id == id)
                    .map(|checkout| {
                        checkout
                            .tabs
                            .iter()
                            .find_map(|tab| grouped_holding(tab.id.as_deref()?))
                            .unwrap_or_else(|| checkout_id(target, &checkout.path))
                    }),
            }
        } else {
            None
        };
        if let Some(new) = new.filter(|new| new != id) {
            renamed.insert(id.clone(), new);
        }
    }
    renamed
}

/// A device's registrations carried onto its grouped session (B3, B23-B25).
///
/// A registered project keeps its row while Herdr has no workspace in it: its
/// main checkout is listed without tabs under its folder's checkout id, the
/// same id the row keeps once a Herdr workspace opens there (D-10). A
/// registered row carries its pin, and every row what `Remove project…`
/// would close, counted from the device's own session: a row without a
/// registration offers the removal too (PRD sidebar-context-menus D-14).
pub(crate) fn apply_registrations(
    target: &str,
    session: &mut RemoteSessionSnapshot,
    registrations: &[crate::model::WorkspaceRegistration],
    facts: &DeviceFacts,
) {
    let registrations = registrations
        .iter()
        .filter(|registration| registration.device_id == target)
        .collect::<Vec<_>>();
    for project in &mut session.workspaces {
        let registration = registrations
            .iter()
            .find(|registration| registration.id == project.id);
        project.registered = registration.is_some();
        project.pinned = registration.is_some_and(|registration| registration.pinned);
        project.is_home = registration.is_some_and(|registration| registration.home);
    }
    for registration in &registrations {
        if session
            .workspaces
            .iter()
            .any(|project| project.id == registration.id)
        {
            continue;
        }
        let known = facts.known(&registration.path);
        let branch = known.and_then(|facts| facts.branch.clone());
        session.workspaces.push(WorkspaceSnapshot {
            agent_presence: Default::default(),
            agent_scope: Default::default(),
            home_issues: Default::default(),
            pull_requests: Vec::new(),
            tasks: Default::default(),
            id: registration.id.clone(),
            label: registration.label.clone(),
            path: registration.path.clone(),
            remote_target_id: Some(target.to_owned()),
            expanded: true,
            device_id: target.to_owned(),
            repo_name: registration.label.clone(),
            is_git: known.is_some_and(|facts| facts.kind == ProjectKind::Git),
            default_branch: branch.clone(),
            branches: Vec::new(),
            registered: true,
            temporary: false,
            session_workspace_ids: Vec::new(),
            last_activity_unix_ms: None,
            pinned: registration.pinned,
            is_home: registration.home,
            checkouts: vec![CheckoutSnapshot {
                id: checkout_id(target, &registration.path),
                next_tab_label: crate::model::next_tab_label(std::iter::empty()),
                workspace_id: registration.id.clone(),
                label: crate::workspace::checkout_row_label(
                    branch.as_deref(),
                    Path::new(&registration.path),
                ),
                path: registration.path.clone(),
                branch,
                exists: true,
                is_primary: known
                    .is_some_and(|facts| facts.kind == ProjectKind::Git && !facts.linked_worktree),
                unconfirmed: known.is_none(),
                ..CheckoutSnapshot::default()
            }],
            inactive_checkouts: Default::default(),
            session_folds: Default::default(),
            removal: Default::default(),
            disk: Default::default(),
            cleanup: None,
        });
    }
    crate::agent_state::sync_workspace_removals(&mut session.workspaces, &session.agents, false);
    crate::project_context::sort_projects(&mut session.workspaces, &session.agents);
}

/// A device's repositories' worktrees as its helper last answered, keyed by
/// main worktree path, and the read in flight.
#[derive(Clone, Debug, Default)]
pub(crate) struct DeviceWorktrees {
    pub projects: BTreeMap<String, crate::model::ProjectWorktreesSnapshot>,
    /// Each repository's branch notes (`branch.<b>.issue`), by the root the
    /// session names it by, read with its worktrees.
    pub notes: BTreeMap<String, BTreeMap<String, hide_node_link::catalog::BranchNote>>,
    /// The helper connection the running read belongs to.
    pub in_flight: Option<u64>,
    /// Asked again while a read ran; one more read follows it.
    pub again: bool,
    /// A confirmed base changed while this bounded worker was reading.
    pub discard_answer: bool,
    pub unavailable: Option<String>,
    /// The next read covers every repository, because a new connection has
    /// not confirmed what the last one listed.
    pub reread: bool,
    /// Repositories the device's Git watch saw change, read on the next read.
    pub stale: BTreeSet<String>,
    /// A read that failed is tried again, every repository, at this time
    /// (`Runtime::retry_device_reads`), so a change it carried is not lost.
    pub retry_at: Option<std::time::Instant>,
}

/// The Git repositories a device's grouped session shows, by main worktree.
pub(crate) fn git_roots(session: &RemoteSessionSnapshot) -> BTreeSet<String> {
    session
        .workspaces
        .iter()
        .filter(|workspace| workspace.is_git)
        .map(|workspace| workspace.path.clone())
        .collect()
}

/// Attributes a device's listeners to its panes, each to the panes at or
/// above the folder it was started in, as this machine's are to its own.
pub(crate) fn apply_ports(
    session: &mut RemoteSessionSnapshot,
    listeners: &[crate::model::ListeningPortSnapshot],
) {
    for pane in session
        .workspaces
        .iter_mut()
        .flat_map(|workspace| workspace.checkouts.iter_mut())
        .flat_map(|checkout| checkout.tabs.iter_mut())
        .flat_map(|tab| tab.panes.iter_mut())
    {
        pane.ports = crate::ports::attributed_ports(&pane.cwd, listeners);
        pane.servers = crate::ports::attributed_servers(&pane.cwd, listeners);
    }
}

/// Carries a device repository's worktree facts onto the rows its session
/// already has. Paths are compared as the device's helper reported them;
/// nothing is resolved on this machine's filesystem, where the same path may
/// name another repository (B2, B7).
///
/// For a node that dials the core (`own_rows`, its id), a listed worktree
/// the session has no checkout for gets one, as this machine's own projects
/// do (`workspace::apply_worktrees`, PRD core-host-node-move D-29): a
/// worktree with no pane open is still one of the node's checkouts, and the
/// id its folder names (`checkout_id`) is the one a move gives it. A device
/// the core dials keeps only the checkouts its session opens.
pub(crate) fn apply_worktrees(
    project: &mut WorkspaceSnapshot,
    listed: &crate::model::ProjectWorktreesSnapshot,
    own_rows: Option<&str>,
) {
    let same = |left: &str, right: &str| left.trim_end_matches('/') == right.trim_end_matches('/');
    project.default_branch = listed.default_branch.clone();
    project.branches = listed.branches.clone();
    if let Some(target) = own_rows {
        for worktree in &listed.worktrees {
            if project
                .checkouts
                .iter()
                .any(|row| same(&row.path, &worktree.path))
            {
                continue;
            }
            project.checkouts.push(CheckoutSnapshot {
                id: checkout_id(target, &worktree.path),
                workspace_id: project.id.clone(),
                label: crate::workspace::checkout_row_label(
                    worktree.branch.as_deref(),
                    Path::new(&worktree.path),
                ),
                path: worktree.path.clone(),
                branch: worktree.branch.clone(),
                is_primary: worktree.is_main,
                next_tab_label: crate::model::next_tab_label(std::iter::empty()),
                ..CheckoutSnapshot::default()
            });
        }
    }
    for row in &mut project.checkouts {
        let Some(worktree) = listed
            .worktrees
            .iter()
            .find(|worktree| same(&worktree.path, &row.path))
        else {
            continue;
        };
        let mut worktree = worktree.clone();
        worktree.pane_count = row.tabs.iter().map(|tab| tab.panes.len()).sum();
        worktree.deletion_gate = crate::worktrees::deletion_gate(
            &worktree,
            worktree.branch == listed.base_branch && listed.base_branch.is_some(),
            worktree.pane_count,
        );
        row.is_worktree = !worktree.is_main;
        row.exists = !worktree.missing;
        row.dirty = worktree.dirty;
        row.changed_file_count = worktree.changed_file_count;
        row.base_branch = worktree.base_branch.clone();
        row.ahead = worktree.ahead;
        row.behind = worktree.behind;
        row.added_lines = worktree.added_lines;
        row.removed_lines = worktree.removed_lines;
        row.unpushed = worktree.unpushed.clone();
        row.branch = worktree.branch.clone();
        row.worktree = Some(worktree);
    }
}
