//! A device's projects from its helper's facts (`crate::device_catalog`).
//!
//! The runtime keeps each device's session as Herdr reported it and derives
//! the published one from it and the facts the device's helper has answered.
//! The facts are asked on a worker with the runtime lock released, once per
//! directory; a new directory in the session, or a new helper connection,
//! asks again. They are kept while the device is, up to a cap.

use super::*;
use crate::device_catalog::{self, DeviceFacts, Fact};
use crate::node_access::{LinkError, call_as};
use hide_project::ProjectFacts;
use std::time::Duration;

const FACTS_TIMEOUT: Duration = Duration::from_secs(15);
/// The directories, and the repositories, a device's answers are kept for.
/// They live as long as the device, so a pane back in a folder it left is
/// grouped at once; past this only what its session still shows is kept.
pub(super) const MAX_KNOWN_PATHS: usize = 1024;

type FactAnswers = Vec<(String, Fact)>;

impl Runtime {
    /// The published session for a device, grouped by the facts known now;
    /// its strips are placed by `place_device_strips`.
    pub(super) fn derive_device_session(
        &self,
        target: &str,
        raw: &RemoteSessionSnapshot,
    ) -> RemoteSessionSnapshot {
        let empty = DeviceFacts::default();
        let facts = self.device_facts.get(target).unwrap_or(&empty);
        let mut session = device_catalog::group(target, raw, facts);
        device_catalog::apply_registrations(
            target,
            &mut session,
            &self.snapshot.ui_state.workspace_registrations,
            facts,
        );
        let reports_own_facts = self
            .link_origin(target)
            .is_some_and(LinkOrigin::reports_own_facts);
        let own_rows = reports_own_facts.then_some(target);
        if let Some(worktrees) = self.device_worktrees.get(target) {
            let measured = self.device_disk.get(target);
            for project in &mut session.workspaces {
                let Some(listed) = worktrees.projects.get(&project.path) else {
                    // A folder project is measured as one folder (D-27).
                    if !project.is_git && reports_own_facts {
                        let (measuring, queued) =
                            self.disk_sweep_marks(Some(target), &project.path);
                        project.disk = super::disk_sweep::folder_disk(
                            measured.map_or(&[][..], Vec::as_slice),
                            &project.path,
                            measuring,
                            queued,
                        );
                    }
                    continue;
                };
                match measured {
                    Some(disk) => {
                        let mut listed = listed.clone();
                        super::projects::measure_worktrees(&mut listed, disk);
                        device_catalog::apply_worktrees(project, &listed, own_rows);
                        let marks = self.disk_sweep_marks(Some(target), &project.path);
                        project.disk = super::projects::project_disk(Some(&listed), marks);
                    }
                    None => device_catalog::apply_worktrees(project, listed, own_rows),
                }
                if let Some(found) = self
                    .device_github
                    .get(target)
                    .and_then(|github| github.project(&project.path))
                {
                    super::projects::associate_pull_requests(project, Some(found));
                    // A node's project lists its pull requests on its PRs tab
                    // as this machine's does (D-29).
                    if reports_own_facts && project.is_git {
                        project.pull_requests = super::pull_requests::shown_pull_requests(
                            &found.pull_requests,
                            &project.checkouts,
                            unix_milliseconds(),
                        );
                        super::links::associate_pr_sessions(
                            &mut project.pull_requests,
                            self.snapshot
                                .link_summaries
                                .as_ref()
                                .and_then(|summaries| summaries.projects.get(&project.id)),
                        );
                    }
                }
            }
        }
        if reports_own_facts {
            self.project_node_issues(target, &mut session);
            for project in &mut session.workspaces {
                project.cleanup = self
                    .cleanup
                    .as_ref()
                    .filter(|review| review.workspace_id == project.id)
                    .cloned();
            }
        }
        if let Some(listeners) = self.device_ports.get(target) {
            device_catalog::apply_ports(&mut session, listeners);
        }
        self.judge_device_pane_children(target, &mut session);
        self.agent_scope_cache
            .restore_projects(&mut session.workspaces);
        let agents = session
            .agents
            .iter()
            .map(|agent| (agent.pane_id.as_str(), agent))
            .collect();
        let now = unix_milliseconds();
        let focused = crate::project_context::device_fold_focus(
            self.snapshot.navigator.focused_checkout_id.as_deref(),
            &session,
        );
        for workspace in &mut session.workspaces {
            crate::project_context::refresh_device_folds(
                workspace,
                &agents,
                &self.snapshot.navigator.sleeping_sessions,
                &self.snapshot.ui_state,
                focused.as_deref(),
                reports_own_facts,
                now,
            );
        }
        self.project_catalog_agent_groups(&mut session.workspaces);
        session
    }

    /// What each device agent pane's header says about the work it
    /// delegated, judged by the same function as a local pane's, from the
    /// pane's hook tokens and the device's own kit (PRD device-parity D-21,
    /// B14). A device whose kit Hide has not read yet leaves the cause
    /// unknown rather than guessing it.
    fn judge_device_pane_children(&self, target: &str, session: &mut RemoteSessionSnapshot) {
        let declined = self
            .device_kit_declined(target)
            .is_some_and(|declined| declined.installed_nothing());
        let kit = self.kit_state(target);
        let status_of = |dialect: hide_agent_adapter::HookDialect| {
            crate::agent_hooks::device_hook_status(declined, &kit, dialect)
        };
        let agents = &session.agents;
        let tokens = &session.pane_hook_tokens;
        for pane in session
            .workspaces
            .iter_mut()
            .flat_map(|project| project.checkouts.iter_mut())
            .flat_map(|checkout| checkout.tabs.iter_mut())
            .flat_map(|tab| tab.panes.iter_mut())
        {
            pane.children = crate::sidebar::project_pane_children_connected(
                agents,
                &pane.id,
                tokens.get(&pane.id).copied().unwrap_or_default(),
                &status_of,
                kit.shares_codex_server(),
            );
        }
    }

    /// Recomputes a device's published session and catalog state from what
    /// is known now. Returns whether either changed.
    pub(super) fn refresh_device_catalog(&mut self, target: &str) -> bool {
        let Some(raw) = self.device_raw_sessions.get(target) else {
            return false;
        };
        let mut session = self.derive_device_session(target, raw);
        let empty = DeviceFacts::default();
        let catalog =
            device_catalog::catalog_state(raw, self.device_facts.get(target).unwrap_or(&empty));
        self.bring_recent_device_tabs(target, &mut session);
        self.rename_saved_device_checkouts(target, &session);
        let mut dropped_moves = Vec::new();
        self.place_device_strips(target, &mut session, &mut dropped_moves);
        if !dropped_moves.is_empty() {
            self.report_dropped_tab_moves(dropped_moves);
        }
        let Some(index) = self
            .snapshot
            .status
            .remote
            .iter()
            .position(|status| status.target_id == target)
        else {
            return false;
        };
        let status = &mut self.snapshot.status.remote[index];
        let mut changed = false;
        if status.catalog != catalog {
            status.catalog = catalog;
            changed = true;
        }
        if status.session.as_ref() != Some(&session) {
            let before = status.session.replace(session);
            // Rows regrouped from Herdr's raw session carry no lineage, which
            // only the pass across every machine derives; a subtree close
            // waits on it, so the session is never published without it.
            self.refresh_agent_lineage();
            changed |= self.snapshot.status.remote[index].session != before;
        }
        if changed {
            self.refresh_agent_sessions();
            self.reconcile_node_agent_topology(target);
            self.prune_device_view_callers(target);
            self.repoint_device_editor_tabs(target);
            // A device Workspace in front waited for its catalog to bring
            // its View tabs back.
            self.restore_front_when_ready();
        }
        changed |= self.refresh_browser_inventory_scope();
        changed
    }

    /// A device tab Herdr no longer lists loses its View bookmark (D-11).
    pub(super) fn device_catalog_ready(&self, target: &str) -> bool {
        self.snapshot
            .status
            .remote
            .iter()
            .find(|status| status.target_id == target)
            .is_some_and(|status| {
                status.catalog.state == "ready" && status.catalog.refused.is_empty()
            })
    }

    /// Only an answered catalog can prove that an empty registered checkout
    /// has no tabs: while grouping is unresolved it is a placeholder.
    pub(super) fn prune_device_view_callers(&mut self, target: &str) {
        let empty_authoritative = self.device_catalog_ready(target);
        let Some(session) = self
            .snapshot
            .status
            .remote
            .iter()
            .find(|status| status.target_id == target)
            .and_then(|status| status.session.as_ref())
        else {
            return;
        };
        let listed: Vec<(workspace_view::WorkspaceKey, Vec<String>)> = session
            .workspaces
            .iter()
            .flat_map(|project| {
                project.checkouts.iter().map(|checkout| {
                    (
                        (project.device_id.clone(), checkout.path.clone()),
                        checkout
                            .tabs
                            .iter()
                            .filter_map(|tab| tab.id.clone())
                            .collect::<Vec<_>>(),
                    )
                })
            })
            .filter(|(_, tabs)| !tabs.is_empty() || empty_authoritative)
            .collect();
        let mut pruned = false;
        for (key, tabs) in &listed {
            let tabs: Vec<&str> = tabs.iter().map(String::as_str).collect();
            pruned |= self.prune_view_callers(key, &tabs);
        }
        if pruned {
            self.persist_workspace_views();
        }
    }

    /// Carries folds saved under a device checkout's old, workspace-keyed id
    /// over to its folder-keyed id (`device_catalog::legacy_checkout_ids`); a
    /// device's selection is not saved (`ui_state.focused_checkout_id` holds
    /// only this machine's), since the device's own Herdr focus picks it.
    fn rename_saved_device_checkouts(&mut self, target: &str, session: &RemoteSessionSnapshot) {
        let Some(raw) = self.device_raw_sessions.get(target) else {
            return;
        };
        let ui = &self.snapshot.ui_state;
        let saved = ui
            .expanded_checkout_ids
            .iter()
            .chain(&ui.collapsed_checkout_ids);
        let renamed = device_catalog::legacy_checkout_ids(
            target,
            raw,
            session,
            &ui.workspace_registrations,
            saved,
        );
        if renamed.is_empty() {
            return;
        }
        let rename = |id: &mut String| {
            if let Some(new) = renamed.get(id.as_str()) {
                *id = new.clone();
            }
        };
        let ui = &mut self.snapshot.ui_state;
        ui.expanded_checkout_ids.iter_mut().for_each(rename);
        ui.collapsed_checkout_ids.iter_mut().for_each(rename);
        ui.expanded_checkout_ids.sort();
        ui.expanded_checkout_ids.dedup();
        ui.collapsed_checkout_ids.sort();
        ui.collapsed_checkout_ids.dedup();
        crate::diagnostic!(serde_json::json!({
            "component": "device_catalog",
            "kind": "catalog.checkout_ids_renamed",
            "target": target,
            "count": renamed.len(),
        }));
        self.persist_current_ui_state();
    }

    /// Remembers the tab the device's Herdr has in focus under the checkout
    /// holding it, and makes each checkout's remembered tab, while it still
    /// exists, the one its row brings forward; the grouping's own choice is
    /// the fallback for a checkout with nothing remembered.
    fn bring_recent_device_tabs(&mut self, target: &str, session: &mut RemoteSessionSnapshot) {
        let recent = self
            .device_recent_tabs
            .entry(target.to_owned())
            .or_default();
        if let (Some(checkout), Some(tab)) = (
            session.focused_checkout_id.as_ref(),
            session.focused_tab_id.as_ref(),
        ) {
            recent.insert(checkout.clone(), tab.clone());
        }
        let listed = session
            .workspaces
            .iter()
            .flat_map(|project| project.checkouts.iter())
            .map(|checkout| checkout.id.as_str())
            .collect::<HashSet<_>>();
        recent.retain(|checkout, _| listed.contains(checkout.as_str()));
        for checkout in session
            .workspaces
            .iter_mut()
            .flat_map(|project| project.checkouts.iter_mut())
        {
            let Some(tab) = recent.get(&checkout.id).filter(|tab| {
                checkout
                    .tabs
                    .iter()
                    .any(|listed| listed.id.as_ref() == Some(*tab))
            }) else {
                continue;
            };
            checkout.active_tab_id = Some(tab.clone());
            session
                .active_tab_ids
                .insert(checkout.id.clone(), tab.clone());
        }
    }

    /// A file tab names its checkout's project; grouping moves a checkout
    /// into a project, so its tabs follow it there.
    fn repoint_device_editor_tabs(&mut self, target: &str) {
        let Some(session) = self
            .snapshot
            .status
            .remote
            .iter()
            .find(|status| status.target_id == target)
            .and_then(|status| status.session.as_ref())
        else {
            return;
        };
        let owners = session
            .workspaces
            .iter()
            .flat_map(|workspace| {
                workspace
                    .checkouts
                    .iter()
                    .map(|checkout| (checkout.id.clone(), workspace.id.clone()))
            })
            .collect::<HashMap<_, _>>();
        for tab in &mut self.snapshot.editor.tabs {
            if let Some(owner) = owners.get(&tab.checkout_id)
                && &tab.workspace_id != owner
            {
                tab.workspace_id = owner.clone();
            }
        }
    }

    /// Asks the device's helper for the facts of every directory its session
    /// names that has no answer yet, unless a request is already running.
    pub(super) fn request_device_facts(&mut self, target: &str) -> bool {
        let Some(raw) = self.device_raw_sessions.get(target) else {
            return false;
        };
        let known = self.device_facts.get(target);
        // One request at a time; and a helper that could not be reached is
        // not asked again on every session sync, only when a connection is
        // established (`reread_device_facts`).
        if known.is_some_and(|facts| facts.in_flight.is_some() || facts.unavailable.is_some()) {
            return self.refresh_device_catalog(target);
        }
        let registered = self
            .snapshot
            .ui_state
            .workspace_registrations
            .iter()
            .filter(|registration| registration.device_id == target)
            .map(|registration| registration.path.clone());
        let needed = device_catalog::needed_paths(raw)
            .into_iter()
            .chain(registered)
            .collect::<BTreeSet<_>>();
        let missing = needed
            .iter()
            .filter(|path| {
                known.is_none_or(|facts| facts.reread || !facts.facts.contains_key(*path))
            })
            .cloned()
            .collect::<Vec<_>>();
        if let Some(entry) = self.device_facts.get_mut(target)
            && entry.facts.len() > MAX_KNOWN_PATHS
        {
            let known = entry.facts.len();
            entry.facts.retain(|path, _| needed.contains(path));
            if entry.facts.len() < known {
                crate::diagnostic!(serde_json::json!({
                    "component": "device_catalog",
                    "kind": "catalog.facts_pruned",
                    "target": target,
                    "known": known,
                    "kept": entry.facts.len(),
                }));
            }
        }
        if missing.is_empty() {
            return self.refresh_device_catalog(target);
        }
        if self.device_host_connecting(target) {
            return self.refresh_device_catalog(target);
        }
        let channel = match self.node_link(target) {
            Ok(channel) => channel,
            Err(_) if self.device_host_connecting(target) => {
                return self.refresh_device_catalog(target);
            }
            Err(reason) => {
                self.device_facts
                    .entry(target.to_owned())
                    .or_default()
                    .unavailable = Some(reason);
                return self.refresh_device_catalog(target);
            }
        };
        let generation = self.device_host_generation(target);
        let entry = self.device_facts.entry(target.to_owned()).or_default();
        entry.in_flight = Some(generation);
        entry.unavailable = None;
        entry.reread = false;
        crate::diagnostic!(serde_json::json!({
            "component": "device_catalog",
            "kind": "catalog.facts_requested",
            "target": target,
            "generation": generation,
            "paths": missing.len(),
        }));
        let Some(context) = self.worker_context.clone() else {
            let (answers, failure) = ask_facts(channel.as_ref(), &missing);
            return self.ingest_device_facts(target, generation, answers, failure);
        };
        let device = target.to_owned();
        let spawned = thread::Builder::new()
            .name("herdr-core-device-catalog".to_owned())
            .spawn(move || {
                let (answers, failure) = ask_facts(channel.as_ref(), &missing);
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                let changed = match runtime.lock() {
                    Ok(mut guard) => {
                        guard.ingest_device_facts(&device, generation, answers, failure)
                    }
                    Err(_) => return,
                };
                drop(runtime);
                if changed {
                    context.notifier.notify();
                }
            });
        if let Err(error) = spawned {
            let entry = self.device_facts.entry(target.to_owned()).or_default();
            entry.in_flight = None;
            entry.unavailable = Some(format!("The catalog reader could not start: {error}"));
        }
        self.refresh_device_catalog(target)
    }

    /// The helper answered. An answer from a connection that has since been
    /// replaced is dropped; the new connection asks again.
    pub(super) fn ingest_device_facts(
        &mut self,
        target: &str,
        generation: u64,
        answers: FactAnswers,
        failure: Option<String>,
    ) -> bool {
        let Some(entry) = self.device_facts.get_mut(target) else {
            return false;
        };
        if entry.in_flight != Some(generation) {
            return false;
        }
        entry.in_flight = None;
        let answered = answers.len();
        for (path, fact) in answers {
            entry.facts.insert(path, fact);
        }
        entry.unavailable = failure.clone();
        crate::diagnostic!(serde_json::json!({
            "component": "device_catalog",
            "kind": "catalog.facts_answered",
            "target": target,
            "generation": generation,
            "answered": answered,
            "failure": failure,
        }));
        let mut changed = self.refresh_device_catalog(target);
        // The session may have named new directories while this ran.
        if failure.is_none() {
            changed |= self.request_device_facts(target);
            changed |= self.request_device_worktrees(target, false);
        }
        changed
    }

    /// Reads the worktrees of the device's Git repositories through its
    /// helper: every repository when `all` (after a worktree was created or
    /// removed there, or a new connection), otherwise the ones not read yet.
    /// One read at a time; a request during it runs one more after it.
    pub(super) fn request_device_worktrees(&mut self, target: &str, all: bool) -> bool {
        let Some(session) = self
            .snapshot
            .status
            .remote
            .iter()
            .find(|status| status.target_id == target)
            .and_then(|status| status.session.as_ref())
        else {
            return false;
        };
        let roots = device_catalog::git_roots(session);
        let entry = self.device_worktrees.entry(target.to_owned()).or_default();
        if entry.projects.len() > MAX_KNOWN_PATHS {
            let known = entry.projects.len();
            entry.projects.retain(|root, _| roots.contains(root));
            if entry.projects.len() < known {
                crate::diagnostic!(serde_json::json!({
                    "component": "device_catalog",
                    "kind": "catalog.worktrees_pruned",
                    "target": target,
                    "known": known,
                    "kept": entry.projects.len(),
                }));
            }
        }
        let all = all || entry.reread;
        if entry.in_flight.is_some() {
            entry.again |= all || roots.iter().any(|root| !entry.projects.contains_key(root));
            return false;
        }
        let wanted = roots
            .into_iter()
            .filter(|root| all || !entry.projects.contains_key(root))
            .collect::<Vec<_>>();
        if wanted.is_empty() {
            return false;
        }
        let bases: BTreeMap<_, _> = wanted
            .iter()
            .map(|root| {
                (
                    root.clone(),
                    crate::project_settings::base_choice(
                        &self.snapshot.ui_state.project_settings,
                        &self.snapshot.ui_state.project_base_branches,
                        target,
                        root,
                    )
                    .map(str::to_owned),
                )
            })
            .collect();
        let Ok(channel) = self.node_link(target) else {
            return false;
        };
        // Branch notes link a node's checkouts to their issues (D-29); a
        // device this core dials keeps its issues there, so none is asked.
        let notes = self
            .link_origin(target)
            .is_some_and(LinkOrigin::reports_own_facts);
        let generation = self.device_host_generation(target);
        let entry = self.device_worktrees.entry(target.to_owned()).or_default();
        entry.in_flight = Some(generation);
        entry.again = false;
        entry.reread = false;
        let Some(context) = self.worker_context.clone() else {
            let answer = ask_worktrees(channel.as_ref(), target, &wanted, &bases, notes);
            return self.ingest_device_worktrees(target, generation, answer);
        };
        let device = target.to_owned();
        let spawned = thread::Builder::new()
            .name("herdr-core-device-worktrees".to_owned())
            .spawn(move || {
                let answer = ask_worktrees(channel.as_ref(), &device, &wanted, &bases, notes);
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                let changed = match runtime.lock() {
                    Ok(mut guard) => guard.ingest_device_worktrees(&device, generation, answer),
                    Err(_) => return,
                };
                drop(runtime);
                if changed {
                    context.notifier.notify();
                }
            });
        if let Err(error) = spawned {
            let entry = self.device_worktrees.entry(target.to_owned()).or_default();
            entry.in_flight = None;
            entry.reread |= all;
            entry.unavailable = Some(format!("The worktree reader could not start: {error}"));
        }
        false
    }

    /// The helper answered for the device's repositories. An answer from a
    /// connection that has since been replaced is dropped.
    pub(super) fn ingest_device_worktrees(
        &mut self,
        target: &str,
        generation: u64,
        answer: WorktreesAnswer,
    ) -> bool {
        let WorktreesAnswer {
            answers,
            notes,
            failure,
        } = answer;
        let Some(entry) = self.device_worktrees.get_mut(target) else {
            return false;
        };
        if entry.in_flight != Some(generation) {
            return false;
        }
        entry.in_flight = None;
        if std::mem::take(&mut entry.discard_answer) {
            entry.again = false;
            return self.request_device_worktrees(target, true);
        }
        for (root, project) in answers {
            match project {
                Some(project) => {
                    entry.projects.insert(root, project);
                }
                None => {
                    entry.notes.remove(&root);
                    entry.projects.remove(&root);
                }
            }
        }
        for (root, read) in notes {
            match read {
                Some(read) => {
                    entry.notes.insert(root, read);
                }
                None => {
                    entry.notes.remove(&root);
                }
            }
        }
        entry
            .notes
            .retain(|root, _| entry.projects.contains_key(root));
        if let Some(reason) = failure.as_deref() {
            crate::diagnostic!(serde_json::json!({
                "component": "device_worktrees",
                "kind": "worktrees.read_failed",
                "target": target,
                "generation": generation,
                "message": reason,
            }));
        }
        entry.unavailable = failure;
        let again = std::mem::take(&mut entry.again);
        self.advance_disk_sweep(Some(target));
        let mut changed = self.refresh_device_catalog(target);
        if again {
            changed |= self.request_device_worktrees(target, true);
        }
        changed
    }

    /// A new helper connection answers afresh: a branch or a checkout may
    /// have moved while none was connected. What the last connection
    /// answered stays until the new answer replaces it, so the device's
    /// panes keep their checkout across a reconnect: a pane's `hide` command
    /// is bound to its checkout when it starts and refused if that changed
    /// before it ran, and a regrouping that would undo itself a moment later
    /// is no change. An answer the old connection still owes is dropped.
    pub(super) fn reread_device_facts(&mut self, target: &str) -> bool {
        if let Some(entry) = self.device_facts.get_mut(target) {
            entry.in_flight = None;
            entry.unavailable = None;
            entry.reread = true;
        }
        if let Some(entry) = self.device_worktrees.get_mut(target) {
            entry.in_flight = None;
            entry.unavailable = None;
            entry.again = false;
            entry.reread = true;
        }
        // Each read runs once the device's session is here to name what it
        // reads, whichever of the two arrives first.
        let changed = self.request_device_facts(target);
        changed | self.request_device_worktrees(target, true)
    }

    /// Forgets what the device's Herdr reported and the tabs Hide created
    /// there. What its helper answered about its directories stays: a Retry
    /// connects to the same device again, and that connection asks again
    /// (`reread_device_facts`), so meanwhile the device's panes keep their
    /// checkouts. Removing the device, or a helper answering as another
    /// machine, forgets those answers too (`forget_device_directories`).
    pub(super) fn forget_device_catalog(&mut self, target: &str) {
        self.device_raw_sessions.remove(target);
        self.unconfirmed_node_issue_tokens.remove(target);
        self.device_recent_tabs.remove(target);
        self.created_device_tabs
            .retain(|(owner, _), _| owner != target);
    }

    /// The device was removed, or another machine answers its address:
    /// nothing its helper answered is kept.
    pub(super) fn forget_device_directories(&mut self, target: &str) {
        self.device_facts.remove(target);
        self.device_worktrees.remove(target);
    }
}

use hide_node_link::worktrees::WORKTREES_TIMEOUT;

/// What one read of a device's repositories answered.
pub(super) struct WorktreesAnswer {
    pub answers: Vec<(String, Option<crate::model::ProjectWorktreesSnapshot>)>,
    /// Each repository's branch notes, `None` where its config could not be
    /// read; empty when the node could not be asked.
    pub notes: Vec<(String, Option<BranchNotes>)>,
    pub failure: Option<String>,
}

type BranchNotes = std::collections::BTreeMap<String, hide_node_link::catalog::BranchNote>;

/// Reads each repository of device `target` in turn, then, when `notes`,
/// every one's branch notes in one `Call::PathFacts`; a refusal is that
/// repository's answer (no rows), a connection failure stops the batch with
/// the reason and asks no notes over the lost link. Notes that cannot be
/// read leave the issue a branch names unread, which is logged, not failed.
fn ask_worktrees(
    channel: &dyn crate::node_access::NodeLink,
    target: &str,
    roots: &[String],
    bases: &BTreeMap<String, Option<String>>,
    notes: bool,
) -> WorktreesAnswer {
    let (answers, failure) = ask_repositories(channel, roots, bases);
    if !notes || failure.is_some() {
        return WorktreesAnswer {
            answers,
            notes: Vec::new(),
            failure,
        };
    }
    let notes = match crate::workspace::PathIndex::ask(
        channel,
        answers.iter().map(|(root, _)| root.clone()).collect(),
    ) {
        Ok(index) => answers
            .iter()
            .map(|(root, _)| (root.clone(), index.branch_notes(root).cloned()))
            .collect(),
        Err(error) => {
            crate::diagnostic!(serde_json::json!({
                "component": "device_worktrees",
                "kind": "branch_notes.read_failed",
                "target": target,
                "message": error,
            }));
            Vec::new()
        }
    };
    WorktreesAnswer {
        answers,
        notes,
        failure,
    }
}

fn ask_repositories(
    channel: &dyn crate::node_access::NodeLink,
    roots: &[String],
    bases: &BTreeMap<String, Option<String>>,
) -> (
    Vec<(String, Option<crate::model::ProjectWorktreesSnapshot>)>,
    Option<String>,
) {
    let mut answers = Vec::new();
    for root in roots {
        match call_as::<Option<hide_node_link::worktrees::RepositoryWorktrees>>(
            channel,
            hide_node_link::protocol::Call::Worktrees {
                path: root.clone(),
                bases: Default::default(),
                base_override: bases.get(root).cloned().flatten(),
                reuse_unchanged: false,
            },
            WORKTREES_TIMEOUT,
        ) {
            Ok(facts) => {
                answers.push((root.clone(), facts.map(crate::worktrees::project_snapshot)))
            }
            Err(LinkError::Refused(error)) => {
                answers.push((
                    root.clone(),
                    Some(crate::model::ProjectWorktreesSnapshot {
                        root_path: root.clone(),
                        unavailable_reason: Some(error.message),
                        ..Default::default()
                    }),
                ));
            }
            Err(error) => return (answers, Some(error.to_string())),
        }
    }
    (answers, None)
}

/// Asks for each directory in turn. A refusal is that directory's answer; a
/// connection failure stops the batch, and the directories not reached stay
/// unanswered with the reason.
fn ask_facts(
    channel: &dyn crate::node_access::NodeLink,
    paths: &[String],
) -> (FactAnswers, Option<String>) {
    let mut answers = Vec::new();
    for path in paths {
        match call_as::<ProjectFacts>(
            channel,
            hide_node_link::protocol::Call::Project { path: path.clone() },
            FACTS_TIMEOUT,
        ) {
            Ok(facts) => answers.push((path.clone(), Fact::Known(facts))),
            Err(LinkError::Refused(error)) => {
                answers.push((path.clone(), Fact::Refused(error.message)));
            }
            Err(error) => return (answers, Some(error.to_string())),
        }
    }
    (answers, None)
}

const REGISTRABLE_TIMEOUT: Duration = Duration::from_secs(15);

impl Runtime {
    /// `create_workspace` on a device (B23, B24): the device's helper judges
    /// the folder against that device's home and names the project it
    /// belongs to; the registration is that project, and it is listed with
    /// or without a Herdr workspace there. Nothing is created on the device.
    pub(super) fn create_device_registration(
        &mut self,
        device: &str,
        path: String,
        label: String,
    ) -> bool {
        let channel = match self.node_link(device) {
            Ok(channel) => channel,
            Err(message) => {
                self.set_error(
                    "workspace.create_unavailable",
                    format!("Adding a project on this device needs its connection: {message}"),
                    true,
                );
                return true;
            }
        };
        let ask = move || {
            call_as::<hide_node_link::register::Registrable>(
                channel.as_ref(),
                hide_node_link::protocol::Call::Registrable { path: path.clone() },
                REGISTRABLE_TIMEOUT,
            )
            .map_err(|error| match error {
                LinkError::Refused(error) => error.message,
                other => other.to_string(),
            })
        };
        let Some(context) = self.worker_context.clone() else {
            let answer = ask();
            return self.ingest_device_registration(device, label, answer);
        };
        let device = device.to_owned();
        let spawned = thread::Builder::new()
            .name("herdr-core-device-registration".to_owned())
            .spawn(move || {
                let answer = ask();
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                let changed = match runtime.lock() {
                    Ok(mut guard) => guard.ingest_device_registration(&device, label, answer),
                    Err(_) => return,
                };
                drop(runtime);
                if changed {
                    context.notifier.notify();
                }
            });
        if let Err(error) = spawned {
            self.set_error(
                "workspace.create_worker_failed",
                format!("The registration worker could not be started: {error}"),
                true,
            );
        }
        true
    }

    pub(super) fn ingest_device_registration(
        &mut self,
        device: &str,
        label: String,
        answer: Result<hide_node_link::register::Registrable, String>,
    ) -> bool {
        // A device removed while its helper judged the folder takes no
        // registration from that late answer.
        // Removing a device forgets its host (`forget_device_host`).
        if !self.device_hosts.contains_key(device) {
            crate::diagnostic!(serde_json::json!({
                "component": "registration", "kind": "create.device_gone",
                "target": device,
            }));
            return false;
        }
        let registrable = match answer {
            Ok(registrable) => registrable,
            Err(message) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "registration", "kind": "create.refused",
                    "target": device, "message": message,
                }));
                self.set_error("workspace.create_refused", message, false);
                return true;
            }
        };
        let id = device_catalog::project_id(device, Path::new(&registrable.root));
        if self
            .snapshot
            .ui_state
            .workspace_registrations
            .iter()
            .any(|registration| registration.id == id)
        {
            // Registering the same project again reaches the same state.
            return false;
        }
        let label = Some(label.trim())
            .filter(|label| !label.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                Path::new(&registrable.root)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| registrable.root.clone());
        self.snapshot
            .ui_state
            .workspace_registrations
            .push(crate::model::WorkspaceRegistration {
                primary_checkout_id: None,
                id: id.clone(),
                label,
                path: registrable.root,
                device_id: device.to_owned(),
                pinned: false,
                home: false,
            });
        self.persist_ui_state();
        crate::diagnostic!(serde_json::json!({
            "component": "registration", "kind": "workspace.registered",
            "target": device, "workspace_id": id,
        }));
        self.request_device_facts(device);
        self.refresh_device_catalog(device);
        true
    }
}
