use super::*;
use std::time::Duration;

/// The most tabs Hide can have created and not yet seen leave Herdr. A tab
/// past it is placed by its pane cwd and the crossing is reported.
const CREATED_TAB_CHECKOUT_LIMIT: usize = 64;
/// How long a created tab's pane may keep reporting a cwd outside its
/// checkout before the report is believed: the birth value lasted about 20
/// seconds on a loaded runner (#400), so a longer silence is the shell's own
/// choice, or a path that never compares equal, and it is reported once.
const CREATED_TAB_BIRTH_WINDOW_MS: u64 = 30_000;

/// Where "now" comes from for the birth window. Monotonic, because a wall
/// clock that steps forward (a sleep, a synchronization) would end the window
/// before the shell had its time; a test hands the runtime its own.
pub(crate) type BirthClock = Arc<dyn Fn() -> Instant + Send + Sync>;

/// The checkout a tab Hide created was created for. The creation intent named
/// it, so it outranks the pane cwd, which Herdr reports from birth and never
/// announces settling.
pub(super) struct CreatedTabCheckout {
    path: String,
    /// The session has carried the tab at least once, so its absence from a
    /// later session means it closed.
    seen: bool,
    /// A pane of the tab has reported a cwd inside the checkout, so the cwd
    /// is no longer a birth value and is read as it is.
    settled: bool,
    recorded_at: Instant,
}

impl CreatedTabCheckout {
    /// The window for believing a birth cwd has ended without a pane ever
    /// reporting the checkout. Reported once, by the settle that follows.
    fn birth_window_over(&self, now: Instant, tab_id: &str) -> bool {
        let over = now.saturating_duration_since(self.recorded_at)
            > Duration::from_millis(CREATED_TAB_BIRTH_WINDOW_MS);
        if over {
            crate::diagnostic!(serde_json::json!({
                "component": "session",
                "kind": "created_tab.cwd_unsettled",
                "tab_id": tab_id,
                "checkout_path": self.path,
                "window_ms": CREATED_TAB_BIRTH_WINDOW_MS,
            }));
        }
        over
    }
}

/// The checkout a `reveal_path` names (`Runtime::reveal_checkout`).
struct RevealCheckout {
    path: String,
    device: String,
    /// The core brings it forward; a device session's checkout it does not.
    comes_forward: bool,
}

/// How a path is read before it is compared with a checkout's: this machine's
/// own paths resolve links and trailing separators on its file system, a
/// device's are its own strings, which this machine's files cannot speak for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PathRules {
    Local,
    Device,
}

impl PathRules {
    /// `path` in the form it is compared in: this machine's as its node
    /// spelled it in `paths`, else by names.
    pub(super) fn read(self, path: &str, paths: &workspace::PathIndex) -> String {
        match self {
            Self::Local => paths.comparison(path),
            Self::Device => path.trim_end_matches('/').to_owned(),
        }
    }
}

/// Whether `cwd` is `root` or below it, by names alone and whole names: a
/// sibling that only starts with the checkout's name is outside it. Both are
/// already read by the same `PathRules`.
pub(super) fn within_by_names(cwd: &str, root: &str) -> bool {
    hide_platform::path::wire_relative(root, cwd).is_ok()
}

/// Where a created tab's panes are read as being until one of them reports a
/// cwd in it: the checkout the tab was created for, less the worktrees of its
/// repository nested below it.
pub(crate) struct CreatedTabClamp {
    pub(crate) tab_id: String,
    /// The checkout as it was asked for, which a clamped pane reads as its cwd.
    pub(crate) path: String,
    rules: PathRules,
    /// Each repository's worktree paths as the catalog lists them. A folder
    /// in another worktree below the checkout is that worktree's, so it is no
    /// more the outer checkout's shell start folder than a sibling is.
    repositories: Arc<Vec<Vec<String>>>,
}

impl CreatedTabClamp {
    /// Whether `cwd` is the checkout's own. The checkout, `cwd` and the
    /// nested worktrees are all read through the same `paths`, the newest
    /// the node answered, and compared by names, so no comparison reads a
    /// folder and a worktree listed before the node knew it is not misread.
    pub(super) fn holds(&self, cwd: &str, paths: &workspace::PathIndex) -> bool {
        let read = |path: &str| self.rules.read(path, paths);
        let root = read(&self.path);
        let cwd = read(cwd);
        within_by_names(&cwd, &root)
            && !nested_worktrees(&self.repositories, &root, read)
                .iter()
                .any(|nested| within_by_names(&cwd, nested))
    }
}

/// The worktrees of the repository that lists `root`, strictly below it, as
/// `read` gives them; a checkout no repository lists has none, so a worktree
/// the catalog has not listed yet is not told apart from the checkout around
/// it.
fn nested_worktrees(
    repositories: &[Vec<String>],
    root: &str,
    read: impl Fn(&str) -> String,
) -> Vec<String> {
    repositories
        .iter()
        .find(|worktrees| worktrees.iter().any(|path| read(path) == root))
        .into_iter()
        .flatten()
        .map(|path| read(path))
        .filter(|path| path.as_str() != root && within_by_names(path, root))
        .collect()
}

enum SessionFocusCheck {
    Unchecked,
    Readback(Option<(PendingPaneFocusControl, bool)>),
}

fn suppress_unconfirmed_created_purposes(
    pending: &HashMap<String, String>,
    workspaces: &mut [crate::model::WorkspaceSnapshot],
    agents: &[crate::model::SidebarAgentSnapshot],
) -> bool {
    use crate::model::CheckoutPurposeOrigin;

    let mut changed = false;
    for checkout in workspaces
        .iter_mut()
        .flat_map(|workspace| workspace.checkouts.iter_mut())
    {
        let path = workspace::comparison_by_names(Path::new(&checkout.path));
        let Some(unconfirmed) = pending.get(&path) else {
            continue;
        };
        if checkout.purpose.as_ref().is_some_and(|purpose| {
            purpose.origin == CheckoutPurposeOrigin::Token && purpose.text == *unconfirmed
        }) {
            checkout.purpose = None;
            changed = true;
        }
    }
    if changed {
        changed |= crate::sidebar::sync_checkout_purposes(workspaces, agents);
    }
    changed
}

impl Runtime {
    pub(crate) fn unconfirmed_created_purpose_values(&self) -> HashMap<String, String> {
        let mut suppressions = self.unconfirmed_created_purposes.clone();
        suppressions.extend(self.created_purpose_writes_in_flight.clone());
        suppressions
    }

    /// Registers the exact value a creation worker is about to write before
    /// the Herdr token request can publish an event to the purpose mirror.
    pub(crate) fn begin_created_purpose_write(&mut self, path: &str, purpose: &str) {
        let path = workspace::comparison_by_names(Path::new(path));
        self.created_purpose_writes_in_flight
            .insert(path, purpose.to_owned());
    }

    /// Groups the session's working directories under the Herdr workspace that
    /// owns them. Shared with the session-sync coordinator so a catalog
    /// precomputed outside the runtime lock is built from the same inputs.
    ///
    /// Layouts carry the workspace a pane belongs to and `panes` carries its
    /// directory, so the two together give each workspace the set of
    /// repositories it actually occupies without asking git anything.
    /// The Herdr workspaces and the directories their panes occupy, as the
    /// project catalog sees them.
    ///
    /// Every path a catalog of `payload` and the created tabs' clamps read.
    fn wanted_paths(&self, payload: &SessionSnapshotPayload) -> BTreeSet<String> {
        let mut wanted = workspace::PathIndex::wanted(
            &self.node,
            &self.snapshot.ui_state.workspace_registrations,
            &[],
            &self.worktree_catalog,
        );
        wanted.extend(Self::session_cwds(payload));
        wanted.extend(
            self.created_tab_checkouts
                .values()
                .map(|created| created.path.clone()),
        );
        wanted
    }

    /// What the core's own node says of `wanted`, asked from inside the
    /// runtime. Only a runtime the sync coordinator does not feed asks this
    /// way: a fixture, a test, and the registrations-only catalog before
    /// Herdr first answers. A failed ask is logged and the previous answer
    /// stands, read by names where it is silent.
    pub(super) fn ask_paths(&self, wanted: BTreeSet<String>) -> Arc<workspace::PathIndex> {
        match workspace::PathIndex::ask(self.own_node.as_ref(), wanted) {
            Ok(paths) => Arc::new(paths),
            Err(error) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "workspace_catalog",
                    "kind": "catalog.path_facts_failed",
                    "message": error,
                }));
                Arc::clone(&self.catalog_paths)
            }
        }
    }

    /// The path facts the accepted catalog was built from.
    pub(crate) fn catalog_paths(&self) -> Arc<workspace::PathIndex> {
        Arc::clone(&self.catalog_paths)
    }

    /// Establish this before the coordinator's first asynchronous read, so
    /// even an empty initial answer cannot fall back to node I/O under lock.
    pub(crate) fn begin_catalog_coordinator(&mut self) {
        self.catalog_from_coordinator = true;
    }

    /// The paths a session applied without a precomputed catalog reads: the
    /// last the coordinator brought once it feeds this runtime, so nothing is
    /// asked under the lock, else what the core's own node answers now.
    fn paths_without_catalog(&self, payload: &SessionSnapshotPayload) -> Arc<workspace::PathIndex> {
        if self.catalog_from_coordinator {
            Arc::clone(&self.catalog_paths)
        } else {
            self.ask_paths(self.wanted_paths(payload))
        }
    }

    /// Every directory the session's panes occupy, as Herdr reported it: what
    /// the core's own node is asked about before a catalog is built.
    pub(crate) fn session_cwds(payload: &SessionSnapshotPayload) -> BTreeSet<String> {
        payload
            .layouts
            .iter()
            .flat_map(|layout| &layout.panes)
            .filter_map(|pane| Self::pane_cwd(payload, &pane.pane_id))
            .collect()
    }

    /// The session's Herdr workspaces with the directories their panes
    /// occupy, each in comparison form as `paths` gives it.
    pub fn session_spaces(
        payload: &SessionSnapshotPayload,
        paths: &workspace::PathIndex,
    ) -> Vec<workspace::SessionSpace> {
        let labels: HashMap<&str, &str> = payload
            .workspaces
            .iter()
            .map(|workspace| (workspace.workspace_id.as_str(), workspace.label.trim()))
            .collect();
        let mut spaces: Vec<workspace::SessionSpace> = Vec::new();
        for layout in &payload.layouts {
            let index = match spaces
                .iter()
                .position(|space| space.id == layout.workspace_id)
            {
                Some(index) => index,
                None => {
                    let label = labels
                        .get(layout.workspace_id.as_str())
                        .copied()
                        .filter(|label| !label.is_empty())
                        .unwrap_or(layout.workspace_id.as_str())
                        .to_owned();
                    spaces.push(workspace::SessionSpace {
                        id: layout.workspace_id.clone(),
                        label,
                        cwds: Vec::new(),
                        purpose: payload
                            .workspaces
                            .iter()
                            .find(|workspace| workspace.workspace_id == layout.workspace_id)
                            .and_then(|workspace| workspace.tokens.get("purpose"))
                            .and_then(serde_json::Value::as_str)
                            .map(str::trim)
                            .filter(|purpose| !purpose.is_empty())
                            .map(str::to_owned),
                    });
                    spaces.len() - 1
                }
            };
            for pane in &layout.panes {
                let Some(cwd) = Self::pane_cwd(payload, &pane.pane_id) else {
                    continue;
                };
                let cwd = paths.comparison(&cwd);
                if !spaces[index].cwds.contains(&cwd) {
                    spaces[index].cwds.push(cwd);
                }
            }
        }
        spaces
    }
    pub(crate) fn pane_cwd(payload: &SessionSnapshotPayload, pane_id: &str) -> Option<String> {
        payload
            .panes
            .iter()
            .find(|pane| pane.pane_id == pane_id)
            .and_then(|pane| pane.cwd.clone())
            .or_else(|| {
                payload
                    .agents
                    .iter()
                    .find(|agent| agent.pane_id.as_deref().or(agent.id.as_deref()) == Some(pane_id))
                    .and_then(|agent| agent.cwd.clone())
            })
            .map(|cwd| cwd.trim().to_owned())
            // Herdr reports `/` for a pane whose process has exited, which
            // says where the pane is not rather than where it is. Treating it
            // as a directory produced an unnamed checkout row with no panes
            // under it.
            .filter(|cwd| !cwd.is_empty() && cwd != "/")
    }
    /// Rebuilds the navigator from the Herdr workspaces the session reports
    /// and any registration Herdr has no workspace for. Registrations are
    /// durable metadata only: removing one never removes a checkout or ends a
    /// remote process.
    pub(super) fn reconcile_session_catalog(
        &mut self,
        payload: &SessionSnapshotPayload,
        precomputed: Option<session_sync::PrecomputedCatalog>,
        inline_paths: Option<Arc<workspace::PathIndex>>,
    ) -> bool {
        let accepted_tab_paths = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .flat_map(|workspace| &workspace.checkouts)
            .flat_map(|checkout| {
                checkout
                    .tabs
                    .iter()
                    .filter_map(|tab| Some((tab.id.clone()?, checkout.path.clone())))
            })
            .collect::<HashMap<_, _>>();
        // Catalog reconstruction precedes canonical machine-qualified lineage.
        // Preserve known ownership until that pass, so admission cannot evict
        // the keyboard from an unchanged delegated canvas at the normal cap.
        let delegated_tabs = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .flat_map(|workspace| &workspace.checkouts)
            .flat_map(|checkout| &checkout.tabs)
            .filter(|tab| tab.delegated)
            .filter_map(|tab| tab.id.clone())
            .collect::<HashSet<_>>();
        self.issue_tokens = crate::wire::issue_tokens(payload);
        // The catalog reads what the core's own node says of its paths, so
        // the sync coordinator asks and builds it before taking the runtime
        // lock. A precomputation whose registrations or worktrees no longer match current
        // state is stale; the last accepted catalog stands in for it and the
        // next publish, a second away, brings a fresh one. Nothing is asked
        // here once the coordinator feeds this runtime: one `git rev-parse`
        // per tab under this lock stalled the main thread and every attach
        // reader for a third of their time (2026-09-06, 18 agents, load 7 to
        // 11). A session applied without a catalog is built from the last
        // paths the coordinator brought; only a runtime it does not feed, the
        // fixture and test path, asks its node inline.
        let (mut workspaces, paths) = match precomputed {
            Some(catalog)
                if catalog.registrations == self.snapshot.ui_state.workspace_registrations
                    && catalog.worktrees == self.worktree_catalog =>
            {
                self.last_session_spaces = Self::session_spaces(payload, &catalog.paths);
                self.last_accepted_catalog = Some(catalog.workspaces.clone());
                self.catalog_paths = Arc::clone(&catalog.paths);
                self.catalog_from_coordinator = true;
                (catalog.workspaces, catalog.paths)
            }
            Some(_) if self.last_accepted_catalog.is_some() => {
                self.push_diagnostic(
                    "catalog.precomputed_stale",
                    "The precomputed workspace catalog no longer matches its inputs; the last accepted catalog stands until the next publish".to_owned(),
                );
                self.last_session_spaces = Self::session_spaces(payload, &self.catalog_paths);
                (
                    self.last_accepted_catalog
                        .clone()
                        .expect("checked by the match guard"),
                    Arc::clone(&self.catalog_paths),
                )
            }
            _ => {
                let paths = inline_paths.unwrap_or_else(|| self.paths_without_catalog(payload));
                self.last_session_spaces = Self::session_spaces(payload, &paths);
                let workspaces = workspace::build_catalog(
                    &self.node,
                    &self.snapshot.ui_state.workspace_registrations,
                    &self.last_session_spaces,
                    &self.worktree_catalog,
                    &paths,
                );
                self.last_accepted_catalog = Some(workspaces.clone());
                self.catalog_paths = Arc::clone(&paths);
                (workspaces, paths)
            }
        };
        let mut unresolved_roots: Vec<String> = Vec::new();
        Self::apply_workspace_expansion(
            &mut workspaces,
            &self.snapshot.ui_state.collapsed_workspace_ids,
        );
        let projected_agents = project_agents(payload.clone()).agents;
        let listening_ports = self.listening_ports.entries.clone();

        // Every workspace's whole tab list, in Herdr's order, before any of it
        // is split across checkouts. A tab whose layout has not arrived yet is
        // in it, because Herdr counts it when it indexes a move. Rebuilt whole
        // each reconcile so a closed workspace leaves no stale order behind.
        self.herdr_worktrees = payload
            .workspaces
            .iter()
            .filter_map(|workspace| {
                Some((workspace.workspace_id.clone(), workspace.worktree.clone()?))
            })
            .collect();
        self.herdr_workspace_tab_order = payload.tabs.iter().fold(
            BTreeMap::<String, Vec<String>>::new(),
            |mut order, session_tab| {
                order
                    .entry(session_tab.workspace_id.clone())
                    .or_default()
                    .push(session_tab.tab_id.clone());
                order
            },
        );

        // A created tab's record lives as long as its tab: it is kept until
        // the session has carried the tab once and then drops it. A blocked
        // workspace republishes its last published tabs, so a tab the session
        // carried does not vanish from here while its workspace waits.
        self.created_tab_checkouts.retain(|tab_id, created| {
            let present = payload.tabs.iter().any(|tab| &tab.tab_id == tab_id);
            created.seen |= present;
            present || !created.seen
        });

        // Herdr's tab order is the navigator's tab order. A layout is the
        // per-tab detail looked up by tab id, never what decides where a tab
        // sits: layouts arrive in the order each tab was first drawn, so a tab
        // Herdr moved kept its original place forever and a tab that redrew
        // never moved back.
        let mut placed_tabs: BTreeMap<&str, usize> = BTreeMap::new();
        // Herdr's own labels, kept per checkout while they are still raw. The
        // snapshot's tabs carry the formatted form, so the free number has to
        // be taken here or read back out of display text later.
        let mut raw_tab_labels: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut missing_created: Vec<String> = Vec::new();
        for session_tab in &payload.tabs {
            let Some(layout) = payload
                .layouts
                .iter()
                .find(|layout| layout.tab_id == session_tab.tab_id)
            else {
                continue;
            };
            // A tab Hide created belongs to the checkout it was created for.
            // Its pane cwd is a birth value until the shell has entered the
            // start folder, and Herdr sends no event when it settles, so
            // reading it placed a new tab under another checkout until the
            // next unrelated publish (#400). A checkout no longer in the
            // catalog is reported once, and the tab falls back to the pane
            // rule below from then on.
            let created = self
                .created_tab_checkouts
                .get(&session_tab.tab_id)
                .and_then(|created| {
                    let found = workspaces
                        .iter()
                        .enumerate()
                        .find_map(|(index, workspace)| {
                            workspace
                                .checkouts
                                .iter()
                                .position(|checkout| checkout.path == created.path)
                                .map(|checkout| (index, checkout))
                        });
                    if found.is_none() {
                        missing_created.push(session_tab.tab_id.clone());
                        crate::diagnostic!(serde_json::json!({
                            "component": "session",
                            "kind": "created_tab.checkout_missing",
                            "tab_id": session_tab.tab_id,
                            "checkout_path": created.path,
                        }));
                    }
                    found
                });
            let (workspace_snapshot, checkout_index) = match created {
                Some((index, checkout)) => (&mut workspaces[index], Some(checkout)),
                None => {
                    // Any other tab has no recorded intent, so its pane cwd
                    // is all there is. A plain terminal pane is not necessarily
                    // represented in the agent list. Its cwd is still
                    // authoritative for attaching the live layout to the
                    // registered checkout. Falling back to the agent record
                    // keeps agent-specific cwd handling intact.
                    let mut context_path = layout.panes.iter().find_map(|pane| {
                        payload
                            .panes
                            .iter()
                            .find(|source| source.pane_id == pane.pane_id)
                            .and_then(|source| source.cwd.clone())
                            .filter(|path| !path.trim().is_empty())
                            .or_else(|| {
                                payload
                                    .agents
                                    .iter()
                                    .find(|agent| {
                                        agent.pane_id.as_deref().or(agent.id.as_deref())
                                            == Some(pane.pane_id.as_str())
                                    })
                                    .and_then(|agent| agent.cwd.clone())
                            })
                    });
                    if self.catalog_from_coordinator
                        && context_path
                            .as_deref()
                            .is_some_and(|path| !paths.knows(path))
                    {
                        // A changed directory is not a new checkout until the
                        // node answers. Keep this tab's confirmed placement;
                        // a new tab with no confirmed place waits for facts.
                        context_path = accepted_tab_paths.get(&session_tab.tab_id).cloned();
                        if context_path.is_none() {
                            continue;
                        }
                    }
                    let Some(workspace_snapshot) = find_workspace_for_context(
                        &self.node,
                        &mut workspaces,
                        context_path.as_deref(),
                        &layout.workspace_id,
                        &paths,
                        &mut unresolved_roots,
                    ) else {
                        continue;
                    };
                    let checkout_index = context_path.as_deref().and_then(|path| {
                        workspace_snapshot.checkouts.iter().position(|checkout| {
                            path_is_within_checkout(&paths, path, &checkout.path)
                        })
                    });
                    (workspace_snapshot, checkout_index)
                }
            };
            let Some(checkout) = workspace_snapshot
                .checkouts
                .get_mut(checkout_index.unwrap_or(0))
            else {
                continue;
            };
            let panes = project_layout_panes(
                layout,
                payload,
                &projected_agents,
                &listening_ports,
                &checkout.path,
            );
            let representative = projected_agents
                .iter()
                .find(|agent| agent.pane_id == layout.focused_pane_id);
            let process = payload
                .panes
                .iter()
                .find(|pane| pane.pane_id == layout.focused_pane_id)
                .and_then(|pane| pane.foreground_process.as_deref());
            let mut tab = TabSnapshot {
                agent: representative.map(Into::into),
                naming: crate::model::TabNaming {
                    // Set with any pending rename, below.
                    by_operator: false,
                    focused_pane_id: layout.focused_pane_id.clone(),
                    number: crate::model::tab_number(&session_tab.label, session_tab.number),
                    processes: payload
                        .panes
                        .iter()
                        .filter(|pane| {
                            layout
                                .panes
                                .iter()
                                .any(|entry| entry.pane_id == pane.pane_id)
                        })
                        .filter_map(|pane| {
                            pane.foreground_process
                                .clone()
                                .map(|name| (pane.pane_id.clone(), name))
                        })
                        .collect(),
                    raw: session_tab.label.clone(),
                    automatic: crate::model::display_tab_label(
                        "",
                        session_tab.number,
                        representative.map(|agent| agent.identity_label.as_str()),
                        process,
                    ),
                },
                id: Some(session_tab.tab_id.clone()),
                workspace_id: Some(workspace_snapshot.id.clone()),
                checkout_id: Some(checkout.id.clone()),
                label: Some(crate::model::display_tab_label(
                    &session_tab.label,
                    session_tab.number,
                    representative.map(|agent| agent.identity_label.as_str()),
                    process,
                )),
                empty: panes.is_empty(),
                delegated: delegated_tabs.contains(&session_tab.tab_id),
                panes,
            };
            self.apply_tab_rename(&mut tab);
            if let Some(existing) = checkout
                .tabs
                .iter_mut()
                .find(|existing| existing.id == tab.id)
            {
                *existing = tab;
            } else {
                checkout.tabs.push(tab);
            }
            *placed_tabs.entry(layout.workspace_id.as_str()).or_default() += 1;
            raw_tab_labels
                .entry(checkout.id.clone())
                .or_default()
                .push(session_tab.label.clone());
        }
        for tab_id in missing_created {
            self.created_tab_checkouts.remove(&tab_id);
        }

        // A worktree earns its row from git, not from a pane, so which rows
        // have a terminal is only known once the tabs are attached.
        for workspace in &mut workspaces {
            for checkout in &mut workspace.checkouts {
                checkout.has_panes = checkout.tabs.iter().any(|tab| !tab.panes.is_empty());
            }
        }

        // Each checkout's owner is read off Herdr's own binding and Hide's
        // mark, never off where its tabs sit (PRD checkout-workspace-binding
        // D-06); this session already carries both, so it costs no call.
        let owner_facts = payload
            .workspaces
            .iter()
            .map(crate::checkout_owner::WorkspaceFacts::from)
            .collect::<Vec<_>>();
        for workspace in &mut workspaces {
            let is_git = workspace.is_git;
            for checkout in &mut workspace.checkouts {
                checkout.owner_workspace_id = crate::checkout_owner::node_owner_of(
                    self.node.as_str(),
                    &checkout.path,
                    is_git,
                    owner_facts.iter().copied(),
                )
                .map(str::to_owned);
            }
        }

        for workspace in &mut workspaces {
            for checkout in &mut workspace.checkouts {
                checkout.next_tab_label = crate::model::next_tab_label(
                    raw_tab_labels
                        .get(&checkout.id)
                        .map_or(&[][..], Vec::as_slice)
                        .iter()
                        .map(String::as_str),
                );
            }
        }

        // Herdr names one active tab per workspace. Hide owns which tab is
        // visible, so that name is what Hide reconciles against rather than
        // what it obeys: it confirms a switch Hide made, or it is an operator
        // focusing a tab outside Hide and Hide follows it and says so.
        // A checkout is keyed by path, so two Herdr workspaces at one path
        // land in one checkout with one active tab each. The view is kept
        // per Herdr workspace for that reason: folding it to one tab per
        // checkout let the last workspace in payload order overwrite the
        // others, and a tab focus on any other workspace was then never
        // confirmed and always followed back.
        if !unresolved_roots.is_empty() {
            unresolved_roots.sort();
            unresolved_roots.dedup();
            self.push_diagnostic(
                "catalog.root_unresolved",
                format!(
                    "{} pane director{} placed by path alone because the root index did not carry {}: {}",
                    unresolved_roots.len(),
                    if unresolved_roots.len() == 1 { "y was" } else { "ies were" },
                    if unresolved_roots.len() == 1 { "it" } else { "them" },
                    unresolved_roots.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
                ),
            );
        }
        // A pending creation already owns its draft's logical slot. Keep its
        // physical staging tab out of focus/admission and the published list.
        self.project_catalog_agent_groups(&mut workspaces);
        let mut unresolved_active_tabs = BTreeSet::new();
        let herdr_tabs = HerdrTabView::from_payload(payload);
        for session_workspace in &payload.workspaces {
            let Some(active_tab_id) = session_workspace
                .active_tab_id
                .as_deref()
                .map(str::trim)
                .filter(|active_tab_id| !active_tab_id.is_empty())
            else {
                continue;
            };
            let resolved = workspaces.iter().any(|workspace| {
                workspace.checkouts.iter().any(|checkout| {
                    checkout
                        .tabs
                        .iter()
                        .any(|tab| tab.id.as_deref() == Some(active_tab_id))
                })
            });
            // A workspace none of whose tabs reached the navigator is not a
            // contradiction, only a workspace outside every registered
            // checkout. An active tab missing from a workspace that did place
            // tabs is the state worth reporting.
            if !resolved
                && placed_tabs
                    .get(session_workspace.workspace_id.as_str())
                    .is_some_and(|placed| *placed > 0)
            {
                unresolved_active_tabs.insert(format!(
                    "{}/{active_tab_id}",
                    session_workspace.workspace_id
                ));
            }
        }
        self.report_unresolved_active_tabs(unresolved_active_tabs);
        self.reconcile_visible_tabs(&mut workspaces, &herdr_tabs, payload.tab_moves.as_ref());

        let previous = self.snapshot.navigator.clone();
        let previous_card = self.snapshot.card.clone();
        crate::agent_state::sync_checkout_agent_summaries(
            &mut workspaces,
            &self.snapshot.navigator.agents,
        );
        let created_purpose_suppressions = self.unconfirmed_created_purpose_values();
        suppress_unconfirmed_created_purposes(
            &created_purpose_suppressions,
            &mut workspaces,
            &self.snapshot.navigator.agents,
        );
        crate::project_context::sort_projects(&mut workspaces, &projected_agents);
        self.snapshot.navigator.workspaces = workspaces;
        self.snapshot.navigator.devices = workspace::devices(
            &self.node,
            self.machine_name.as_deref(),
            &self.snapshot.ui_state,
        );
        self.refresh_device_snapshots();
        // An agent belongs to the device whose project holds its pane. The
        // project label is no longer Herdr's workspace label once a
        // registration covers the repository, so labels cannot be the key.
        for device in &mut self.snapshot.navigator.devices {
            device.agent_count = crate::agent_state::device_catalog_count(
                &self.snapshot.navigator.workspaces,
                &projected_agents,
                &device.id,
            );
        }
        if self.snapshot.navigator.focused_device_id.is_none() {
            self.snapshot.navigator.focused_device_id = Some(self.node.as_str().to_owned());
        }
        self.resync_navigator_focus();
        self.rebuild_tab_strips();
        self.refresh_agent_scopes();
        previous != self.snapshot.navigator || previous_card != self.snapshot.card
    }
    /// Puts one strip entry at a new place in its checkout's strip.
    ///
    /// The two kinds of entry have different owners. A file tab's slot is
    /// Hide's, so a move that only rearranges file slots is committed here and
    /// nothing is sent to Herdr. The relative order of Herdr's tabs is Herdr's,
    /// so a move that changes it is a request: the arrangement is held until
    /// Herdr reports the order it actually has, and a refusal leaves the strip
    /// on the order Herdr last reported.
    pub(super) fn reorder_tab(&mut self, payload: ReorderTabPayload) -> bool {
        // Only the legacy single-surface host still arranges local strip
        // entries. The web host persists Agent order through agent_layout;
        // never let an obsolete local request move Herdr's tabs there.
        if self.separate_view_areas()
            && self.snapshot.navigator.workspaces.iter().any(|workspace| {
                workspace.id == payload.workspace_id && workspace.remote_target_id.is_none()
            })
        {
            self.set_error(
                "tab.area_order_required",
                "Use the Agent area to move this tab",
                false,
            );
            return true;
        }
        // The checkout is this machine's or a device's; a device's strip is
        // placed by the same rule, and its Herdr tabs move on its own Herdr.
        let local = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .filter(|workspace| workspace.remote_target_id.is_none())
            .find(|workspace| workspace.id == payload.workspace_id);
        let device = self.snapshot.status.remote.iter().find_map(|remote| {
            remote
                .session
                .as_ref()?
                .workspaces
                .iter()
                .find(|workspace| workspace.id == payload.workspace_id)
                .map(|workspace| (remote.target_id.clone(), workspace))
        });
        let (device, workspace) = match (local, device) {
            (Some(workspace), _) => (None, workspace),
            (None, Some((target, workspace))) => (Some(target), workspace),
            (None, None) => {
                self.set_error(
                    "tab.unknown_workspace",
                    format!("Workspace {} is not registered", payload.workspace_id),
                    false,
                );
                return true;
            }
        };
        let Some(checkout) = workspace
            .checkouts
            .iter()
            .find(|checkout| checkout.id == payload.checkout_id)
        else {
            self.set_error(
                "tab.unknown_checkout",
                format!("Checkout {} is not available", payload.checkout_id),
                false,
            );
            return true;
        };
        let strip = checkout.strip.clone();
        if self
            .pending_tab_move
            .get(&payload.checkout_id)
            .is_some_and(|pending| {
                matches!(
                    pending.phase.as_str(),
                    "transmitting" | "awaiting_topology" | "unknown"
                )
            })
        {
            self.set_error(
                "tab.move_in_progress",
                format!(
                    "Tab order for checkout {} is still being confirmed; the new move was not sent",
                    payload.checkout_id
                ),
                true,
            );
            self.sync_async_operations();
            return false;
        }
        let Some(from) = strip.iter().position(|entry| entry.id == payload.tab_id) else {
            self.set_error(
                "tab.reorder_unknown",
                format!("Tab {} is not in this checkout's strip", payload.tab_id),
                false,
            );
            return true;
        };
        if payload.to_index >= strip.len() {
            self.set_error(
                "tab.reorder_out_of_range",
                format!(
                    "Position {} is past the end of a strip of {}",
                    payload.to_index,
                    strip.len()
                ),
                false,
            );
            return true;
        }

        let mut desired = strip.clone();
        let moved = desired.remove(from);
        desired.insert(payload.to_index, moved.clone());
        let desired_ids = desired
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>();
        let herdr_ids = |entries: &[StripTabSnapshot]| {
            entries
                .iter()
                .filter(|entry| entry.kind == StripTabKind::Herdr)
                .map(|entry| entry.source_id.clone())
                .collect::<Vec<_>>()
        };
        let current_herdr = herdr_ids(&strip);
        let desired_herdr = herdr_ids(&desired);

        if current_herdr == desired_herdr {
            // Only slots Hide owns changed, so Herdr has nothing to do and the
            // arrangement is the operator's the moment they drop it.
            self.pending_tab_move.remove(&payload.checkout_id);
            self.checkout_tab_order
                .insert(payload.checkout_id.clone(), desired_ids);
            // A preview tab the operator placed by hand is one they mean to
            // keep (B7); the rebuild below draws the promoted slot.
            if moved.preview {
                self.promote_editor_tab(&moved.source_id);
            }
            self.rebuild_tab_strips();
            return true;
        }

        // Herdr indexes a move inside the workspace that owns the tab, so the
        // drag - not the checkout - decides whether Herdr hears about it. What
        // matters is the moved tab's own workspace subsequence: a drag that
        // only steps over tabs belonging to another Herdr workspace changes
        // nothing Herdr can see, and the strip settles locally.
        //
        // Deciding this per checkout is what refused every drag in a checkout
        // whose tabs come from two Herdr workspaces, which is the ordinary
        // arrangement for a repository opened twice.
        let workspace_orders = match device.as_deref() {
            None => self.herdr_workspace_tab_order.clone(),
            Some(target) => self
                .device_raw_sessions
                .get(target)
                .map(device_workspace_tab_order)
                .unwrap_or_default(),
        };
        let Some((moved_workspace_id, workspace_order)) = workspace_orders
            .iter()
            .find(|(_, order)| order.contains(&moved.source_id))
            .map(|(workspace_id, order)| (workspace_id.clone(), order.clone()))
        else {
            self.set_error(
                "tab.reorder_inconsistent",
                format!(
                    "Tab {} cannot be placed there: Herdr does not list it in any workspace",
                    moved.source_id
                ),
                false,
            );
            return true;
        };
        let owned = |entries: &[String]| {
            entries
                .iter()
                .filter(|tab_id| workspace_order.contains(tab_id))
                .cloned()
                .collect::<Vec<_>>()
        };
        let current_owned = owned(&current_herdr);
        let desired_owned = owned(&desired_herdr);
        if current_owned == desired_owned {
            // The moved tab kept its place among its own workspace's tabs, so
            // only slots Hide arranges changed and the drop is final now.
            self.pending_tab_move.remove(&payload.checkout_id);
            self.checkout_tab_order
                .insert(payload.checkout_id.clone(), desired_ids);
            self.rebuild_tab_strips();
            return true;
        }

        let Some(insert_index) =
            herdr_insert_index(&workspace_order, &desired_owned, &moved.source_id)
        else {
            self.set_error(
                "tab.reorder_inconsistent",
                format!(
                    "Tab {} cannot be placed there: it is not one of Herdr's tabs in this checkout",
                    moved.source_id
                ),
                false,
            );
            return true;
        };
        let carrier = match device.as_deref() {
            None => self
                .live
                .as_ref()
                .map(|_| (TabMoveCarrier::Local, self.live_generation))
                .ok_or_else(|| "Moving a Herdr tab requires a live Herdr connection".to_owned()),
            Some(target) => self.device_tab_move_carrier(target),
        };
        let (carrier, connection_generation) = match carrier {
            Ok(carrier) => carrier,
            Err(message) => {
                self.set_error("tab.control_unavailable", message, true);
                return true;
            }
        };
        let generation = self.next_tab_move_generation;
        self.next_tab_move_generation += 1;
        let now = unix_milliseconds();
        self.pending_tab_move.insert(
            payload.checkout_id.clone(),
            PendingTabMove {
                desired: desired_ids,
                workspace_id: moved_workspace_id,
                // The order asked for is the moved tab's own workspace's, the
                // same subsequence the request was indexed in. Holding the
                // checkout's mixed order here would wait for an interleaving
                // Herdr never reports once a checkout draws tabs from two
                // workspaces, and the drag would snap back and stay back.
                herdr_order: desired_owned.clone(),
                target_id: moved.source_id.clone(),
                generation,
                connection_generation,
                phase: "transmitting".to_owned(),
                stage: "request".to_owned(),
                started_at_unix_ms: now,
                // A move on this machine waits its turn on the control lane,
                // so its five seconds start when it leaves (`start_job`).
                deadline_at_unix_ms: (!matches!(carrier, TabMoveCarrier::Local))
                    .then(|| now.saturating_add(CLOSE_STAGE_TIMEOUT_MS)),
                message: Some("Waiting for Herdr to confirm the tab order".to_owned()),
                retryable: false,
            },
        );
        self.sync_async_operations();
        self.push_diagnostic(
            "tab.move.requested",
            format!(
                "Asking Herdr to insert tab {} at {insert_index} in its own workspace",
                moved.source_id
            ),
        );
        // Herdr answers with its own workspace's tabs, so the order to check
        // the answer against is the moved tab's workspace subsequence, never
        // the checkout's mixed order. A device's Herdr names its tabs without
        // the device scope the strip carries.
        let herdr_id = |tab_id: &str| match device.as_deref() {
            None => Some(tab_id.to_owned()),
            Some(target) => remote_tab_source_id(target, tab_id).map(str::to_owned),
        };
        let (Some(tab_id), Some(expected_order)) = (
            herdr_id(&moved.source_id),
            desired_owned
                .iter()
                .map(|tab_id| herdr_id(tab_id))
                .collect::<Option<Vec<_>>>(),
        ) else {
            self.pending_tab_move.remove(&payload.checkout_id);
            self.sync_async_operations();
            self.set_error(
                "tab.reorder_inconsistent",
                format!("Tab {} is not scoped to its device", moved.source_id),
                false,
            );
            return true;
        };
        let action = RemoteControlAction::MoveTab {
            checkout_id: payload.checkout_id.clone(),
            tab_id,
            insert_index,
            expected_order,
            generation,
            connection_generation,
        };
        let spawned = match carrier {
            TabMoveCarrier::Local => self.submit_local_control(action),
            TabMoveCarrier::Device(context) => live::spawn_remote_control(
                context,
                format!("tab.move:{}:{generation}", payload.checkout_id),
                action,
                connection_generation,
            ),
        };
        if let Err(message) = spawned {
            self.pending_tab_move.remove(&payload.checkout_id);
            self.sync_async_operations();
            self.set_error("tab.move_worker_failed", message, true);
        }
        true
    }

    /// The device's Herdr control and its connection generation, when the
    /// device can take a tab move now.
    fn device_tab_move_carrier(&self, target: &str) -> Result<(TabMoveCarrier, u64), String> {
        let connected = self
            .snapshot
            .status
            .remote
            .iter()
            .any(|remote| remote.target_id == target && remote.state == "connected");
        if !connected {
            return Err(format!("{target} is not connected; the tab was not moved"));
        }
        let context = self
            .remote_controls
            .get(target)
            .cloned()
            .ok_or_else(|| format!("Herdr control for {target} is unavailable"))?;
        let generation = self
            .remote_connection_generations
            .get(target)
            .copied()
            .unwrap_or(0);
        Ok((TabMoveCarrier::Device(context), generation))
    }
    /// Drops a held reorder and says why, so a refused move is never a strip
    /// that silently stayed where it was.
    pub(super) fn abandon_tab_move(
        &mut self,
        checkout_id: &str,
        generation: u64,
        connection_generation: u64,
        reason: String,
    ) -> bool {
        // A result from a drag a later drag has replaced must not cancel the
        // newer one.
        if self
            .pending_tab_move
            .get(checkout_id)
            .is_none_or(|pending| {
                pending.generation != generation
                    || pending.connection_generation != connection_generation
            })
        {
            return false;
        }
        self.pending_tab_move.remove(checkout_id);
        self.set_error("tab.move_refused", reason, true);
        self.rebuild_tab_strips();
        true
    }
    /// Rewrites every checkout's tab strip, this machine's and each device's,
    /// from the Herdr and editor tabs it currently holds.
    pub(super) fn rebuild_tab_strips(&mut self) {
        let editor_tabs = &self.snapshot.editor.tabs;
        let order = &mut self.checkout_tab_order;
        let pending = &mut self.pending_tab_move;
        // Which Herdr workspace each tab belongs to, so a strip slot is
        // refilled from that workspace's order rather than from a flat one.
        let owners = &tab_owners(&self.herdr_workspace_tab_order);
        let mut live_checkouts = BTreeSet::new();
        // Checkouts whose held arrangement became unreachable. The diagnostic
        // is pushed after the loop, which is where the snapshot is free again.
        let mut dropped_moves = Vec::new();
        for workspace in &mut self.snapshot.navigator.workspaces {
            if workspace.remote_target_id.is_some() {
                continue;
            }
            for checkout in &mut workspace.checkouts {
                live_checkouts.insert(checkout.id.clone());
                let editor = editor_tabs
                    .iter()
                    .filter(|tab| {
                        tab.workspace_id == checkout.workspace_id && tab.checkout_id == checkout.id
                    })
                    .map(StripTabSnapshot::editor)
                    .collect::<Vec<_>>();
                place_checkout_strip(checkout, editor, owners, order, pending, &mut dropped_moves);
            }
        }
        let targets = self
            .snapshot
            .status
            .remote
            .iter()
            .filter(|remote| remote.session.is_some())
            .map(|remote| remote.target_id.clone())
            .collect::<Vec<_>>();
        for target in targets {
            let Some(mut session) = self
                .snapshot
                .status
                .remote
                .iter_mut()
                .find(|remote| remote.target_id == target)
                .and_then(|remote| remote.session.take())
            else {
                continue;
            };
            self.place_device_strips(&target, &mut session, &mut dropped_moves);
            live_checkouts.extend(
                session
                    .workspaces
                    .iter()
                    .flat_map(|workspace| workspace.checkouts.iter())
                    .map(|checkout| checkout.id.clone()),
            );
            if let Some(remote) = self
                .snapshot
                .status
                .remote
                .iter_mut()
                .find(|remote| remote.target_id == target)
            {
                remote.session = Some(session);
            }
        }
        self.checkout_tab_order
            .retain(|checkout_id, _| live_checkouts.contains(checkout_id));
        self.pending_tab_move
            .retain(|checkout_id, _| live_checkouts.contains(checkout_id));
        // A rebuilt entry starts without its agent; name it before the strip
        // is published so a fresh tab never draws as a bare number first.
        sync_strip_agent_identity(
            &mut self.snapshot.navigator.workspaces,
            &self.snapshot.navigator.agents,
        );
        self.report_dropped_tab_moves(dropped_moves);
    }

    /// Places a device's checkout strips by the rule this machine's follow:
    /// the device's Herdr tabs in its host's order, its file tabs in the
    /// slots the operator put them in (`place_checkout_strip`).
    pub(super) fn place_device_strips(
        &mut self,
        target: &str,
        session: &mut RemoteSessionSnapshot,
        dropped_moves: &mut Vec<String>,
    ) {
        let owners = self
            .device_raw_sessions
            .get(target)
            .map(|raw| tab_owners(&device_workspace_tab_order(raw)))
            .unwrap_or_default();
        for workspace in &mut session.workspaces {
            for checkout in &mut workspace.checkouts {
                let editor = self
                    .snapshot
                    .editor
                    .tabs
                    .iter()
                    .filter(|tab| tab.checkout_id == checkout.id)
                    .map(StripTabSnapshot::editor)
                    .collect::<Vec<_>>();
                place_checkout_strip(
                    checkout,
                    editor,
                    &owners,
                    &mut self.checkout_tab_order,
                    &mut self.pending_tab_move,
                    dropped_moves,
                );
            }
        }
    }

    pub(super) fn report_dropped_tab_moves(&mut self, dropped_moves: Vec<String>) {
        for checkout_id in dropped_moves {
            self.push_diagnostic(
                "tab.move.dropped",
                format!(
                    "The tab arrangement held for {checkout_id} was abandoned: its tabs changed before Herdr reported the order"
                ),
            );
        }
        self.sync_async_operations();
    }
    /// Reconciles the focused checkout, its owning workspace, root path, and
    /// active tab projection after a catalog replacement. Catalog rebuilds
    /// happen from both background sync and event handlers, so this policy
    /// must have one implementation to keep those paths convergent.
    pub(super) fn resync_navigator_focus(&mut self) {
        let focused_checkout_exists = self.snapshot.navigator.workspaces.iter().any(|workspace| {
            workspace.checkouts.iter().any(|checkout| {
                Some(checkout.id.as_str()) == self.snapshot.navigator.focused_checkout_id.as_deref()
            })
        });
        if !focused_checkout_exists && self.snapshot.ui_state.focused_checkout_id.is_none() {
            // The catalog's first checkout, passing over one whose folder
            // the node found gone: a restored pane's row stays while its
            // folder is missing, and with no activity its place in the order
            // is its id's, so the window would open on it by chance.
            let starts = || {
                self.snapshot
                    .navigator
                    .workspaces
                    .iter()
                    .flat_map(|workspace| workspace.checkouts.iter())
                    .filter(|checkout| checkout.exists)
            };
            self.snapshot.navigator.focused_checkout_id = starts()
                .find(|checkout| !self.catalog_paths.gone(&checkout.path))
                .or_else(|| starts().next())
                .map(|checkout| checkout.id.clone());
        }
        let focused = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .find_map(|workspace| {
                workspace
                    .checkouts
                    .iter()
                    .find(|checkout| {
                        Some(checkout.id.as_str())
                            == self.snapshot.navigator.focused_checkout_id.as_deref()
                    })
                    .map(|checkout| (workspace.id.clone(), checkout.path.clone()))
            });
        self.snapshot.navigator.focused_workspace_id = focused
            .as_ref()
            .map(|(workspace_id, _)| workspace_id.clone());
        self.snapshot.navigator.root_path = focused.map(|(_, path)| path);
        self.sync_changes_root_path();
        self.sync_active_tab_projection();
        // Every catalog rebuild and every focus change lands here, so this is
        // the one place the pull-request badges and the summary card have to
        // be re-derived from. Doing it at each call site is how the row and
        // the card would come to disagree.
        self.apply_pull_requests();
        self.refresh_worktree_projection();
    }
    /// Decides which tab each checkout shows, given what Herdr says is active
    /// and what Hide has already chosen.
    ///
    /// Hide owns the visible tab, so Herdr's name is read four ways:
    /// it agrees with Hide and confirms a switch in flight; it disagrees while
    /// Hide's notification is still unconfirmed, and Hide keeps its own value
    /// until Herdr answers; it disagrees with nothing in flight, which is an
    /// operator focusing that tab outside Hide, so Hide follows it and reports
    /// the tab and where the change came from; or Herdr names no tab in this
    /// checkout, and Hide keeps showing what it was showing.
    ///
    /// A checkout that has tabs always ends with one of them visible. Leaving
    /// a sibling checkout of a split workspace without an active tab is what
    /// made its canvas draw the empty-checkout state over real panes.
    pub(super) fn reconcile_visible_tabs(
        &mut self,
        workspaces: &mut [WorkspaceSnapshot],
        herdr: &HerdrTabView,
        herdr_moves: Option<&crate::sidebar::SessionTabMoves>,
    ) {
        let mut followed: Vec<(String, String, String)> = Vec::new();
        let mut follow_pane: Option<String> = None;
        let mut clear_group_pane = false;
        // A changed value or a new focus event is an action to follow. Repeated
        // projections of the same event are only state; a later explicit focus
        // on the same Herdr tab still carries a fresh intent after creation.
        let herdr_focus_moved = herdr.focused_tab_id != self.herdr_focused_tab_seen
            || herdr
                .focus_event
                .as_ref()
                .is_some_and(|event| self.herdr_tab_focus_seen.as_ref() != Some(event));
        // Raw close/status snapshots have no stream stamp. They cannot make a
        // previously consumed event fresh; a new replica has a new generation.
        if let Some(event) = &herdr.focus_event {
            self.herdr_tab_focus_seen = Some(event.clone());
        }
        self.herdr_focused_tab_seen = herdr.focused_tab_id.clone();
        // Herdr's moves since the last session answer Hide's requests in
        // the order they were sent; a focus move among them is not followed.
        let answered = self.take_tab_focus_answers(herdr, herdr_moves, herdr_focus_moved);
        let asks = self.tab_asks();
        let focus_answered = herdr_focus_moved
            && herdr
                .focused_tab_id
                .as_ref()
                .is_some_and(|tab_id| answered.iter().any(|request| request.tab_id == *tab_id));
        let selected_pane_id = self.snapshot.terminal.pane_id.clone();
        // The catalog is rebuilt whole on every pass, so a checkout absent
        // from it is gone rather than momentarily missing. Keeping its tab
        // would grow this map for the life of the process.
        let live_checkout_ids = workspaces
            .iter()
            .flat_map(|workspace| workspace.checkouts.iter())
            .map(|checkout| checkout.id.clone())
            .collect::<HashSet<_>>();
        self.visible_tab_ids
            .retain(|checkout_id, _| live_checkout_ids.contains(checkout_id));
        let mut agent_layout_changed = false;
        let mut agent_layout_errors = Vec::new();
        if let Some(store) = self.workspace_views.as_mut() {
            let keys = workspaces
                .iter()
                .flat_map(|workspace| {
                    workspace
                        .checkouts
                        .iter()
                        .map(|checkout| (workspace.device_id.clone(), checkout.path.clone()))
                })
                .collect::<HashSet<_>>();
            store.agent_live.retain(|key| keys.contains(key));
            // A timeout does not prove an external creation did not happen.
            store
                .agent_admissions
                .retain(|key, pending| keys.contains(key) && !pending.is_empty());
            store
                .agent_placements
                .retain(|_, (key, _, _, _)| keys.contains(key));
        }
        for workspace in workspaces.iter_mut() {
            for checkout in workspace.checkouts.iter_mut() {
                // A registered row can exist before the coordinator knows its
                // path. Its empty tab list cannot prune a durable Agent layout
                // or choose a replacement tab until that path is answered.
                if self.catalog_from_coordinator
                    && workspace.device_id == self.node.as_str()
                    && !self.catalog_paths.knows(&checkout.path)
                {
                    continue;
                }
                let key = (workspace.device_id.clone(), checkout.path.clone());
                let mut restored_agent_tab = None;
                let mut unlisted_agent_selection = false;
                let mut placed_tabs: Option<HashSet<String>> = None;
                if workspace.device_id == self.node.as_str() && self.workspace_views.is_some() {
                    let topology = checkout
                        .tabs
                        .iter()
                        .filter_map(|tab| Some((tab.id.clone()?, tab.delegated)))
                        .collect::<Vec<_>>();
                    match self.reconcile_agent_topology(&key, &topology) {
                        Ok(changed) => agent_layout_changed |= changed,
                        Err(error) => {
                            agent_layout_errors.push(format!("{}: {error:?}", checkout.id))
                        }
                    }
                    let store = self.workspace_views.as_mut().expect("checked above");
                    let first = store.agent_live.insert(key.clone());
                    let view = store.views.entry(&key.0, &key.1);
                    let placements = store
                        .agent_placements
                        .iter()
                        .filter(|(id, (scope, _, _, _))| {
                            scope == &key && view.agent_tab_group(id).is_some()
                        })
                        .map(|(id, (_, group, area, index))| {
                            (id.clone(), group.clone(), area.clone(), *index)
                        })
                        .collect::<Vec<_>>();
                    for (id, group, area, index) in placements {
                        let group = group.filter(|id| view.group_layout(Some(id)).is_some());
                        let selected = view.agent_groups.selected.clone();
                        if let Err(error) =
                            view.move_agent_to_group(&id, group.as_deref(), unix_milliseconds())
                        {
                            agent_layout_errors.push(format!("Created tab {id}: {error:?}"));
                            continue;
                        }
                        view.agent_groups.selected = selected;
                        let layout = view
                            .group_layout_mut(group.as_deref())
                            .expect("placement group checked");
                        let target = layout
                            .tree
                            .area(&area)
                            .unwrap_or_else(|| layout.tree.active_area());
                        let area = target.id.clone();
                        let index = index.unwrap_or(target.displays.len());
                        match layout.tree.move_display(
                            &id,
                            &area,
                            index,
                            layout.tree.next_stamp(unix_milliseconds()),
                        ) {
                            Ok(changed) => {
                                agent_layout_changed |= changed;
                                store.agent_placements.remove(&id);
                            }
                            Err(error) => {
                                agent_layout_errors.push(format!("Created tab {id}: {error:?}"))
                            }
                        }
                    }
                    let layout = view.selected_agent_layout();
                    placed_tabs = Some(
                        layout
                            .tree
                            .displays()
                            .map(|t| t.id.clone())
                            .chain(
                                topology
                                    .iter()
                                    .filter(|(_, delegated)| *delegated)
                                    .map(|(id, _)| id.clone()),
                            )
                            .collect(),
                    );
                    // A promoted draft may name a newly opened owner before
                    // its catalog arrives. With no visible tab, the selected
                    // slot still owns the screen; Herdr's old tab cannot
                    // replace it while that creation becomes visible.
                    let missing_visible_tab =
                        self.visible_tab_ids.get(&checkout.id).is_none_or(|id| {
                            !checkout.tabs.iter().any(|tab| tab.id.as_ref() == Some(id))
                        });
                    unlisted_agent_selection = !first
                        && layout.active().is_some_and(|id| {
                            !checkout
                                .tabs
                                .iter()
                                .any(|tab| tab.id.as_deref() == Some(id))
                        });
                    if first
                        || missing_visible_tab
                        || self.visible_tab_ids.get(&checkout.id).is_some_and(|id| {
                            placed_tabs
                                .as_ref()
                                .is_some_and(|placed| !placed.contains(id))
                        })
                    {
                        restored_agent_tab = layout.active().map(str::to_owned);
                    }
                }
                let has_tab = |tab_id: &str| {
                    checkout
                        .tabs
                        .iter()
                        .any(|tab| tab.id.as_deref() == Some(tab_id))
                        && placed_tabs
                            .as_ref()
                            .is_none_or(|placed| placed.contains(tab_id))
                };
                if herdr_focus_moved
                    && herdr.focused_tab_id.as_ref().is_some_and(|id| {
                        placed_tabs
                            .as_ref()
                            .is_some_and(|placed| !placed.contains(id))
                            && checkout.tabs.iter().any(|tab| tab.id.as_ref() == Some(id))
                    })
                {
                    agent_layout_errors.push(format!(
                        "Ignored Herdr focus on waiting Agent tab in {}",
                        checkout.id
                    ));
                }
                let hide_tab = restored_agent_tab.clone().or_else(|| {
                    self.visible_tab_ids
                        .get(&checkout.id)
                        .filter(|tab_id| has_tab(tab_id))
                        .cloned()
                });
                let herdr_tab = herdr
                    .focused_tab_id
                    .as_deref()
                    .filter(|tab_id| {
                        (restored_agent_tab.is_none()
                            // An explicit external selection can supersede a
                            // started slot even in its first catalog gap.
                            || (unlisted_agent_selection
                                && herdr.focus_event.as_ref().is_some_and(|event| !event.creation)))
                            && herdr_focus_moved
                            && has_tab(tab_id)
                            // Creation provenance keeps external admissions from
                            // stealing the canvas. Snapshots without event metadata
                            // use first membership; local creates carry a claim.
                            && (workspace.device_id != self.node.as_str()
                                || self.workspace_views.is_none()
                                || answered.iter().any(|request| request.tab_id == *tab_id)
                                || asks.asked(tab_id)
                                || (herdr.focus_event.as_ref().is_some_and(|event| !event.creation))
                                || (herdr.focus_event.is_none() && self.snapshot.navigator.workspaces.iter().any(|previous| {
                                    previous.checkouts.iter().any(|previous| {
                                        previous.id == checkout.id
                                            && previous.tabs.iter().any(|tab| {
                                                tab.id.as_deref() == Some(*tab_id)
                                            })
                                    })
                                })))
                    })
                    .map(str::to_owned);
                let pending_tab = asks.newest_in(&checkout.id).map(str::to_owned);
                let outstanding = asks.outstanding_in(&checkout.id);
                // The tab holding the selected pane, when it is in this
                // checkout. With no tab of its own yet, Hide shows the tab the
                // keyboard is in rather than one Herdr remembers, so a restore
                // draws the layout it attaches.
                let selected_tab = selected_pane_id.as_deref().and_then(|pane_id| {
                    checkout
                        .tabs
                        .iter()
                        .find(|tab| tab.panes.iter().any(|pane| pane.id == pane_id))
                        .and_then(|tab| tab.id.clone())
                        .filter(|id| has_tab(id))
                });
                // A pending tab Herdr has not listed yet (one just created)
                // keeps its claim on the checkout instead of being replaced
                // by whichever tab is drawn while it arrives.
                let pending_tab_unlisted = pending_tab.as_deref().is_some_and(|tab_id| {
                    !checkout
                        .tabs
                        .iter()
                        .any(|tab| tab.id.as_deref() == Some(tab_id))
                });
                // Herdr arriving at a tab Hide asked for and has since moved
                // off is that request's answer, recorded where it would have
                // been followed.
                if focus_answered
                    && let (Some(hide_tab), Some(tab_id)) =
                        (hide_tab.as_deref(), herdr_tab.as_deref())
                    && hide_tab != tab_id
                {
                    crate::diagnostic!(serde_json::json!({
                        "component": "view_state",
                        "kind": "tab.focus.late_answer",
                        "checkout_id": checkout.id,
                        "from_tab_id": hide_tab,
                        "tab_id": tab_id,
                    }));
                }
                let empty_group = self
                    .workspace_views
                    .as_ref()
                    .and_then(|s| s.views.get(&key.0, &key.1))
                    .is_some_and(|v| {
                        v.selected_agent_layout()
                            .active()
                            .is_none_or(|id| v.agent_groups.drafts.contains(id))
                    });
                let visible = if empty_group {
                    if self.snapshot.navigator.focused_checkout_id.as_deref()
                        == Some(checkout.id.as_str())
                    {
                        // A group's last tab can close while other groups
                        // still have panes. Its now-empty canvas also owns
                        // keyboard focus; a stale pane must not reveal another
                        // group during the selection-alignment pass.
                        clear_group_pane = selected_pane_id.is_some();
                    }
                    None
                } else {
                    match (hide_tab, herdr_tab) {
                        (Some(hide_tab), Some(herdr_tab)) if hide_tab == herdr_tab => {
                            Some(hide_tab)
                        }
                        // Hide's own answer, or Herdr about to be moved again by
                        // a request still outstanding: neither is followed.
                        (Some(hide_tab), Some(_)) if focus_answered || outstanding => {
                            Some(hide_tab)
                        }
                        (Some(hide_tab), Some(herdr_tab)) => {
                            // Following the tab has to bring the keyboard with
                            // it. Leaving the projection on the tab that just
                            // stopped being visible parks the focus ring and
                            // the first responder on a pane nobody can see.
                            if self.snapshot.navigator.focused_checkout_id.as_deref()
                                == Some(checkout.id.as_str())
                            {
                                let first_pane_id = checkout
                                    .tabs
                                    .iter()
                                    .find(|tab| tab.id.as_deref() == Some(herdr_tab.as_str()))
                                    .and_then(|tab| tab.panes.first())
                                    .map(|pane| pane.id.clone());
                                follow_pane = self.tab_focus_pane_id(&herdr_tab, first_pane_id);
                            }
                            followed.push((checkout.id.clone(), hide_tab, herdr_tab.clone()));
                            Some(herdr_tab)
                        }
                        (Some(hide_tab), None) => Some(hide_tab),
                        // With no value of its own yet, Hide takes the first of:
                        // the tab it asked for, the tab holding the keyboard,
                        // the tab Herdr has focused, a tab Herdr shows in any of
                        // the checkout's workspaces, the first tab.
                        (None, herdr_tab) => pending_tab
                            .clone()
                            .filter(|tab_id| has_tab(tab_id))
                            .or(selected_tab)
                            .or(herdr_tab)
                            .or_else(|| {
                                checkout
                                    .tabs
                                    .iter()
                                    .filter_map(|tab| tab.id.as_deref())
                                    .find(|tab_id| {
                                        has_tab(tab_id) && herdr.is_active_in_its_workspace(tab_id)
                                    })
                                    .map(str::to_owned)
                            })
                            .or_else(|| {
                                checkout
                                    .tabs
                                    .iter()
                                    .filter_map(|tab| tab.id.as_deref())
                                    .find(|tab_id| has_tab(tab_id))
                                    .map(str::to_owned)
                            }),
                    }
                };
                match visible {
                    Some(tab_id) => {
                        let selected_waiting = selected_pane_id.as_deref().is_some_and(|pane| {
                            checkout.tabs.iter().any(|tab| {
                                tab.panes.iter().any(|p| p.id == pane)
                                    && tab.id.as_deref().is_some_and(|id| !has_tab(id))
                            })
                        });
                        // On startup a coordinator's first catalog can be
                        // empty. Its later first checkout restores the saved
                        // group before navigator focus has a checkout yet.
                        // Only that unchosen, request-free start may replace
                        // the temporary Herdr keyboard here.
                        let restoring_without_focus = restored_agent_tab.is_some()
                            && self.snapshot.navigator.focused_checkout_id.is_none()
                            && self.snapshot.ui_state.focused_checkout_id.is_none()
                            && !outstanding;
                        if selected_waiting
                            && (self.snapshot.navigator.focused_checkout_id.as_deref()
                                == Some(&checkout.id)
                                || restoring_without_focus)
                        {
                            follow_pane = checkout
                                .tabs
                                .iter()
                                .find(|tab| tab.id.as_deref() == Some(&tab_id))
                                .and_then(|tab| tab.panes.first())
                                .map(|pane| pane.id.clone());
                        }
                        if !pending_tab_unlisted {
                            self.visible_tab_ids
                                .insert(checkout.id.clone(), tab_id.clone());
                        }
                        checkout.active_tab_id = Some(tab_id);
                    }
                    None => {
                        if !pending_tab_unlisted {
                            self.visible_tab_ids.remove(&checkout.id);
                        }
                        checkout.active_tab_id = None;
                    }
                }
            }
        }
        if agent_layout_changed {
            self.persist_workspace_views();
        }
        for error in agent_layout_errors {
            self.push_diagnostic("agent_layout.reconcile_refused", error);
        }
        if clear_group_pane {
            self.select_terminal_pane(None);
            self.operator_focused_pane_id = None;
        } else if let Some(pane_id) = follow_pane {
            self.select_terminal_pane(Some(pane_id));
            // Herdr moved the keyboard, not the operator. The read record
            // follows only a focus the operator made in Hide.
            self.operator_focused_pane_id = None;
        }
        for (checkout_id, hide_tab, herdr_tab) in followed {
            self.supersede_start_navigation();
            crate::diagnostic!(serde_json::json!({
                "component": "view_state",
                "kind": "tab.focus.followed",
                "checkout_id": checkout_id,
                "from_tab_id": hide_tab,
                "to_tab_id": herdr_tab,
                "origin": "herdr",
            }));
            self.push_diagnostic(
                "tab.focus.followed",
                format!(
                    "Herdr focused tab {herdr_tab} in {checkout_id}; Hide was showing {hide_tab}"
                ),
            );
        }
    }
    /// The pane that becoming visible should put the keyboard on: the one the
    /// operator last had in that tab, and its first pane before it has ever
    /// been visited.
    pub(super) fn tab_focus_pane_id(
        &self,
        tab_id: &str,
        first_pane_id: Option<String>,
    ) -> Option<String> {
        self.snapshot
            .pane_layouts
            .iter()
            .find(|layout| layout.tab_id == tab_id)
            .map(|layout| layout.focused_pane_id.clone())
            .or(first_pane_id)
    }
    /// Keeps the focused checkout drawing the tab that holds the keyboard.
    ///
    /// The visible tab and the selected pane are two core-owned values with
    /// one invariant between them: the selected pane lies in the visible tab
    /// of the focused checkout. A tab action moves the pane into the tab; a
    /// pane action, a restore or a retirement moves the tab to the pane,
    /// here. Without it the canvas drew one tab while the pane attached was
    /// in another, and the operator saw five terminals with nothing in them.
    pub(super) fn align_visible_tab_with_selected_pane(&mut self) -> bool {
        let Some(pane_id) = self.snapshot.terminal.pane_id.clone() else {
            return false;
        };
        let Some(checkout_id) = self.snapshot.navigator.focused_checkout_id.clone() else {
            return false;
        };
        let Some(checkout) = self
            .snapshot
            .navigator
            .workspaces
            .iter_mut()
            .flat_map(|workspace| workspace.checkouts.iter_mut())
            .find(|checkout| checkout.id == checkout_id)
        else {
            return false;
        };
        let Some(tab_id) = checkout
            .tabs
            .iter()
            .find(|tab| tab.panes.iter().any(|pane| pane.id == pane_id))
            .and_then(|tab| tab.id.clone())
        else {
            return false;
        };
        if checkout.active_tab_id.as_deref() == Some(tab_id.as_str())
            && self.visible_tab_ids.get(&checkout_id) == Some(&tab_id)
        {
            return false;
        }
        let from_tab_id = checkout.active_tab_id.replace(tab_id.clone());
        self.visible_tab_ids
            .insert(checkout_id.clone(), tab_id.clone());
        self.sync_active_tab_projection();
        crate::diagnostic!(serde_json::json!({
            "component": "view_state",
            "kind": "tab.visible_aligned",
            "checkout_id": checkout_id,
            "from_tab_id": from_tab_id,
            "to_tab_id": tab_id,
            "pane_id": pane_id,
        }));
        self.push_diagnostic(
            "tab.visible_aligned",
            format!("Tab {tab_id} is visible because it holds the selected pane {pane_id}"),
        );
        true
    }
    /// Stops waiting on a view-state notification Herdr never answered.
    ///
    /// The value Hide chose is kept: the operator's tab and pane are Hide's,
    /// and a silent Herdr is a reason to report, not a reason to move the
    /// screen out from under them. Dropping the wait is what lets the next
    /// Herdr event be read as an external focus rather than as a late answer.
    pub(super) fn expire_pending_view_focus(&mut self, now_unix_ms: u64) -> bool {
        let tab_timed_out = self.expire_tab_focus_requests(now_unix_ms);
        // A socket control already has its own bounded transport wait.
        // Expiring its latest intent would misread its eventual response as
        // external focus and could release an older request's successor.
        if self.pane_focus_in_flight.is_some() {
            return tab_timed_out;
        }
        let Some(pending) = self
            .pending_pane_focus
            .take_if(|pending| pending.expired_at(now_unix_ms))
        else {
            return tab_timed_out;
        };
        self.finish_pane_focus_request(
            pending.request_id.as_deref(),
            &pending.target_id,
            "failed",
            Some(format!(
                "Herdr did not confirm pane focus within {VIEW_FOCUS_NOTIFICATION_TIMEOUT_MS} ms."
            )),
            true,
        );
        self.report_timed_out_view_focus(ViewFocusSlot::Pane, &pending.target_id);
        true
    }
    pub(super) fn report_timed_out_view_focus(&mut self, slot: ViewFocusSlot, target_id: &str) {
        let what = slot.what();
        crate::diagnostic!(serde_json::json!({
            "component": "view_state",
            "kind": "view_focus.timed_out",
            "what": what,
            "target_id": target_id,
            "timeout_ms": VIEW_FOCUS_NOTIFICATION_TIMEOUT_MS,
        }));
        self.push_diagnostic(
            "view_focus.timed_out",
            format!(
                "Herdr did not confirm {what} focus {target_id} within {VIEW_FOCUS_NOTIFICATION_TIMEOUT_MS} ms; Hide keeps it"
            ),
        );
    }
    pub(super) fn sync_active_tab_projection(&mut self) {
        let Some(focused_checkout_id) = self.snapshot.navigator.focused_checkout_id.as_deref()
        else {
            self.snapshot.tab = TabSnapshot {
                agent: None,
                naming: Default::default(),
                id: None,
                workspace_id: None,
                checkout_id: None,
                label: None,
                empty: true,
                delegated: false,
                panes: Vec::new(),
            };
            return;
        };
        let Some((workspace_id, checkout)) =
            self.snapshot
                .navigator
                .workspaces
                .iter()
                .find_map(|workspace| {
                    workspace
                        .checkouts
                        .iter()
                        .find(|checkout| checkout.id == focused_checkout_id)
                        .map(|checkout| (workspace.id.clone(), checkout))
                })
        else {
            self.snapshot.tab = TabSnapshot {
                agent: None,
                naming: Default::default(),
                id: None,
                workspace_id: None,
                checkout_id: None,
                label: None,
                empty: true,
                delegated: false,
                panes: Vec::new(),
            };
            return;
        };
        // The visible tab is looked up by the id Hide holds for this checkout.
        // The first tab is not a stand-in for a missing one: reading position
        // as focus is what made a tab move look like a focus change. A
        // checkout that has tabs always names one, so the only tabless case
        // left is a checkout with no tabs at all.
        let active = checkout.active_tab_id.as_deref().and_then(|active_tab_id| {
            checkout
                .tabs
                .iter()
                .find(|tab| tab.id.as_deref() == Some(active_tab_id))
        });
        if let Some(tab) = active {
            self.snapshot.tab = tab.clone();
        } else {
            self.snapshot.tab = TabSnapshot {
                agent: None,
                naming: Default::default(),
                id: None,
                workspace_id: Some(workspace_id),
                checkout_id: Some(checkout.id.clone()),
                label: Some("No tabs".to_owned()),
                empty: true,
                delegated: false,
                panes: Vec::new(),
            };
        }
    }
    /// Applies a live session-sync result: projected agents on success, an
    /// explicit Herdr status on failure. Returns whether the snapshot changed.
    pub fn ingest_session(
        &mut self,
        fetched: Result<SessionSnapshotPayload, SessionFetchError>,
    ) -> bool {
        self.ingest_session_with_catalog(fetched, None)
    }
    /// Applies the authoritative session projection for one configured remote
    /// Herdr target. The last valid session remains visible through a stale or
    /// disconnected interval, while status always names the current failure.
    pub fn ingest_remote_session(
        &mut self,
        target_id: &str,
        fetched: Result<RemoteSessionSnapshot, SessionFetchError>,
    ) -> bool {
        // The server's own answer names its version; the status read the
        // connection made once does not follow a live handoff (PRD
        // device-herdr-update B3).
        let live_herdr_version = match &fetched {
            Ok(session) => session.herdr_version.clone(),
            Err(error) => error
                .protocol_details()
                .and_then(|(_, _, version)| version.map(str::to_owned)),
        };
        // A device's server on an older protocol is the device's to update
        // or hand over, never this machine's to stop (D-15).
        let mut fetched = fetched.map_err(SessionFetchError::for_device);
        // A remote projection is built off the runtime, so it cannot see the
        // read ledger; without this every stopped remote pane published `Done`
        // and demanded a close confirmation. Hide never focuses a remote pane,
        // so the remote server's own focus is the read signal.
        let mut read_changed = false;
        if let Ok(session) = fetched.as_mut() {
            // A node that dials this core sleeps as the core's own machine
            // does (PRD core-host-node-remote-core B13).
            self.stamp_node_agent_sleep(target_id, &mut session.agents);
            read_changed = self.apply_remote_read_state(target_id, session);
            for workspace in &mut session.workspaces {
                for checkout in &mut workspace.checkouts {
                    for tab in &mut checkout.tabs {
                        self.apply_tab_rename(tab);
                    }
                    rename::refresh_strip_labels(checkout);
                }
            }
        }
        // The session is kept as Herdr sent it except for the cwds of tabs Hide
        // created (`clamp_device_created_tabs`); the published one groups its
        // workspaces into the device's projects (`device_catalog`).
        let mut dropped_moves = Vec::new();
        if fetched.is_err() {
            // The device's tab ids are its Herdr's own; a later connection may
            // hand one to a tab Hide never created.
            self.created_device_tabs
                .retain(|(target, _), _| target != target_id);
        }
        let fetched = fetched.map(|mut raw| {
            self.clamp_device_created_tabs(target_id, &mut raw, None);
            self.settle_node_issue_tokens(target_id, &raw);
            // Keep the complete core projection when Herdr's facts did not
            // move. Rebuilding raw rows would temporarily discard resolved
            // state and cross-device lineage, making every idle fetch publish.
            // Catalog/helper changes already refresh this published session.
            let previous = (self.device_raw_sessions.get(target_id) == Some(&raw))
                .then(|| {
                    self.snapshot
                        .status
                        .remote
                        .iter()
                        .find(|status| status.target_id == target_id)
                        .and_then(|status| status.session.clone())
                })
                .flatten();
            let mut derived =
                previous.unwrap_or_else(|| self.derive_device_session(target_id, &raw));
            self.device_raw_sessions.insert(target_id.to_owned(), raw);
            self.place_device_strips(target_id, &mut derived, &mut dropped_moves);
            self.lay_remote_requests(target_id, &mut derived.agents, true);
            derived
        });
        if !dropped_moves.is_empty() {
            self.report_dropped_tab_moves(dropped_moves);
        }
        let session_ok = fetched.is_ok();
        let Some(status_index) = self
            .snapshot
            .status
            .remote
            .iter()
            .position(|status| status.target_id == target_id)
        else {
            self.set_error(
                "remote.target_unknown",
                format!("Remote Herdr sync returned an unconfigured target {target_id}"),
                false,
            );
            return true;
        };

        let was_connected = self.snapshot.status.remote[status_index].state == "connected";
        if (fetched.is_ok() && !was_connected) || (fetched.is_err() && was_connected) {
            let generation = self
                .remote_connection_generations
                .entry(target_id.to_owned())
                .or_insert(0);
            *generation = generation.saturating_add(1);
            self.push_diagnostic(
                "remote.connection_generation_advanced",
                format!(
                    "Advanced remote connection generation for {target_id} after a session state transition"
                ),
            );
        }
        // The machine answers again: its helper link does not wait out a
        // retry that doubled while it was away.
        let reachable_again = session_ok && !was_connected;
        // The row's line holds only while the server it is about still
        // answers with the release it was left on: a server restarted on
        // another release ends it without waiting for the next kit pass (B8).
        if let Some(live) = live_herdr_version.as_deref()
            && self
                .device_herdr_behind
                .get(target_id)
                .is_some_and(|behind| behind.server_version != live)
        {
            self.device_herdr_behind.remove(target_id);
        }
        let herdr_version = live_herdr_version.or_else(|| {
            self.remote_connections
                .get(target_id)
                .and_then(|connection| connection.transport.device().cached_herdr_version())
        });
        let status = &mut self.snapshot.status.remote[status_index];

        let mut changed = read_changed;
        if status.herdr_version != herdr_version {
            status.herdr_version = herdr_version;
            changed = true;
        }
        let mut observed_session = None;
        match fetched {
            Ok(session) => {
                if status.state != "connected"
                    || status.message.is_some()
                    || status.unreachable_since_unix_ms.is_some()
                {
                    status.state = "connected".to_owned();
                    status.message = None;
                    status.unreachable_since_unix_ms = None;
                    changed = true;
                }
                if status.files.root_path.as_deref().is_some_and(|root_path| {
                    !session
                        .workspaces
                        .iter()
                        .flat_map(|workspace| workspace.checkouts.iter())
                        .any(|checkout| checkout.path == root_path)
                }) {
                    status.files = RemoteFileListSnapshot::idle();
                    changed = true;
                }
                if status.session.as_ref() != Some(&session) {
                    Self::log_unknown_descendants(
                        status
                            .session
                            .as_ref()
                            .map(|previous| previous.agents.as_slice())
                            .unwrap_or(&[]),
                        &session.agents,
                    );
                    status.session = Some(session);
                    changed = true;
                }
                observed_session = status.session.clone();
            }
            Err(error) => {
                if status.state != error.state()
                    || status.message.as_deref() != Some(error.message())
                {
                    // The operator sees the plain row; the words the
                    // connection failed with go to the log.
                    crate::diagnostic!(serde_json::json!({
                        "component": "remote_session_sync",
                        "kind": "remote.session_unavailable",
                        "target": target_id,
                        "state": error.state(),
                        "message": error.message(),
                    }));
                    status.state = error.state().to_owned();
                    status.message = Some(error.message().to_owned());
                    changed = true;
                }
                // The first failure after an answer starts the clock; a
                // failure that only changes its reason keeps it.
                if status.unreachable_since_unix_ms.is_none() {
                    status.unreachable_since_unix_ms = Some(unix_milliseconds());
                    changed = true;
                }
            }
        }

        let agent_count = crate::agent_state::remote_session_count(status.session.as_ref());
        if let Some(device) = self
            .snapshot
            .navigator
            .devices
            .iter_mut()
            .find(|device| device.id == target_id)
            && device.agent_count != agent_count
        {
            device.agent_count = agent_count;
            changed = true;
        }
        if reachable_again {
            changed |= self.device_reachable_again(target_id);
        }
        changed |= self.refresh_device_snapshots();
        if observed_session.is_some() {
            self.reconcile_node_agent_topology(target_id);
            changed |= self.follow_start(Some(target_id));
        }
        if let Some(session) = observed_session.as_ref() {
            let (live_pane_ids, active_pane_ids) = remote_terminal_pane_sets(
                session,
                self.device_in_front(target_id),
                self.node_shown_agent_tabs(target_id).as_deref(),
            );
            changed |=
                self.reconcile_remote_terminal_panes(target_id, &live_pane_ids, &active_pane_ids);
        }
        changed |= self.expire_remote_operations(unix_milliseconds());
        if let Some(session) = observed_session.as_ref() {
            changed |= self.observe_remote_operations(target_id, session);
        }
        if session_ok {
            changed |= self.request_device_facts(target_id);
            changed |= self.node_agent_sleep_visit(target_id);
        }
        changed |= self.refresh_agent_lineage();
        changed |= self.advance_tree_closes();
        changed |= self.advance_session_archive();
        if changed {
            self.sync_async_operations();
        }
        changed |= self.refresh_browser_inventory_scope();
        changed |= self.refresh_agent_scopes();
        changed
    }
    /// Reconciles the pane and checkout ids loaded from disk against the first
    /// live session, then retires the hint.
    ///
    /// Persisted ids name a session that has already ended, so a pane Herdr no
    /// longer has is the expected case on launch rather than a fault. Dropping
    /// the unusable parts here lets the ordinary resolution below pick the
    /// session's own focus, and keeps `pane.projection_unavailable` meaning
    /// what it says: a pane the user chose against a live session went away.
    pub(super) fn consume_restore_hint(&mut self, payload: &SessionSnapshotPayload) {
        if !self.restore_hint_pending {
            return;
        }
        // A saved selection this session does not hold is gone, or was this
        // machine's under another core, which saved it as its device's and
        // so never at all (a core move's rekey says so, `focus_from_herdr`).
        // Either way Herdr's own focus is where Hide left the keyboard last,
        // since Hide tells Herdr every focus it takes, so the window opens on
        // Herdr's pane and its checkout; with nothing saved, or Herdr's focus
        // outside every checkout, the catalog's first checkout is the start.
        // Only a pane a catalog checkout holds: one outside every checkout
        // would leave a selected pane with no checkout to show it.
        let herdr_focus = payload.focused_pane_id.as_deref().filter(|pane_id| {
            payload
                .layouts
                .iter()
                .any(|layout| layout.panes.iter().any(|pane| pane.pane_id == *pane_id))
                && self
                    .snapshot
                    .navigator
                    .workspaces
                    .iter()
                    .flat_map(|workspace| workspace.checkouts.iter())
                    .flat_map(|checkout| checkout.tabs.iter())
                    .any(|tab| tab.panes.iter().any(|pane| pane.id == *pane_id))
        });
        let restored_pane_exists = self
            .snapshot
            .ui_state
            .selected_pane_id
            .as_deref()
            .is_some_and(|pane_id| {
                payload
                    .layouts
                    .iter()
                    .any(|layout| layout.panes.iter().any(|pane| pane.pane_id == pane_id))
            });
        let restored_checkout_exists = self
            .snapshot
            .ui_state
            .focused_checkout_id
            .as_deref()
            .is_some_and(|checkout_id| {
                self.snapshot
                    .navigator
                    .workspaces
                    .iter()
                    .flat_map(|workspace| workspace.checkouts.iter())
                    .any(|checkout| checkout.id == checkout_id)
            });
        // A coordinator starts before its first path answer. An absent row
        // while the saved pane's directory is still unknown is not evidence
        // that its checkout disappeared. Unrelated unknown directories must
        // not hold an already answered restore choice.
        if self.catalog_from_coordinator
            && restored_pane_exists
            && self.snapshot.ui_state.focused_checkout_id.is_some()
            && self
                .snapshot
                .ui_state
                .selected_pane_id
                .as_deref()
                .and_then(|pane| Self::pane_cwd(payload, pane))
                .is_some_and(|path| !self.catalog_paths.knows(&path))
        {
            return;
        }
        self.restore_hint_pending = false;
        let moved = std::mem::take(&mut self.snapshot.ui_state.focus_from_herdr);
        let restore_missed = moved
            || (self.snapshot.ui_state.selected_pane_id.is_some() && !restored_pane_exists)
            || (self.snapshot.ui_state.focused_checkout_id.is_some() && !restored_checkout_exists);
        if !restored_pane_exists {
            self.snapshot.ui_state.selected_pane_id =
                herdr_focus.filter(|_| restore_missed).map(str::to_owned);
            self.snapshot.terminal.pane_id = None;
            self.snapshot.focused.pane_id = None;
        }

        if !restored_checkout_exists {
            // The checkout holding the pane the window opens on, so the two
            // never disagree.
            let checkout = self
                .snapshot
                .ui_state
                .selected_pane_id
                .as_deref()
                .filter(|_| restore_missed)
                .and_then(|pane_id| {
                    self.snapshot
                        .navigator
                        .workspaces
                        .iter()
                        .flat_map(|workspace| workspace.checkouts.iter())
                        .find(|checkout| {
                            checkout
                                .tabs
                                .iter()
                                .any(|tab| tab.panes.iter().any(|pane| pane.id == pane_id))
                        })
                        .map(|checkout| checkout.id.clone())
                });
            self.snapshot.ui_state.focused_checkout_id = checkout.clone();
            self.snapshot.navigator.focused_checkout_id = checkout;
            self.resync_navigator_focus();
        }
    }
    /// Keeps the labels the local coordinator published for every later
    /// local projection; see [`crate::labels::overlay::LabelOverlay`].
    pub(crate) fn set_label_overlay(&mut self, overlay: crate::labels::overlay::LabelOverlay) {
        self.label_overlay = overlay;
    }
    /// Like [`Self::ingest_session`], with a workspace catalog the caller
    /// built outside the runtime lock.
    pub fn ingest_session_with_catalog(
        &mut self,
        fetched: Result<SessionSnapshotPayload, SessionFetchError>,
        precomputed: Option<session_sync::PrecomputedCatalog>,
    ) -> bool {
        self.ingest_session_with_focus_check(fetched, precomputed, SessionFocusCheck::Unchecked)
    }

    /// Ingests a session update whose focus is Herdr's own stream's, and
    /// says whether a read of Herdr's focus has to follow.
    ///
    /// The stream has no ordering cursor, so a focus that differs from the
    /// selected pane is not authority until a fresh read agrees. That read
    /// waits on Herdr, so it never gates this update: the layout, the panes
    /// and every other part of the update are taken now, and the differing
    /// focus (and the zoom of the pane it names) stays Hide's until
    /// [`Self::finish_pane_focus_readback`] hears the answer. At most one
    /// read is outstanding.
    pub(crate) fn ingest_session_awaiting_focus_readback(
        &mut self,
        fetched: Result<SessionSnapshotPayload, SessionFetchError>,
        precomputed: Option<session_sync::PrecomputedCatalog>,
    ) -> (bool, Option<PendingPaneFocusControl>) {
        let changed = self.ingest_session_with_focus_check(
            fetched,
            precomputed,
            SessionFocusCheck::Readback(None),
        );
        (changed, self.begin_pane_focus_readback())
    }

    /// An external move within the selected pane's tab, as the stored layout
    /// shows it. Missing panes and tab arrivals keep their existing topology
    /// handling.
    fn pane_focus_readback_target(&self) -> Option<PendingPaneFocusControl> {
        if self.pending_pane_focus.is_some() || self.pane_focus_in_flight.is_some() {
            return None;
        }
        let selected = self.snapshot.focused.pane_id.as_deref()?;
        let layout = self
            .snapshot
            .pane_layouts
            .iter()
            .find(|layout| layout.pane_ids().contains(&selected))?;
        (layout.focused_pane_id != selected).then(|| PendingPaneFocusControl {
            serial: self.next_pane_focus_serial,
            target_id: layout.focused_pane_id.clone(),
            live_generation: self.live_generation,
        })
    }

    /// Takes the one readback slot for the move the stored layouts show. A
    /// move that finds the slot taken is remembered for the answer to settle.
    fn begin_pane_focus_readback(&mut self) -> Option<PendingPaneFocusControl> {
        let identity = self.pane_focus_readback_target()?;
        if self.pane_focus_readback.is_some() {
            self.pane_focus_readback_again = true;
            return None;
        }
        self.pane_focus_readback_again = false;
        self.pane_focus_readback = Some(identity.clone());
        Some(identity)
    }

    #[cfg(test)]
    pub(crate) fn pane_focus_readback_is_outstanding(&self) -> bool {
        self.pane_focus_readback.is_some()
    }

    /// Frees the readback slot for a worker that ended without an answer.
    pub(crate) fn abandon_pane_focus_readback(&mut self) {
        self.pane_focus_readback = None;
        self.pane_focus_readback_again = false;
    }

    /// Settles the readback for `identity` and returns whether the snapshot
    /// changed, with the next readback when the stored layouts still show a
    /// move this answer did not cover.
    ///
    /// A confirmed answer applies the layout the session already stored,
    /// now with its focus and zoom. The identity is checked again there:
    /// a newer selection, a pending focus request or another connection
    /// leaves the answer without authority. An unconfirmed answer changes
    /// nothing and is not asked again until the next session update; an
    /// update that arrived while the read was out is that next update.
    ///
    /// Herdr's focus on another pane is the readback doing its work: the
    /// stream showed a move older than Herdr's focus, most often older than
    /// Hide's own last focus. Only a readback Herdr could not answer while
    /// it was still the latest is a failure, logged with Herdr's reason; one
    /// a newer focus, selection or connection overtook ends quietly.
    pub(crate) fn finish_pane_focus_readback(
        &mut self,
        identity: PendingPaneFocusControl,
        answer: crate::live::FocusReadback,
    ) -> (bool, Option<PendingPaneFocusControl>) {
        self.pane_focus_readback = None;
        let again = std::mem::take(&mut self.pane_focus_readback_again);
        // The move must still be the one the selected pane's layout shows:
        // the apply below rechecks the identity only inside that layout, so
        // an answer for another tab than the selected one has no authority.
        let latest = self.pane_focus_readback_target().as_ref() == Some(&identity);
        if latest && let crate::live::FocusReadback::Unread(message) = &answer {
            crate::diagnostic!(serde_json::json!({
                "component": "pane_focus",
                "kind": "pane.focus.stream_readback_failed",
                "pane_id": identity.target_id,
                "serial": identity.serial,
                "connection_generation": identity.live_generation,
                "message": message,
            }));
        }
        let confirmed = answer == crate::live::FocusReadback::Confirmed;
        let stored = self
            .snapshot
            .pane_layouts
            .iter()
            .find(|layout| layout.focused_pane_id == identity.target_id)
            .cloned();
        let mut changed = false;
        if confirmed
            && latest
            && let Some(layout) = stored.clone()
        {
            changed |= self.apply_pane_layout_with_focus_check(
                layout,
                false,
                SessionFocusCheck::Readback(Some((identity.clone(), true))),
            );
            changed |= self.settle_selection_projection();
            changed |= self.refresh_browser_inventory_scope();
        }
        let next = if confirmed || stored.is_none() || again {
            self.begin_pane_focus_readback()
        } else {
            None
        };
        (changed, next)
    }

    /// What follows a change of the selected pane: the worktree rows, the
    /// visible tab and the attaches the visible tab holds.
    fn settle_selection_projection(&mut self) -> bool {
        let mut changed = self.refresh_worktree_projection();
        changed |= self.align_visible_tab_with_selected_pane();
        changed | self.track_visible_tab_attachments()
    }

    fn ingest_session_with_focus_check(
        &mut self,
        fetched: Result<SessionSnapshotPayload, SessionFetchError>,
        precomputed: Option<session_sync::PrecomputedCatalog>,
        focus_check: SessionFocusCheck,
    ) -> bool {
        self.ingesting_session = true;
        let changed = self.apply_session_update(fetched, precomputed, focus_check);
        self.ingesting_session = false;
        // A close waiting in a tab's line starts from the session just
        // applied, never from the one it replaced.
        self.pump_geometry_queues() || changed
    }

    fn apply_session_update(
        &mut self,
        fetched: Result<SessionSnapshotPayload, SessionFetchError>,
        precomputed: Option<session_sync::PrecomputedCatalog>,
        focus_check: SessionFocusCheck,
    ) -> bool {
        // Sleeping agents Herdr no longer lists are drawn from their records
        // before anything below reads the agents (PRD agent-sleep B10).
        let mut fetched = fetched;
        let mut inline_paths = None;
        if let Ok(payload) = fetched.as_mut() {
            self.label_overlay.apply(payload);
            self.settle_agent_sleep(payload);
            // A session without a catalog reads its paths here, before the
            // clamps read the session's cwds.
            let paths = match &precomputed {
                Some(catalog) => Arc::clone(&catalog.paths),
                None => {
                    let paths = self.paths_without_catalog(payload);
                    inline_paths = Some(Arc::clone(&paths));
                    paths
                }
            };
            let unsettled = self.settle_created_tabs(payload, self.created_tab_clamps(), &paths);
            Self::clamp_created_tab_cwds(payload, &unsettled, &paths);
        } else {
            // Herdr's tab ids are its own server's; a later connection may
            // hand one to a tab Hide never created.
            self.created_tab_checkouts.clear();
            self.provisional_tabs.clear();
        }
        // The session update is this runtime's only regular tick, so it is
        // also where a notification Herdr never answered stops being pending.
        // Doing it first lets this same update be read as an external focus
        // rather than as a late answer to a request that has gone quiet.
        let now_unix_ms = unix_milliseconds();
        let fresh_layout_signatures = fetched.as_ref().ok().map(|payload| {
            payload
                .layouts
                .iter()
                .map(|layout| {
                    (
                        layout.tab_id.clone(),
                        Self::session_layout_signature(layout),
                    )
                })
                .collect::<HashMap<_, _>>()
        });
        let timed_out = self.expire_pending_view_focus(now_unix_ms);
        let tab_move_timed_out = self.expire_tab_moves(now_unix_ms);
        let close_timed_out = self.expire_close_operations(now_unix_ms);
        let pane_operation_timed_out = self.expire_pane_operations(now_unix_ms);
        let close_topology_changed = fetched
            .as_ref()
            .is_ok_and(|payload| self.observe_close_topology(payload));
        let pane_topology_changed = fetched
            .as_ref()
            .is_ok_and(|payload| self.observe_pane_operations(payload));
        if let Ok(payload) = &fetched {
            self.op_timings.applied_tabs(
                payload.layouts.iter().map(|layout| layout.tab_id.as_str()),
                std::time::Instant::now(),
            );
        }
        // Only a focus that has left can be confirmed: a layout that arrives
        // before then is Herdr from before the request, and taking it as the
        // answer kept the focus from ever being sent (#629).
        let session_confirms_pending_pane = fetched.as_ref().ok().is_some_and(|payload| {
            let Some(pending) = self
                .pending_pane_focus
                .as_ref()
                .filter(|pending| pending.sent)
            else {
                return false;
            };
            let herdr_tabs = HerdrTabView::from_payload(payload);
            payload.layouts.iter().any(|layout| {
                layout.focused_pane_id == pending.target_id
                    && layout
                        .panes
                        .iter()
                        .any(|pane| pane.pane_id == pending.target_id)
                    && herdr_tabs.is_active_in_its_workspace(&layout.tab_id)
            })
        });
        // What Hide draws ahead of Herdr is laid over the session only now,
        // after every confirmation above has read Herdr's own layout, so
        // every reader below sees one layout and nothing Hide drew confirms
        // an operation (PRD instant-pane-topology D-07).
        if let Ok(payload) = fetched.as_mut() {
            self.input_requests.mark_laid_out(|pane_id| {
                payload
                    .layouts
                    .iter()
                    .any(|layout| layout.panes.iter().any(|pane| pane.pane_id == pane_id))
            });
            self.overlay_drawn_ahead(payload);
        }
        let previously_projected_pane = self
            .snapshot
            .terminal
            .pane_id
            .as_deref()
            .filter(|pane_id| self.layout_holding_pane(pane_id).is_some())
            .map(str::to_owned);
        let previously_projected_tab = self
            .active_pane_layout()
            .map(|layout| layout.tab_id.clone());
        let previously_projected_in_focused_checkout =
            previously_projected_pane.as_deref().is_some_and(|pane_id| {
                self.snapshot
                    .navigator
                    .focused_checkout_id
                    .as_deref()
                    .and_then(|checkout_id| {
                        self.snapshot
                            .navigator
                            .workspaces
                            .iter()
                            .flat_map(|workspace| workspace.checkouts.iter())
                            .find(|checkout| checkout.id == checkout_id)
                    })
                    .is_some_and(|checkout| {
                        checkout
                            .tabs
                            .iter()
                            .flat_map(|tab| tab.panes.iter())
                            .any(|pane| pane.id == pane_id)
                    })
            });
        let mut requests_changed = false;
        let mut terminal_metadata_changed = false;
        let live_pane_ids = fetched.as_ref().ok().map(|payload| {
            payload
                .layouts
                .iter()
                .flat_map(|layout| layout.panes.iter())
                .map(|pane| pane.pane_id.clone())
                .collect::<HashSet<_>>()
        });
        if let Some(live_pane_ids) = live_pane_ids.as_ref() {
            let keep =
                |pane_id: &str| pane_id.starts_with("remote:") || live_pane_ids.contains(pane_id);
            self.retain_terminal_pane_state(keep);
            // Metadata belongs to the session's panes, not its selection.
            // Prune even when the selected creation still has no layout.
            let before = self.snapshot.terminal.panes.len();
            self.snapshot
                .terminal
                .panes
                .retain(|pane| keep(&pane.pane_id));
            terminal_metadata_changed = before != self.snapshot.terminal.panes.len();
            let panes_closing = &self.panes_closing;
            if self
                .input_requests
                .follow_panes(keep, |pane_id| panes_closing.contains(pane_id))
            {
                self.sync_input_requests();
                requests_changed = true;
            }
        }
        let mut excluded = Vec::new();
        let mut rejected_layouts: Vec<(String, String)> = Vec::new();
        let catalog_changed = fetched
            .as_ref()
            .map(|payload| self.reconcile_session_catalog(payload, precomputed, inline_paths))
            .unwrap_or(false);
        // A refused revision names both protocols; a connected Herdr's details
        // are its version and the revision this build requires, which its
        // snapshot already matched, so Settings can show real values (B5).
        let protocol_details = match fetched.as_ref() {
            Ok(payload) => Some((
                u64::from(crate::herdr_contract::HERDR_PROTOCOL_REVISION),
                u64::from(crate::herdr_contract::HERDR_PROTOCOL_REVISION),
                payload.herdr_version.clone(),
            )),
            Err(error) => error
                .protocol_details()
                .map(|(expected, received, version)| {
                    (expected, received, version.map(str::to_owned))
                }),
        };
        let (state, message, agents, layouts, layout, selection_changed) = match fetched {
            Ok(payload) => {
                self.consume_restore_hint(&payload);
                let focused_checkout = self
                    .snapshot
                    .navigator
                    .focused_checkout_id
                    .as_deref()
                    .and_then(|focused_checkout_id| {
                        self.snapshot
                            .navigator
                            .workspaces
                            .iter()
                            .flat_map(|workspace| workspace.checkouts.iter())
                            .find(|checkout| checkout.id == focused_checkout_id)
                    });
                let focused_checkout_pane_ids = focused_checkout
                    .map(|checkout| {
                        checkout
                            .tabs
                            .iter()
                            .flat_map(|tab| tab.panes.iter())
                            .map(|pane| pane.id.clone())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let focused_checkout_pane_set = focused_checkout_pane_ids
                    .iter()
                    .map(String::as_str)
                    .collect::<HashSet<_>>();
                // A `remote:` id names a pane on another machine, which this
                // local session can never hold, so it is not a local selection
                // that has gone missing. Reading it as one is what left the
                // shell stuck on "Selected pane remote:...:pane:w59:p2 is not
                // available for the selected checkout" after a trip to a
                // remote device and back: every local sync tick compared the
                // leftover remote id against local layouts, never matched, and
                // re-raised the same error. Dropping it here means no path
                // that leaves a remote id in the selection can poison local
                // projection, rather than fixing the one navigation that did.
                let selected_pane_id = self
                    .snapshot
                    .terminal
                    .pane_id
                    .clone()
                    .or_else(|| self.snapshot.ui_state.selected_pane_id.clone())
                    .filter(|pane_id| !pane_id.starts_with("remote:"));
                let selected_still_exists = selected_pane_id.as_deref().is_some_and(|pane_id| {
                    payload
                        .layouts
                        .iter()
                        .any(|layout| layout.panes.iter().any(|pane| pane.pane_id == pane_id))
                });
                let selected_pane_missing = selected_pane_id.is_some() && !selected_still_exists;
                // An acknowledged owner opening has a root pane before its
                // workspace/layout can be projected. The bounded creation
                // record ends on confirmation, timeout or connection loss.
                let selected_creation_pending = selected_pane_missing
                    && selected_pane_id.as_deref().is_some_and(|pane_id| {
                        focused_checkout.is_some_and(|checkout| {
                            self.created_pane_awaits_layout(pane_id, &checkout.path)
                        })
                    });
                let selected_was_projected = previously_projected_in_focused_checkout
                    && selected_pane_id.as_deref() == previously_projected_pane.as_deref();
                let selected_left_focused_checkout =
                    selected_pane_id.as_deref().is_some_and(|pane_id| {
                        focused_checkout.is_some() && !focused_checkout_pane_set.contains(pane_id)
                    });
                // The shell a running close made in its tab was read from
                // Herdr directly, and the event stream can still be behind
                // that read: an update without the shell, or with it before
                // its folder is known, is not its end. The keyboard stays on
                // it rather than fall back to the pane the close is taking.
                let selected_close_shell_pending = (selected_pane_missing
                    || selected_left_focused_checkout)
                    && selected_pane_id
                        .as_deref()
                        .is_some_and(|pane_id| self.close_shell_in_flight(pane_id));
                // A rendered pane that leaves the selected checkout has
                // completed an expected lifecycle transition. Retarget only
                // inside that checkout, or leave it empty. A pane that was
                // never rendered is still a pending or invalid selection and
                // keeps the explicit projection error below.
                let selected_pane_retired = selected_was_projected
                    && (selected_pane_missing || selected_left_focused_checkout)
                    && !selected_close_shell_pending;
                let replacement_pane_id = {
                    selected_pane_retired
                        .then(|| {
                            previously_projected_tab
                                .as_deref()
                                .and_then(|tab_id| {
                                    payload
                                        .layouts
                                        .iter()
                                        .find(|layout| layout.tab_id == tab_id)
                                        .map(|layout| layout.focused_pane_id.as_str())
                                })
                                .filter(|pane_id| focused_checkout_pane_set.contains(*pane_id))
                                .or_else(|| {
                                    // The tab this pass has the checkout
                                    // showing is where the operator looks.
                                    // Herdr's keyboard can be on another Agent
                                    // area's tab, and landing there moves the
                                    // screen with it: a reopen's seed tab,
                                    // followed, went as its restored tab came
                                    // (#425).
                                    self.snapshot
                                        .ui_state
                                        .focused_checkout_id
                                        .as_deref()
                                        .and_then(|checkout_id| {
                                            self.visible_tab_ids.get(checkout_id)
                                        })
                                        .and_then(|tab_id| {
                                            payload
                                                .layouts
                                                .iter()
                                                .find(|layout| &layout.tab_id == tab_id)
                                        })
                                        .map(|layout| layout.focused_pane_id.as_str())
                                        .filter(|pane_id| {
                                            focused_checkout_pane_set.contains(*pane_id)
                                        })
                                })
                                .or_else(|| {
                                    payload.focused_pane_id.as_deref().filter(|pane_id| {
                                        focused_checkout_pane_set.contains(*pane_id)
                                    })
                                })
                                .or_else(|| {
                                    // A close moves Herdr's keyboard to another
                                    // tab, and the pane focus for it can arrive
                                    // after this snapshot; the tab Herdr names now
                                    // is where the operator is looking, not the
                                    // checkout's first pane.
                                    HerdrTabView::from_payload(&payload)
                                        .focused_tab_id
                                        .and_then(|tab_id| {
                                            payload
                                                .layouts
                                                .iter()
                                                .find(|layout| layout.tab_id == tab_id)
                                        })
                                        .map(|layout| layout.focused_pane_id.as_str())
                                        .filter(|pane_id| {
                                            focused_checkout_pane_set.contains(*pane_id)
                                        })
                                })
                                .or_else(|| focused_checkout_pane_ids.first().map(String::as_str))
                                .map(str::to_owned)
                        })
                        .flatten()
                };
                let explicit_checkout_missing =
                    self.snapshot.ui_state.focused_checkout_id.is_some()
                        && focused_checkout.is_none();
                // Once the user or persisted state chooses a pane, a session
                // snapshot that omits that workspace must not silently retarget
                // commands to Herdr's unrelated globally focused workspace.
                let draft_surface = self
                    .front_workspace_key()
                    .and_then(|key| {
                        let view = self.workspace_views.as_ref()?.views.get(&key.0, &key.1)?;
                        let reconciled_tab = focused_checkout
                            .filter(|checkout| key.0 == self.node.as_str() && checkout.path == key.1)
                            .and_then(|checkout| checkout.active_tab_id.as_deref());
                        Some((view, reconciled_tab))
                    })
                    .is_some_and(|(view, reconciled_tab)| {
                        // Reconciliation may already have followed an explicit
                        // external choice while the layout's selection still
                        // awaits the ordinary snapshot alignment pass.
                        reconciled_tab
                            .or_else(|| view.selected_agent_layout().active())
                            .is_none_or(|id| {
                                view.agent_groups.drafts.contains(id)
                                    // An acknowledged start owns its slot
                                    // before the stream supplies its pane.
                                    // Its temporary absence is not permission
                                    // to select another group's first pane.
                                    || !focused_checkout.is_some_and(|checkout| {
                                        checkout.tabs.iter().any(|tab| tab.id.as_deref() == Some(id))
                                    })
                            })
                    });
                let target_pane_id = if draft_surface {
                    None
                } else if selected_pane_retired {
                    replacement_pane_id.as_deref()
                } else if selected_pane_missing
                    || explicit_checkout_missing
                    || selected_close_shell_pending
                {
                    None
                } else if focused_checkout.is_some() {
                    // With no selection, the keyboard lands in the tab the
                    // checkout is showing, on that tab's remembered pane, so
                    // the tab drawn is the tab attached. The first pane of the
                    // checkout is for a checkout that shows no tab yet.
                    let visible_tab_pane_id = self
                        .snapshot
                        .ui_state
                        .focused_checkout_id
                        .as_deref()
                        .and_then(|checkout_id| self.visible_tab_ids.get(checkout_id))
                        .and_then(|tab_id| {
                            payload
                                .layouts
                                .iter()
                                .find(|layout| &layout.tab_id == tab_id)
                        })
                        .map(|layout| layout.focused_pane_id.as_str())
                        .filter(|pane_id| focused_checkout_pane_set.contains(*pane_id));
                    selected_pane_id
                        .as_deref()
                        .filter(|pane_id| {
                            selected_still_exists && focused_checkout_pane_set.contains(*pane_id)
                        })
                        .or(visible_tab_pane_id)
                        .or_else(|| {
                            focused_checkout_pane_ids
                                .iter()
                                .find(|pane_id| {
                                    payload.layouts.iter().any(|layout| {
                                        layout
                                            .panes
                                            .iter()
                                            .any(|pane| pane.pane_id == pane_id.as_str())
                                    })
                                })
                                .map(|pane_id| pane_id.as_str())
                        })
                } else if selected_pane_id.is_some() && !selected_still_exists {
                    None
                } else {
                    selected_pane_id
                        .as_deref()
                        .or(payload.focused_pane_id.as_deref())
                        .or_else(|| {
                            payload
                                .layouts
                                .first()
                                .map(|layout| layout.focused_pane_id.as_str())
                        })
                };
                let selected_pane_invalid_for_context = !draft_surface
                    && selected_pane_id.is_some()
                    && !self.restore_hint_pending
                    && ((focused_checkout.is_some() && target_pane_id.is_none())
                        || (focused_checkout.is_none()
                            && self.snapshot.ui_state.focused_checkout_id.is_some()));
                let mut selection_changed = false;
                if draft_surface {
                    // The first session can have Herdr's keyboard before
                    // path facts discover its checkout. A restored draft or
                    // empty group clears that fallback too, or alignment
                    // below would reveal the old pane's group again.
                    if self.snapshot.terminal.pane_id.is_some()
                        || self.snapshot.ui_state.selected_pane_id.is_some()
                    {
                        self.select_terminal_pane(None);
                        self.operator_focused_pane_id = None;
                        selection_changed = true;
                    }
                } else if selected_pane_retired {
                    let retired_pane_id = selected_pane_id.as_deref().unwrap_or("<missing>");
                    self.fail_pending_pane_focus_for_target(
                        retired_pane_id,
                        format!("Pane {retired_pane_id} retired before Herdr confirmed focus."),
                    );
                    self.clear_selected_terminal_state();
                    self.snapshot.terminal.pane_id = replacement_pane_id.clone();
                    self.snapshot.focused.pane_id = replacement_pane_id.clone();
                    self.snapshot.ui_state.selected_pane_id = replacement_pane_id.clone();
                    if self
                        .snapshot
                        .status
                        .last_error
                        .as_ref()
                        .is_some_and(|error| error.kind == "pane.projection_unavailable")
                    {
                        self.snapshot.status.last_error = None;
                    }
                    let message = replacement_pane_id.as_deref().map_or_else(
                        || {
                            format!(
                                "Retired pane {retired_pane_id}; the selected checkout is now empty"
                            )
                        },
                        |replacement| {
                            format!("Retired pane {retired_pane_id}; selected {replacement}")
                        },
                    );
                    self.push_diagnostic("pane.selection_retired", message);
                    selection_changed = true;
                } else if !selected_creation_pending
                    && !selected_close_shell_pending
                    && (selected_pane_missing || selected_pane_invalid_for_context)
                {
                    let pane_id = selected_pane_id.as_deref().unwrap_or("<missing>");
                    self.fail_pending_pane_focus_for_target(
                        pane_id,
                        format!("Pane {pane_id} is no longer available in the selected checkout."),
                    );
                    self.clear_selected_terminal_state();
                    self.set_error(
                        "pane.projection_unavailable",
                        format!(
                            "Selected pane {pane_id} is not available for the selected checkout; terminal projection is waiting"
                        ),
                        true,
                    );
                }
                let (layouts, rejected) = live::project_layouts(&payload);
                rejected_layouts = rejected;
                let layout = target_pane_id
                    .map(|pane_id| live::project_layout_for_pane(&payload, pane_id))
                    .transpose();
                self.pane_hook_tokens = payload
                    .panes
                    .iter()
                    .map(|pane| {
                        (
                            pane.pane_id.clone(),
                            crate::agent_hooks::PaneHookTokens::read(&pane.tokens),
                        )
                    })
                    .collect();
                let projection = project_agents(payload);
                excluded = projection.excluded;
                match layout {
                    Ok(layout) => (
                        "connected",
                        None,
                        Some(projection.agents),
                        layouts,
                        layout,
                        selection_changed,
                    ),
                    Err(projection_error) => (
                        "malformed",
                        Some(format!(
                            "Herdr pane layout could not be projected: {projection_error}"
                        )),
                        None,
                        layouts,
                        None,
                        selection_changed,
                    ),
                }
            }
            Err(error) => (
                error.state(),
                Some(error.message().to_owned()),
                None,
                Vec::new(),
                None,
                false,
            ),
        };

        // A single unreadable agent record excludes only itself; the
        // exclusion is stated rather than silently folded into the count.
        for exclusion in &excluded {
            let pane_id = exclusion.pane_id.as_deref().unwrap_or("<missing pane id>");
            crate::diagnostic!(serde_json::json!({
                "component": "session",
                "kind": "agent.excluded",
                "pane_id": pane_id,
                "source_index": exclusion.source_index,
                "message": exclusion.reason,
            }));
            self.push_diagnostic(
                "agent.excluded",
                format!("Agent {pane_id} was excluded: {}", exclusion.reason),
            );
        }

        let mut changed = catalog_changed
            || requests_changed
            || selection_changed
            || terminal_metadata_changed
            || timed_out
            || tab_move_timed_out
            || close_timed_out
            || pane_operation_timed_out
            || close_topology_changed
            || pane_topology_changed
            || !excluded.is_empty();
        let (expected_protocol, received_protocol, received_version) = protocol_details
            .map(|(expected, received, version)| (Some(expected), Some(received), version))
            .unwrap_or((None, None, None));
        if self.snapshot.status.herdr.state != state
            || self.snapshot.status.herdr.message.as_deref() != message.as_deref()
            || self.snapshot.status.herdr.expected_protocol != expected_protocol
            || self.snapshot.status.herdr.received_protocol != received_protocol
            || self.snapshot.status.herdr.received_version != received_version
        {
            self.snapshot.status.herdr.state = state.to_owned();
            self.snapshot.status.herdr.message = message;
            self.snapshot.status.herdr.expected_protocol = expected_protocol;
            self.snapshot.status.herdr.received_protocol = received_protocol;
            self.snapshot.status.herdr.received_version = received_version;
            changed = true;
        }
        if let Some(mut agents) = agents {
            self.stamp_agent_sleep(&mut agents);
            self.place_agents_in_navigator(&mut agents);
            // The lineage is built before the read axis is applied, because
            // the read fingerprint carries what each row's descendants are
            // asking for and that is only known once the tree exists.
            if crate::agent_state::prune_lineage_expansion(
                &mut self.snapshot.ui_state.expanded_agent_pane_ids,
                &agents,
                ReadRecordScope::Local,
            ) | crate::agent_state::prune_lineage_expansion(
                &mut self.snapshot.ui_state.sessions_expanded_agent_pane_ids,
                &agents,
                ReadRecordScope::Local,
            ) {
                self.persist_ui_state();
                changed = true;
            }
            // Completions are kept first, so the lineage hands up the ones a
            // restored server no longer reports.
            self.apply_pane_completions(
                &mut agents,
                None,
                ReadRecordScope::Local,
                live_pane_ids.as_ref(),
            );
            self.apply_pane_launches(&agents, ReadRecordScope::Local, live_pane_ids.as_ref());
            // A pull request's checks are a wait the lineage decides and an
            // end the read axis compares, so they are laid before both.
            self.lay_checks(&mut agents);
            crate::agent_state::apply_lineage(
                &mut agents,
                &self.snapshot.navigator.workspaces,
                &self.snapshot.ui_state.expanded_agent_pane_ids,
            );
            changed |= self.apply_pane_read_state(
                &mut agents,
                ReadRecordScope::Local,
                live_pane_ids.as_ref(),
            );
            changed |= self.sync_conversation_modes(&agents);
            self.lay_local_requests(&mut agents, live_pane_ids.as_ref());
            if self.snapshot.navigator.agents != agents {
                Self::log_unknown_descendants(&self.snapshot.navigator.agents, &agents);
                self.snapshot.navigator.agents = agents;
                changed = true;
            }
        }
        changed |= self.refresh_agent_lineage();
        changed |= self.advance_tree_closes();
        changed |= self.advance_session_archive();
        for (tab_id, reason) in &rejected_layouts {
            crate::diagnostic!(serde_json::json!({
                "component": "session",
                "kind": "layout.excluded",
                "tab_id": tab_id,
                "message": reason,
            }));
            self.push_diagnostic(
                "layout.excluded",
                format!("Tab {tab_id} has no drawable layout: {reason}"),
            );
        }
        changed |= self.store_pane_layouts(layouts);
        // After the layouts this update carries, so a move they already show
        // is not asked for again from the previous update's placement.
        changed |= self.relocate_delegated_child_panes();
        if let Some(signatures) = fresh_layout_signatures {
            self.confirmed_pane_layout_signatures = signatures;
        }
        if let Some(layout) = layout {
            if self.snapshot.terminal.pane_id.is_none() {
                let pane_id = layout.focused_pane_id.clone();
                self.snapshot.terminal.pane_id = Some(pane_id.clone());
                self.snapshot.focused.pane_id = Some(pane_id);
            }
            changed |= self.apply_pane_layout_with_focus_check(
                layout,
                session_confirms_pending_pane,
                focus_check,
            );
        }
        changed |= self.settle_selection_projection();
        changed |= self.follow_start(None);
        changed |= self.refresh_sessions_after_catalog_change(catalog_changed);
        changed |= self.refresh_browser_inventory_scope();
        changed | self.refresh_pet()
    }
    /// Moves the navigator to the checkout that owns a pane without emitting
    /// a second Herdr command.
    ///
    /// A relationship action is one intent. If its child belongs to another
    /// checkout, changing only the pane leaves reconciliation scoped to the
    /// old checkout and the next session event rejects Herdr's confirmation.
    /// Moving the local context here lets the following `pane.focus` remain
    /// the single external effect.
    ///
    /// A pane Hide made that Herdr has not laid out yet is its checkout's
    /// already (`created_pane_place`): reading only the catalog left a
    /// fork's keyboard in a pane of a checkout not in front, where the
    /// projection waited on it or followed that checkout's own pane instead.
    fn focus_context_for_pane(&mut self, pane_id: &str) -> bool {
        let checkout_where = |found: &dyn Fn(&CheckoutSnapshot) -> bool| {
            self.snapshot
                .navigator
                .workspaces
                .iter()
                .find_map(|workspace| {
                    workspace
                        .checkouts
                        .iter()
                        .find(|checkout| found(checkout))
                        .map(|checkout| {
                            (
                                workspace.id.clone(),
                                checkout.id.clone(),
                                checkout.path.clone(),
                            )
                        })
                })
        };
        let Some((workspace_id, checkout_id, checkout_path)) = checkout_where(&|checkout| {
            checkout
                .tabs
                .iter()
                .any(|tab| tab.panes.iter().any(|pane| pane.id == pane_id))
        })
        .or_else(|| {
            let (made_in, _) = self.created_pane_place(pane_id)?;
            checkout_where(&|checkout| checkout.path == made_in)
        }) else {
            return false;
        };
        if self.snapshot.navigator.focused_workspace_id.as_deref() == Some(workspace_id.as_str())
            && self.snapshot.navigator.focused_checkout_id.as_deref() == Some(checkout_id.as_str())
        {
            return false;
        }
        let from_workspace_id = self.snapshot.navigator.focused_workspace_id.clone();
        let from_checkout_id = self.snapshot.navigator.focused_checkout_id.clone();
        self.snapshot.navigator.focused_workspace_id = Some(workspace_id.clone());
        self.snapshot.navigator.focused_checkout_id = Some(checkout_id.clone());
        self.snapshot.navigator.root_path = Some(checkout_path);
        self.sync_changes_root_path();
        self.refresh_inactive_groups();
        self.remeasure_disk();
        self.yield_surface_to_terminal();
        crate::diagnostic!(serde_json::json!({
            "component": "view_state",
            "kind": "pane.focus_context_changed",
            "pane_id": pane_id,
            "from_workspace_id": from_workspace_id,
            "from_checkout_id": from_checkout_id,
            "to_workspace_id": workspace_id,
            "to_checkout_id": checkout_id,
        }));
        self.push_diagnostic(
            "pane.focus_context_changed",
            format!("Pane {pane_id} moved focus to checkout {checkout_id}"),
        );
        true
    }

    /// Records the number a page gave an operator focus, so the next snapshot
    /// tells that page how much of what it sent the focused pane already
    /// includes (`OperatorFocusAck`).
    ///
    /// Called for every operator focus the core receives, before it decides
    /// what to do with it: a refused or redundant focus still answers the page.
    /// The record is capped by page, oldest evicted, and an eviction is logged
    /// because the evicted page then reads as having nothing in flight.
    pub(super) fn acknowledge_operator_focus(
        &mut self,
        client_id: Option<&str>,
        sequence: Option<u64>,
    ) {
        let (client_id, sequence) = match (client_id, sequence) {
            (None, None) => return,
            (Some(client_id), Some(sequence))
                if !client_id.is_empty() && client_id.len() <= OPERATOR_FOCUS_CLIENT_ID_LIMIT =>
            {
                (client_id, sequence)
            }
            _ => {
                self.push_diagnostic(
                    "pane.focus.sequence_invalid",
                    "An operator focus carried an unusable client id or sequence".to_owned(),
                );
                return;
            }
        };
        let acks = &mut self.snapshot.focused.operator_focus;
        let applied = match acks.iter().position(|ack| ack.client_id == client_id) {
            Some(index) => {
                let mut ack = acks.remove(index);
                ack.sequence = ack.sequence.max(sequence);
                ack
            }
            None => OperatorFocusAck {
                client_id: client_id.to_owned(),
                sequence,
            },
        };
        acks.push(applied);
        if acks.len() > OPERATOR_FOCUS_CLIENT_LIMIT {
            let evicted = acks.remove(0);
            self.push_diagnostic(
                "pane.focus.sequence_evicted",
                format!(
                    "Dropped the operator focus record of page {} (limit {OPERATOR_FOCUS_CLIENT_LIMIT})",
                    evicted.client_id
                ),
            );
        }
    }

    /// Moves the keyboard focus to a pane and tells Herdr afterwards.
    ///
    /// Hide owns the focused pane, so the focus ring and the first responder
    /// move on this frame rather than on Herdr's confirming event. What Herdr
    /// still owns is which panes exist and how they are split; this only says
    /// which of them has the keyboard.
    ///
    /// For an operator focus the pane also becomes the one the read record
    /// follows. The record is raised here rather than when the resulting
    /// layout lands, so one click on a Done row clears that row inside the
    /// same dispatch. A focus that never reaches Herdr arms nothing.
    pub(super) fn focus_pane(
        &mut self,
        pane_id: String,
        origin: PaneFocusOrigin,
        request_id: Option<String>,
    ) {
        if self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .flat_map(|w| &w.checkouts)
            .flat_map(|c| &c.tabs)
            .any(|tab| {
                tab.panes.iter().any(|pane| pane.id == pane_id)
                    && tab.id.as_ref().is_some_and(|id| self.agent_tab_waiting(id))
            })
        {
            self.set_error(
                "agent_layout.display_limit",
                "This Agent tab is waiting. Close a tab to make room.",
                false,
            );
            return;
        }

        let request_id = request_id.filter(|value| !value.trim().is_empty());
        if let Some(request_id) = request_id.as_deref() {
            if self
                .snapshot
                .status
                .pane_focus_request
                .as_ref()
                .is_some_and(|request| request.request_id == request_id)
            {
                self.push_diagnostic(
                    "pane.focus.duplicate_ignored",
                    format!("Ignored duplicate pane focus request {request_id} for {pane_id}"),
                );
                return;
            }
            self.snapshot.status.pane_focus_request = Some(PaneFocusRequestSnapshot {
                request_id: request_id.to_owned(),
                target_pane_id: pane_id.clone(),
                phase: "pending".to_owned(),
                message: None,
                retryable: false,
            });
        }
        // A refusal still answers the request it refuses, or the control
        // that asked would wait for a receipt that never comes.
        if self.close_operation_holds_pane(&pane_id) {
            let message = format!("Pane {pane_id} is closing; focus was not moved back to it");
            self.finish_pane_focus_request(
                request_id.as_deref(),
                &pane_id,
                "failed",
                Some(message.clone()),
                false,
            );
            self.set_error("pane.close_pending", message, true);
            return;
        }
        if request_id.is_some() && !self.pane_exists_for_focus(&pane_id) {
            let message = format!("Pane {pane_id} is no longer available.");
            self.finish_pane_focus_request(
                request_id.as_deref(),
                &pane_id,
                "failed",
                Some(message.clone()),
                true,
            );
            self.set_error("pane.focus_target_unavailable", message, true);
            return;
        }
        let context_changed = self.focus_context_for_pane(&pane_id);
        let already_focused =
            !context_changed && self.snapshot.focused.pane_id.as_deref() == Some(pane_id.as_str());
        // The persisted selection follows the ring. The shell echoes this
        // field back on every UI-state save, and a stale value there put the
        // keyboard back on the previous pane when the sidebar was toggled.
        self.select_terminal_pane(Some(pane_id.clone()));
        if context_changed {
            self.sync_active_tab_projection();
        }
        // A pane in a tab the checkout is not showing brings its tab forward.
        self.align_visible_tab_with_selected_pane();
        if context_changed {
            self.persist_current_ui_state();
        }
        if self.live.is_none() {
            let message = "Pane focus requires a live Herdr connection".to_owned();
            self.finish_pane_focus_request(
                request_id.as_deref(),
                &pane_id,
                "failed",
                Some(message.clone()),
                true,
            );
            self.set_error("pane.control_unavailable", message, true);
            return;
        }
        // Rule 11: focusing the pane that already has the keyboard, with
        // nothing in flight, converges without a second notification. The
        // look itself still counts, so only the notification is skipped.
        // Settled means Herdr's own layout agrees too. A checkout coming
        // forward selects a pane locally without telling Herdr, and skipping
        // the notification then let Herdr's next layout take the focus back.
        // A tab active only in a background workspace is not global focus.
        let herdr_agrees = self.layout_holding_pane(&pane_id).is_none_or(|layout| {
            layout.focused_pane_id == pane_id
                && (self.herdr_focused_tab_seen.as_deref() == Some(layout.tab_id.as_str())
                    || self.pane_focus_tab_confirmed(&layout.tab_id))
        });
        let notify = !already_focused
            || !herdr_agrees
            || !self.view_focus_settled_on(ViewFocusSlot::Pane, &pane_id)
            || self.pane_focus_in_flight.is_some();
        let same_pending = self
            .pending_pane_focus
            .as_ref()
            .is_some_and(|pending| pending.target_id == pane_id);
        if same_pending {
            // Reaching for the same pending target shares its external effect.
            // A new correlated caller still receives that operation's outcome.
            if request_id.is_some()
                && let Some(pending) = self.pending_pane_focus.as_mut()
            {
                pending.request_id = request_id;
            }
        } else if notify {
            if let Some(pending) = self.pending_pane_focus.take() {
                self.finish_pane_focus_request(
                    pending.request_id.as_deref(),
                    &pending.target_id,
                    "failed",
                    Some("A newer pane focus replaced this request.".to_owned()),
                    true,
                );
            }
            self.push_diagnostic("pane.focus.requested", format!("Focusing pane {pane_id}"));
            let Some(serial) = self.next_pane_focus_serial.checked_add(1) else {
                let message = "Pane focus request serial exhausted.".to_owned();
                self.finish_pane_focus_request(
                    request_id.as_deref(),
                    &pane_id,
                    "failed",
                    Some(message.clone()),
                    true,
                );
                self.set_error("pane.focus_unavailable", message, false);
                return;
            };
            self.next_pane_focus_serial = serial;
            let mut pending = match request_id {
                Some(request_id) => PendingViewFocus::pane_request(pane_id.clone(), request_id),
                None => PendingViewFocus::new(pane_id.clone()),
            };
            pending.pane_control_serial = Some(serial);
            self.pending_pane_focus = Some(pending);
            if let Err(message) = self.submit_pane_focus_turn() {
                self.fail_unsent_pane_focus(&pane_id, message);
                return;
            }
        } else {
            self.finish_pane_focus_request(
                request_id.as_deref(),
                &pane_id,
                "succeeded",
                None,
                false,
            );
        }
        if origin == PaneFocusOrigin::Restore {
            return;
        }
        self.operator_focused_pane_id = Some(pane_id);
        self.refresh_pane_read_state();
    }

    /// Reserves only the latest intent. The running worker calls this after
    /// completing its previous operation, so no focus burst grows a queue.
    pub(super) fn begin_pane_focus_control(
        &mut self,
    ) -> Option<(LiveContext, PendingPaneFocusControl)> {
        if self.pane_focus_in_flight.is_some() {
            return None;
        }
        let context = self.live.as_ref()?.clone();
        let pending = self.pending_pane_focus.as_mut()?;
        let serial = pending.pane_control_serial?;
        pending.sent = true;
        pending.requested_at_unix_ms = unix_milliseconds();
        let control = PendingPaneFocusControl {
            serial,
            target_id: pending.target_id.clone(),
            live_generation: self.live_generation,
        };
        self.send_pane_focus_tab(&control.target_id, serial);
        self.pane_focus_in_flight = Some(control.clone());
        Some((context, control))
    }
    pub(super) fn pane_exists_for_focus(&self, pane_id: &str) -> bool {
        self.snapshot
            .pane_layouts
            .iter()
            .any(|layout| layout.pane_ids().contains(&pane_id))
            || self
                .snapshot
                .terminal
                .panes
                .iter()
                .any(|pane| pane.pane_id == pane_id)
    }
    pub(super) fn finish_pane_focus_request(
        &mut self,
        request_id: Option<&str>,
        target_pane_id: &str,
        phase: &str,
        message: Option<String>,
        retryable: bool,
    ) {
        let Some(request_id) = request_id else { return };
        let Some(request) = self.snapshot.status.pane_focus_request.as_mut() else {
            return;
        };
        if request.request_id != request_id || request.target_pane_id != target_pane_id {
            return;
        }
        request.phase = phase.to_owned();
        request.message = message;
        request.retryable = retryable;
    }
    pub(super) fn finish_pane_focus_request_by_id(
        &mut self,
        request_id: &str,
        phase: &str,
        message: Option<String>,
        retryable: bool,
    ) {
        let Some(request) = self.snapshot.status.pane_focus_request.as_mut() else {
            return;
        };
        if request.request_id != request_id {
            return;
        }
        request.phase = phase.to_owned();
        request.message = message;
        request.retryable = retryable;
    }
    /// A pane focus the lane would not take: it is refused where the operator
    /// asked, and Hide keeps the pane it selected.
    fn fail_unsent_pane_focus(&mut self, pane_id: &str, message: String) {
        self.fail_pending_pane_focus_for_target(pane_id, message.clone());
        self.set_error("pane.focus_failed", message, true);
    }
    pub(super) fn fail_pending_pane_focus_for_target(
        &mut self,
        target_pane_id: &str,
        message: String,
    ) {
        let Some(pending) = self
            .pending_pane_focus
            .as_ref()
            .filter(|pending| pending.target_id == target_pane_id)
            .cloned()
        else {
            return;
        };
        self.pending_pane_focus = None;
        self.drop_pane_focus_tab(pending.pane_control_serial);
        self.finish_pane_focus_request(
            pending.request_id.as_deref(),
            target_pane_id,
            "failed",
            Some(message),
            true,
        );
    }
    /// The layout of the tab that holds this pane. Every tab in the session
    /// has one, so this answers for a pane in any tab, not only the visible
    /// one.
    pub(super) fn layout_holding_pane(&self, pane_id: &str) -> Option<&PaneLayoutSnapshot> {
        self.snapshot
            .pane_layouts
            .iter()
            .find(|layout| layout.pane_ids().contains(&pane_id))
    }
    /// The layout being drawn: the one holding the selected pane.
    pub(super) fn active_pane_layout(&self) -> Option<&PaneLayoutSnapshot> {
        self.snapshot.active_pane_layout()
    }
    /// Replaces the session's layouts wholesale from one session projection.
    ///
    /// Herdr sends the whole session on every topology update, so a
    /// `layout_updated` for one tab arrives as a payload in which only that
    /// tab's entry differs; comparing the projected vector is what keeps the
    /// other tabs' entries and the revision they ride untouched.
    pub(super) fn store_pane_layouts(&mut self, layouts: Vec<PaneLayoutSnapshot>) -> bool {
        let mut layouts = layouts
            .into_iter()
            .map(|layout| self.draw_closes(layout))
            .collect::<Vec<_>>();
        // Sorted by tab id, because Herdr's own order for the layouts array
        // carries no meaning - the tab list is what orders tabs - and a
        // reshuffle of it would otherwise restamp the revisioned section and
        // resend the whole navigator with it.
        layouts.sort_by(|left, right| left.tab_id.cmp(&right.tab_id));
        if self.snapshot.pane_layouts == layouts {
            return false;
        }
        self.snapshot.pane_layouts = layouts;
        true
    }
    pub(super) fn apply_pane_layout(
        &mut self,
        layout: PaneLayoutSnapshot,
        session_confirms_pending_pane: bool,
    ) -> bool {
        self.apply_pane_layout_with_focus_check(
            layout,
            session_confirms_pending_pane,
            SessionFocusCheck::Unchecked,
        )
    }

    fn apply_pane_layout_with_focus_check(
        &mut self,
        layout: PaneLayoutSnapshot,
        session_confirms_pending_pane: bool,
        focus_check: SessionFocusCheck,
    ) -> bool {
        let layout = self.draw_closes(layout);
        let pane_ids = layout
            .pane_ids()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let layout_changed = self
            .snapshot
            .pane_layouts
            .iter()
            .find(|stored| stored.tab_id == layout.tab_id)
            != Some(&layout);
        // The rendered projection is compared on its own, because an
        // unchanged layout no longer implies an unchanged projection. Layouts
        // now survive a tab switch, so this runs for a tab whose geometry
        // Herdr never altered while the panes it puts on the canvas still
        // change. Returning the layout comparison alone would then withhold
        // the notification for a canvas that did change.
        let previous_pane_ids = self
            .snapshot
            .terminal
            .panes
            .iter()
            .map(|pane| pane.pane_id.clone())
            .collect::<Vec<_>>();
        let previous_selected = self.snapshot.terminal.pane_id.clone();
        let previous_zoomed = self.snapshot.zoomed.clone();

        // A pane a visited tab left behind keeps its projection entry. The
        // terminal view the shell holds open for that tab reads its transport
        // state from here, and dropping the entry on every switch is what
        // dropped the attach with it and made the tab come back empty. Only a
        // pane that has left the session goes, and the session is the union
        // of every tab's layout.
        let arriving_tab_id = layout.tab_id.clone();
        match self
            .snapshot
            .pane_layouts
            .iter_mut()
            .find(|stored| stored.tab_id == arriving_tab_id)
        {
            Some(stored) => *stored = layout.clone(),
            None => {
                self.snapshot.pane_layouts.push(layout.clone());
                self.snapshot
                    .pane_layouts
                    .sort_by(|left, right| left.tab_id.cmp(&right.tab_id));
            }
        }
        let session_pane_ids = self
            .snapshot
            .pane_layouts
            .iter()
            .flat_map(|stored| stored.pane_ids())
            .map(str::to_owned)
            .collect::<HashSet<_>>();
        self.snapshot.terminal.panes.retain(|pane| {
            pane.pane_id.starts_with("remote:") || session_pane_ids.contains(&pane.pane_id)
        });
        for pane_id in &pane_ids {
            self.ensure_terminal_pane(pane_id);
        }

        // Hide owns the focused pane. An arriving layout confirms the focus
        // Hide notified Herdr about, or - with nothing in flight - it is a
        // focus made outside Hide and Hide follows it and says so. While a
        // notification is unconfirmed the layout's geometry is taken and its
        // focus is not, so the operator's click is not undone by the frame
        // that was already on its way.
        let previous_focus = self.snapshot.focused.pane_id.clone();
        let pending_pane = self.pending_pane_focus.clone();
        let arriving_confirms_pending_tab = self.tab_asks().asked(&arriving_tab_id);
        let mut adopt_focus = match pending_pane.as_ref() {
            _ if self.pane_focus_in_flight.is_some() => false,
            Some(pending)
                if pending.target_id == layout.focused_pane_id && session_confirms_pending_pane =>
            {
                self.pending_pane_focus = None;
                self.finish_pane_focus_request(
                    pending.request_id.as_deref(),
                    &pending.target_id,
                    "succeeded",
                    None,
                    false,
                );
                true
            }
            Some(_) => false,
            None => true,
        };
        // A direct confirmation may finish while an older stream frame is
        // queued. The stream has no cursor: even with no pending intent, its
        // differing focus is not external authority until a fresh read agrees.
        if adopt_focus
            && previous_focus
                .as_deref()
                .is_some_and(|previous| pane_ids.iter().any(|pane| pane == previous))
            && previous_focus.as_deref() != Some(layout.focused_pane_id.as_str())
            && let SessionFocusCheck::Readback(readback) = focus_check
        {
            adopt_focus = readback.as_ref().is_some_and(|(identity, confirmed)| {
                *confirmed
                    && identity.target_id == layout.focused_pane_id
                    && identity.serial == self.next_pane_focus_serial
                    && identity.live_generation == self.live_generation
                    && self.pending_pane_focus.is_none()
                    && self.pane_focus_in_flight.is_none()
            });
            // `None` is an update held for its readback, which is not a refusal.
            if !adopt_focus && readback.is_some() {
                crate::diagnostic!(serde_json::json!({
                    "component": "pane_focus",
                    "kind": "pane.focus.stream_unconfirmed",
                    "previous_pane_id": previous_focus,
                    "pane_id": layout.focused_pane_id,
                    "serial": self.next_pane_focus_serial,
                    "connection_generation": self.live_generation,
                    "readback_serial": readback.as_ref().map(|(identity, _)| identity.serial),
                    "readback_generation": readback.as_ref().map(|(identity, _)| identity.live_generation),
                }));
            }
        }
        if adopt_focus {
            if previous_focus.as_deref() != Some(layout.focused_pane_id.as_str())
                && pending_pane.is_none()
                && !arriving_confirms_pending_tab
            {
                self.report_followed_pane_focus(previous_focus.as_deref(), &layout.focused_pane_id);
            }
            self.snapshot.terminal.pane_id = Some(layout.focused_pane_id.clone());
            self.snapshot.focused.pane_id = Some(layout.focused_pane_id.clone());
            self.release_operator_focus_if_moved(&previous_focus, &layout.focused_pane_id);
            // Zoom is Herdr's, and its subject is the focused pane. While
            // Hide keeps a focus Herdr has not confirmed, taking the zoom
            // would hide the pane the operator just clicked behind another.
            self.snapshot.zoomed = layout.zoomed.then(|| layout.focused_pane_id.clone());
        }
        if previous_focus != self.snapshot.focused.pane_id {
            sync_pane_status(
                &mut self.snapshot.navigator.workspaces,
                &self.snapshot.navigator.agents,
                self.snapshot.focused.pane_id.as_deref(),
                crate::agent_sleep::SleepMachine::Core,
            );
            self.sync_active_tab_projection();
        }
        let mut notice_cleared = false;
        if self
            .snapshot
            .status
            .last_error
            .as_ref()
            .is_some_and(|error| error.kind == "pane.projection_unavailable")
        {
            self.snapshot.status.last_error = None;
            notice_cleared = true;
        }
        self.sync_focused_terminal_projection();

        if self.live.is_some() {
            for pane_id in pane_ids {
                self.request_terminal_control(&pane_id);
            }
        }
        let projection_changed = notice_cleared
            || previous_selected != self.snapshot.terminal.pane_id
            || previous_focus != self.snapshot.focused.pane_id
            || previous_zoomed != self.snapshot.zoomed
            || previous_pane_ids
                != self
                    .snapshot
                    .terminal
                    .panes
                    .iter()
                    .map(|pane| pane.pane_id.clone())
                    .collect::<Vec<_>>();
        layout_changed || projection_changed
    }
    /// Ends the wait on a pane focus Herdr refused, keeping the pane Hide
    /// chose and reporting the refusal.
    ///
    /// A refusal that names some other pane is not this wait's answer and
    /// is left alone, so a late refusal for a pane the operator has already
    /// moved on from cannot end the wait on the current one.
    pub(super) fn clear_refused_pane_focus(&mut self, pane_id: &str, message: &str) {
        let Some(pending) = self
            .pending_pane_focus
            .take_if(|pending| pending.target_id == pane_id)
        else {
            return;
        };
        self.drop_pane_focus_tab(pending.pane_control_serial);
        self.finish_pane_focus_request(
            pending.request_id.as_deref(),
            pane_id,
            "failed",
            Some(message.to_owned()),
            true,
        );
        self.report_refused_view_focus(ViewFocusSlot::Pane, pane_id, message);
    }
    pub(super) fn report_refused_view_focus(
        &mut self,
        slot: ViewFocusSlot,
        target_id: &str,
        message: &str,
    ) {
        let what = slot.what();
        crate::diagnostic!(serde_json::json!({
            "component": "view_state",
            "kind": slot.refused_kind(),
            slot.id_key(): target_id,
            "message": message,
        }));
        self.push_diagnostic(
            slot.refused_kind(),
            format!(
                "Herdr refused {what} focus {target_id}: {message}; {}",
                slot.kept_phrase()
            ),
        );
    }
    /// The checkout and tab holding `pane_id`, or, for a pane Hide made that
    /// Herdr has not laid out yet, the ones it was made in: a focus on a
    /// fork's pane is a tab move Herdr then answers, and without it Herdr's
    /// step through the source's tab on the way read as a tab Herdr chose.
    pub(super) fn checkout_tab_holding_pane(&self, pane_id: &str) -> Option<(String, String)> {
        let checkouts = || {
            self.snapshot
                .navigator
                .workspaces
                .iter()
                .flat_map(|workspace| &workspace.checkouts)
        };
        checkouts()
            .find_map(|checkout| {
                checkout
                    .tabs
                    .iter()
                    .find(|tab| tab.panes.iter().any(|pane| pane.id == pane_id))
                    .and_then(|tab| tab.id.clone())
                    .map(|tab_id| (checkout.id.clone(), tab_id))
            })
            .or_else(|| {
                let (made_in, tab_id) = self.created_pane_place(pane_id)?;
                checkouts()
                    .find(|checkout| checkout.path == made_in)
                    .map(|checkout| (checkout.id.clone(), tab_id.to_owned()))
            })
    }
    /// Rule 11: whether repeating this view-state change would converge on
    /// what the core already holds, so Herdr needs no second notification.
    /// True when nothing is asked for, or what was asked last is this very
    /// target.
    pub(super) fn view_focus_settled_on(&self, slot: ViewFocusSlot, target_id: &str) -> bool {
        match slot {
            ViewFocusSlot::Tab => self
                .newest_tab_ask()
                .is_none_or(|tab_id| tab_id == target_id),
            ViewFocusSlot::Pane => self
                .pending_pane_focus
                .as_ref()
                .is_none_or(|pending| pending.target_id == target_id),
        }
    }
    /// Reports that Hide moved its keyboard focus to follow a pane focus made
    /// outside it.
    ///
    /// Rule 9: the record names the panes and where the change came from, and
    /// carries nothing about what is in them.
    pub(super) fn report_followed_pane_focus(&mut self, previous: Option<&str>, arriving: &str) {
        self.supersede_start_navigation();
        let from = previous.unwrap_or("<none>").to_owned();
        crate::diagnostic!(serde_json::json!({
            "component": "view_state",
            "kind": "pane.focus.followed",
            "from_pane_id": from,
            "to_pane_id": arriving,
            "origin": "herdr",
        }));
        self.push_diagnostic(
            "pane.focus.followed",
            format!("Herdr focused pane {arriving}; Hide was on {from}"),
        );
    }
    /// Drops the operator focus once Herdr moves focus off the pane the
    /// operator chose.
    ///
    /// The pane has to have been focused before it can be moved away from.
    /// Requiring that is what lets a requested focus survive the stale layout
    /// a tab brings forward on its way: bringing a checkout forward to reach
    /// its pane makes Herdr report that tab's remembered pane first, and
    /// clearing on that would leave the clicked row unread.
    pub(super) fn release_operator_focus_if_moved(
        &mut self,
        previous: &Option<String>,
        arriving: &str,
    ) {
        let Some(operator) = self.operator_focused_pane_id.as_deref() else {
            return;
        };
        if arriving == operator || previous.as_deref() != Some(operator) {
            return;
        }
        self.push_diagnostic(
            "pane.read_focus.released",
            format!("Herdr moved focus from {operator} to {arriving}"),
        );
        self.operator_focused_pane_id = None;
    }
    pub(super) fn begin_task_operation(
        &mut self,
        kind: &str,
        repository_root: Option<String>,
        branch: Option<String>,
        base_branch: Option<String>,
        agent_kind: Option<String>,
    ) -> Result<u64, String> {
        if self
            .snapshot
            .task_operation
            .as_ref()
            .is_some_and(|operation| {
                // A created pane's agent answer still lands in this slot.
                operation.phase == "working" || operation.agent_phase.as_deref() == Some("starting")
            })
        {
            return Err("Another task operation is still running".into());
        }
        self.next_task_operation_id = self.next_task_operation_id.wrapping_add(1).max(1);
        let id = self.next_task_operation_id;
        self.capture_task_start_focus(id, kind);
        self.begin_op_timing(
            &super::op_timing::task_create_op_id(id),
            "tab.create",
            None,
            &[],
        );
        self.snapshot.task_operation = Some(crate::model::TaskOperationSnapshot {
            id,
            // A device task names its device after this returns.
            device_id: None,
            kind: kind.to_owned(),
            phase: "working".into(),
            repository_root,
            branch,
            base_branch,
            path: None,
            pane_id: None,
            agent_kind,
            message: None,
            agent_phase: None,
            agent_message: None,
            request_id: None,
            stays_behind: false,
        });
        Ok(id)
    }
    pub fn ingest_task_operation_result(
        &mut self,
        id: u64,
        result: Result<live::WorktreeTaskOutcome, String>,
    ) -> bool {
        let Some(operation) = self.snapshot.task_operation.as_ref() else {
            return false;
        };
        if operation.id != id || operation.phase != "working" {
            return false;
        }
        let operation_kind = operation.kind.clone();
        let repository_root = operation.repository_root.clone();
        let device = operation.device_id.clone();
        let stays_behind = operation.stays_behind;
        // A task that stays behind (a phone start, or an archived descendant
        // resumed after its root) takes the screen nowhere. Remote agent
        // starts follow through their existing remote-control boundary once
        // the exact pane is listed; other device tasks retain their own flow.
        let should_focus = operation_kind != "branch_migrate"
            && (device.is_none() || operation_kind == "agent_start")
            && !stays_behind
            && self.group_start_should_focus(id);
        let adds_pane = self.group_start_adds_pane(id);
        match result {
            Ok(outcome) => {
                let live::WorktreeTaskOutcome {
                    path,
                    pane_id,
                    created_tab_id,
                    purpose_error,
                    unconfirmed_purpose_token,
                    issue_error,
                    shell,
                } = outcome;
                self.task_pane_shell = Some((id, shell));
                if let Some(tab_id) = created_tab_id.as_deref().filter(|_| !adds_pane) {
                    match device.as_deref() {
                        None => self.record_created_tab_checkout(tab_id, &path),
                        Some(device) => self.record_created_device_tab(device, tab_id, &path),
                    }
                }
                // Only this machine's Herdr reports the events the record
                // times.
                let timing_id = super::op_timing::task_create_op_id(id);
                match (device.as_deref(), created_tab_id.as_deref()) {
                    (None, Some(tab_id)) => {
                        self.op_timings.stamp(
                            &timing_id,
                            super::op_timing::Stage::HerdrAck,
                            std::time::Instant::now(),
                        );
                        self.op_timings
                            .learn(&timing_id, Some(tab_id), Some(pane_id.as_str()));
                        self.await_op_frame(&timing_id, &pane_id, false);
                    }
                    (Some(_), _) => self.op_timings.finish(&timing_id, "device"),
                    (None, None) => self.op_timings.finish(&timing_id, "no_tab"),
                }
                // A tab made in a checkout this machine's Herdr already holds
                // is drawn from this answer (D-04, D-05); a new worktree's
                // workspace is Herdr's to report first.
                if !adds_pane
                    && device.is_none()
                    && let Some(tab_id) = created_tab_id.as_deref()
                    && let Some(workspace_id) = self.owner_workspace_of_path(&path)
                {
                    // The tab is named as the agent start names it.
                    let label = self
                        .snapshot
                        .navigator
                        .workspaces
                        .iter()
                        .flat_map(|workspace| &workspace.checkouts)
                        .find(|checkout| checkout.path == path)
                        .map(|checkout| checkout.next_tab_label.clone())
                        .unwrap_or_default();
                    self.draw_created_tab(super::drawn_ahead::ProvisionalTab {
                        workspace_id: Some(workspace_id),
                        tab_id: tab_id.to_owned(),
                        pane_id: pane_id.clone(),
                        cwd: path.clone(),
                        label,
                        timing_id: Some(timing_id.clone()),
                        recorded_at_unix_ms: unix_milliseconds(),
                    });
                }
                self.complete_group_start(id, created_tab_id.as_deref(), &pane_id, should_focus);
                if let Some(detail) = issue_error {
                    self.push_diagnostic(
                        "checkout_issue.create_failed",
                        format!(
                            "The worktree was created, but its issue link was not saved: {detail}"
                        ),
                    );
                }
                let pane_id = match device.as_deref() {
                    Some(device) => super::operations::remote_pane_id(device, &pane_id),
                    None => pane_id,
                };
                let operation = self
                    .snapshot
                    .task_operation
                    .as_mut()
                    .expect("task operation was checked above");
                operation.phase = "ready".into();
                operation.path = Some(path.clone());
                operation.pane_id = Some(pane_id.clone());
                // The creation is settled here; the chosen agent starts on the
                // same worker afterwards and reports on its own axis.
                operation.agent_phase = operation.agent_kind.as_ref().map(|_| "starting".into());
                // Keys typed for a plain terminal tab follow it once its shell
                // reads them; for an agent they wait for its start
                // (`ingest_task_agent_result`).
                let input_ready = operation.agent_phase.is_none().then(|| pane_id.clone());
                let waits_for_local_catalog = operation.kind == "agent_start"
                    && device.is_none()
                    && self.created_pane_place(&pane_id).is_none()
                    && !self.catalog_workspaces().any(|workspace| {
                        workspace.device_id == self.node.as_str()
                            && workspace.checkouts.iter().any(|checkout| {
                                checkout.path == path
                                    && checkout
                                        .tabs
                                        .iter()
                                        .any(|tab| tab.panes.iter().any(|pane| pane.id == pane_id))
                            })
                    });
                crate::diagnostic!(serde_json::json!({
                    "component": "agent_start",
                    "kind": "start.focus_disposition",
                    "task_id": id,
                    "pane_id": pane_id,
                    "connection_generation": self.live_generation,
                    "should_focus": should_focus,
                    "waiting_for_catalog": waits_for_local_catalog,
                    "local_checkout_owner": device.is_none().then(|| self.owner_workspace_of_path(&path)).flatten(),
                    "focused_checkout_id": self.snapshot.navigator.focused_checkout_id,
                    "selected_pane_id": self.snapshot.focused.pane_id,
                }));
                if should_focus && let Some(device) = device.as_deref() {
                    self.queue_start_focus(id, pane_id.clone());
                    self.follow_start(Some(device));
                } else if should_focus && waits_for_local_catalog {
                    // A first Home's catalog may still contain only the
                    // owner's seed pane. Wait for this returned pane, or the
                    // next session stays scoped to the previous checkout.
                    self.queue_start_focus(id, pane_id.clone());
                } else if should_focus {
                    // Targeted creation intentionally did not focus Herdr.
                    // Use the same ordered, acknowledged focus as a pane press.
                    self.focus_pane(pane_id, PaneFocusOrigin::Restore, None);
                    if self.snapshot.status.last_error.is_none()
                        && !self.device_in_front(self.node.as_str())
                    {
                        self.bring_device_forward(self.node.as_str().to_owned());
                    }
                }
                // A tab that does not take the screen is not where the
                // operator's keys were going, agent or not.
                let origin = super::terminal_input::InputOrigin::Task(id);
                match input_ready {
                    _ if !should_focus || device.is_some() => {
                        self.discard_input_request(&origin, "not_shown")
                    }
                    Some(pane_id) => self.resolve_input_request_at_shell(&origin, &pane_id),
                    None => {}
                }
                let normalized = workspace::comparison_by_names(Path::new(&path));
                self.created_purpose_writes_in_flight.remove(&normalized);
                if let Some(unconfirmed) = unconfirmed_purpose_token {
                    self.unconfirmed_created_purposes
                        .insert(normalized, unconfirmed);
                    suppress_unconfirmed_created_purposes(
                        &self.unconfirmed_created_purposes,
                        &mut self.snapshot.navigator.workspaces,
                        &self.snapshot.navigator.agents,
                    );
                } else {
                    self.unconfirmed_created_purposes.remove(&normalized);
                }
                if let Some(detail) = purpose_error {
                    self.push_diagnostic(
                        "checkout_purpose.create_failed",
                        format!(
                            "The worktree was created, but its purpose was not saved: {detail}"
                        ),
                    );
                }
                if operation_kind == "worktree_create"
                    && let Some(workspace_id) = repository_root.as_deref().and_then(|root| {
                        self.snapshot
                            .navigator
                            .workspaces
                            .iter()
                            .find(|workspace| workspace.path == root)
                            .map(|workspace| workspace.id.clone())
                    })
                {
                    let checkout_id =
                        workspace::checkout_id_for_path(&workspace_id, std::path::Path::new(&path));
                    if !self
                        .snapshot
                        .ui_state
                        .collapsed_checkout_ids
                        .contains(&checkout_id)
                    {
                        self.snapshot
                            .ui_state
                            .collapsed_checkout_ids
                            .push(checkout_id);
                        self.snapshot.ui_state.collapsed_checkout_ids.sort();
                        self.persist_ui_state();
                    }
                }
                self.refresh_worktrees();
            }
            Err(message) => {
                self.discard_input_request(&super::terminal_input::InputOrigin::Task(id), "failed");
                self.op_timings
                    .finish(&super::op_timing::task_create_op_id(id), "failed");
                let operation = self
                    .snapshot
                    .task_operation
                    .as_mut()
                    .expect("task operation was checked above");
                operation.phase = "failed".into();
                operation.message = Some(message);
                self.refresh_worktrees();
            }
        }
        if let Some(device) = device.as_deref() {
            self.request_device_worktrees(device, true);
        }
        true
    }

    pub fn ingest_purpose_operation_result(
        &mut self,
        id: u64,
        result: Result<live::PurposeTaskOutcome, String>,
    ) -> bool {
        let Some(operation) = self.snapshot.task_operation.as_ref() else {
            return false;
        };
        if operation.id != id
            || operation.kind != "checkout_purpose"
            || operation.phase != "working"
        {
            return false;
        }
        let target = self
            .purpose_operation_target
            .as_ref()
            .filter(|target| target.id == id)
            .cloned();
        let mut visible_purpose = None;
        let (phase, message, diagnostic) = match result {
            Ok(live::PurposeTaskOutcome::Saved {
                purpose,
                token_written,
            }) => {
                visible_purpose = Some((purpose, token_written));
                ("ready", None, None)
            }
            Ok(live::PurposeTaskOutcome::GitFailed {
                purpose,
                token_written,
                detail,
            }) => {
                if token_written {
                    visible_purpose = Some((purpose, true));
                }
                (
                    "failed",
                    Some(if token_written {
                        "Saved to Herdr, but git did not record it. Save tries again.".to_owned()
                    } else {
                        "Git did not record it. Your text is kept; Save tries again.".to_owned()
                    }),
                    Some(("checkout_purpose.git_failed", detail)),
                )
            }
            Err(detail) => (
                "failed",
                Some("Herdr did not answer. Your text is kept; Save tries again.".to_owned()),
                Some(("checkout_purpose.token_failed", detail)),
            ),
        };
        if visible_purpose.is_some()
            && target
                .as_ref()
                .is_some_and(|target| target.remote_target_id.is_none())
            && let Some(path) = self
                .snapshot
                .navigator
                .workspaces
                .iter()
                .flat_map(|workspace| workspace.checkouts.iter())
                .find(|checkout| {
                    target
                        .as_ref()
                        .is_some_and(|target| checkout.id == target.checkout_id)
                })
                .map(|checkout| checkout.path.clone())
        {
            let normalized = workspace::comparison_by_names(Path::new(&path));
            self.unconfirmed_created_purposes.remove(&normalized);
        }
        if let Some((purpose, token_written)) = visible_purpose {
            let next = (!purpose.is_empty()).then_some(crate::model::CheckoutPurposeSnapshot {
                text: purpose,
                origin: if token_written {
                    crate::model::CheckoutPurposeOrigin::Token
                } else {
                    crate::model::CheckoutPurposeOrigin::BranchDescription
                },
            });
            match target
                .as_ref()
                .and_then(|target| target.remote_target_id.as_deref())
            {
                Some(remote_target_id) => {
                    if let Some(session) = self
                        .snapshot
                        .status
                        .remote
                        .iter_mut()
                        .find(|status| status.target_id == remote_target_id)
                        .and_then(|status| status.session.as_mut())
                    {
                        if let Some(checkout) = session
                            .workspaces
                            .iter_mut()
                            .flat_map(|workspace| workspace.checkouts.iter_mut())
                            .find(|checkout| {
                                target
                                    .as_ref()
                                    .is_some_and(|target| checkout.id == target.checkout_id)
                            })
                        {
                            checkout.purpose = next;
                        }
                        crate::agent_state::sync_checkout_agent_summaries(
                            &mut session.workspaces,
                            &session.agents,
                        );
                    }
                }
                None => {
                    if let Some(checkout) = self
                        .snapshot
                        .navigator
                        .workspaces
                        .iter_mut()
                        .flat_map(|workspace| workspace.checkouts.iter_mut())
                        .find(|checkout| {
                            target
                                .as_ref()
                                .is_some_and(|target| checkout.id == target.checkout_id)
                        })
                    {
                        checkout.purpose = next;
                    }
                    crate::sidebar::sync_checkout_purposes(
                        &mut self.snapshot.navigator.workspaces,
                        &self.snapshot.navigator.agents,
                    );
                }
            }
        }
        let operation = self
            .snapshot
            .task_operation
            .as_mut()
            .expect("purpose operation was checked above");
        operation.phase = phase.to_owned();
        operation.message = message;
        self.purpose_operation_target = None;
        if let Some((kind, detail)) = diagnostic {
            self.push_diagnostic(kind, detail);
        }
        if phase == "ready" {
            self.refresh_worktrees();
        }
        true
    }

    pub(super) fn fail_purpose_operation(
        &mut self,
        id: u64,
        message: impl Into<String>,
        diagnostic_kind: &'static str,
    ) -> bool {
        let message = message.into();
        let Some(operation) = self.snapshot.task_operation.as_mut() else {
            return false;
        };
        if operation.id != id
            || operation.kind != "checkout_purpose"
            || operation.phase != "working"
        {
            return false;
        }
        operation.phase = "failed".to_owned();
        operation.message = Some(message.clone());
        self.purpose_operation_target = None;
        self.push_diagnostic(diagnostic_kind, message);
        true
    }
    /// Starts an Explorer change, or keeps it behind the one still running.
    /// The page sends a drop and the key that follows it without waiting for
    /// the drop to land, and a change refused because another one ran was
    /// lost with nothing logged and nothing on screen (#630); now they run one
    /// at a time in the order they were asked. At most
    /// `EXPLORER_QUEUE_LIMIT` wait, and the newest past that is refused and
    /// logged. The queue is core state only: the snapshot shows the running
    /// change, never the waiting ones.
    pub(super) fn request_explorer_change(&mut self, request: ExplorerRequest) -> bool {
        let running = self
            .snapshot
            .explorer_operation
            .as_ref()
            .is_some_and(|operation| operation.phase == "working");
        if !running {
            return self.start_explorer_operation(&request);
        }
        if self.explorer_queue.len() >= EXPLORER_QUEUE_LIMIT {
            self.push_diagnostic(
                "explorer.busy",
                format!(
                    "{}: {EXPLORER_QUEUE_LIMIT} file operations are already waiting; this one was not run",
                    request.change.started_from()
                ),
            );
            return true;
        }
        crate::diagnostic!(serde_json::json!({
            "component": "explorer", "kind": "explorer.change_waiting",
            "operation": request.change.kind().as_str(),
            "waiting": self.explorer_queue.len() + 1,
        }));
        let owner = self.front_checkout_owned();
        self.explorer_queue
            .push_back(WaitingExplorerChange { owner, request });
        false
    }
    /// Starts the next waiting change once the running one has settled, planned
    /// against the checkout as it is now. A change that failed or was refused
    /// stops the queue: the changes behind it may rely on it, and the operator
    /// has not seen the failure they would run past. Changes asked in a
    /// checkout that is no longer in front are dropped as well, since the tree
    /// they came from is not the one on screen.
    fn continue_explorer_queue(&mut self) {
        let failed = self
            .snapshot
            .explorer_operation
            .as_ref()
            .is_some_and(|operation| operation.phase == "failed");
        if failed {
            self.drop_explorer_queue("the change before them did not happen");
            return;
        }
        let Some(next) = self.explorer_queue.pop_front() else {
            return;
        };
        if next.owner != self.front_checkout_owned() {
            self.explorer_queue.push_front(next);
            self.drop_explorer_queue("the checkout they were asked in is no longer in front");
            return;
        }
        self.start_explorer_operation(&next.request);
        let running = self
            .snapshot
            .explorer_operation
            .as_ref()
            .is_some_and(|operation| operation.phase == "working");
        if !running {
            // Refused at its turn, or settled in place without a worker; a
            // settle already continued the queue, so what is left here
            // follows a refusal.
            self.drop_explorer_queue("the change before them was refused");
        }
    }
    fn drop_explorer_queue(&mut self, reason: &str) {
        if self.explorer_queue.is_empty() {
            return;
        }
        let count = self.explorer_queue.len();
        self.explorer_queue.clear();
        self.push_diagnostic(
            "explorer.queue_dropped",
            format!("{count} waiting file operation(s) were not run: {reason}"),
        );
    }
    /// Decides an explorer change under the lock and runs it off the lock.
    ///
    /// The decision reads nothing from disk: `plan` refuses a path outside
    /// the checkout in front and a name that is not one component from the
    /// strings alone, and the refusal lands in the slot as a failed
    /// operation so the tree can say why under the row. The change is made
    /// by the host of that checkout's device, this machine's in process or a
    /// device's helper, on a worker with the mutex released, and reports back
    /// through `ingest_explorer_operation_result`; a runtime without a worker
    /// context has no shared mutex and runs it in place.
    ///
    /// `device` is the device the tree that asked was showing. A change for
    /// another device than the checkout in front is refused: the tree it came
    /// from is no longer the one on screen (PRD S5.5 B34).
    ///
    /// The slot holds one change at a time; `request_explorer_change` keeps
    /// the ones asked meanwhile, and this starts one only on an empty slot.
    fn start_explorer_operation(&mut self, request: &ExplorerRequest) -> bool {
        let root = request.root.as_str();
        let started_from = request.change.started_from();
        let device = request.device.as_deref();
        self.next_explorer_operation_id = self.next_explorer_operation_id.wrapping_add(1).max(1);
        let id = self.next_explorer_operation_id;
        let device = device.unwrap_or(self.node.as_str());
        let target = self
            .front_checkout_owned()
            .and_then(|(workspace_id, checkout_id)| {
                self.catalog_checkout(&workspace_id, &checkout_id)
                    .map(|(workspace, checkout)| ExplorerTarget {
                        workspace_id: workspace_id.clone(),
                        checkout_id: checkout_id.clone(),
                        device_id: workspace.device_id.clone(),
                        root: checkout.path.clone(),
                    })
            });
        let planned = match target {
            Some(target) if target.device_id != device => Err(format!(
                "{root} is not on the device in front; nothing was changed"
            )),
            Some(target) if target.root == root => {
                request.change.plan(root).and_then(|operation| {
                    self.document_source(&target.workspace_id, &target.checkout_id)
                        .map(|source| (operation, target, source))
                })
            }
            Some(target) => Err(format!(
                "{root} is not the focused checkout {}",
                target.root
            )),
            None => Err("No checkout is focused".to_owned()),
        };
        let (mut operation, target, (document_root, channel)) = match planned {
            Ok(planned) => planned,
            Err(message) => {
                self.snapshot.explorer_operation = Some(ExplorerOperationSnapshot {
                    id,
                    kind: "refused".to_owned(),
                    phase: "failed".to_owned(),
                    path: started_from.to_owned(),
                    destination: started_from.to_owned(),
                    message: Some(message.clone()),
                });
                self.push_diagnostic("explorer.refused", format!("{started_from}: {message}"));
                return true;
            }
        };
        self.snapshot.explorer_operation = Some(ExplorerOperationSnapshot {
            id,
            kind: operation.kind.as_str().to_owned(),
            phase: "working".to_owned(),
            path: operation.source.clone(),
            destination: operation.destination.clone(),
            message: None,
        });
        crate::diagnostic!(serde_json::json!({
            "component": "explorer", "kind": "explorer.change_requested",
            "operation": operation.kind.as_str(), "id": id, "device": target.device_id,
        }));
        // A drop's bytes wait in hided's staging on this machine, whatever
        // device the checkout is on, so its worker reads them through the
        // core's own node.
        let own_node = if operation.kind == files::ExplorerOperationKind::PathImport {
            let node = self.node.as_str().to_owned();
            self.node_link(&node).ok()
        } else {
            None
        };
        let Some(context) = self.worker_context.clone() else {
            let result = files::run_explorer_operation(
                channel.as_ref(),
                own_node.as_deref(),
                &document_root,
                &mut operation,
            );
            return self.ingest_explorer_operation_result(id, &target, &operation, result);
        };
        let worker_operation = operation.clone();
        let worker_target = target.clone();
        match thread::Builder::new()
            .name(format!("herdr-core-explorer-{}", operation.kind.as_str()))
            .spawn(move || {
                let mut operation = worker_operation;
                let result = files::run_explorer_operation(
                    channel.as_ref(),
                    own_node.as_deref(),
                    &document_root,
                    &mut operation,
                );
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                let changed = match runtime.lock() {
                    Ok(mut guard) => guard.ingest_explorer_operation_result(
                        id,
                        &worker_target,
                        &operation,
                        result,
                    ),
                    Err(_) => return,
                };
                drop(runtime);
                if changed {
                    context.notifier.notify();
                }
            }) {
            Ok(_) => true,
            Err(error) => {
                let message = format!("The file operation worker could not start: {error}");
                self.ingest_explorer_operation_result(id, &target, &operation, Err(message))
            }
        }
    }
    /// The folders the Explorer has expanded on `device`: this machine's in
    /// `expanded_paths`, a device's in its own entry.
    pub(super) fn expanded_paths_on(&mut self, device: &str) -> &mut Vec<String> {
        if device == self.node.as_str() {
            &mut self.snapshot.ui_state.expanded_paths
        } else {
            self.snapshot
                .ui_state
                .device_expanded_paths
                .entry(device.to_owned())
                .or_default()
        }
    }
    /// Settles the slot with what the checkout's host said. Success moves the
    /// selection to the operation's `selection` and carries the paths the
    /// core owns in that checkout - its device's expanded folders and its
    /// open file tabs, with the place their saves go to - from the old path
    /// to the new one, so a renamed folder stays open and a renamed file's
    /// tab saves to the file it shows. The same path in another checkout or
    /// on another device is not touched. An item moved to the Trash drops
    /// its expanded folders and keeps its file tabs with their drafts, each
    /// marked removed: a save does not recreate a trashed file, and the
    /// operator exports or discards the draft (B18). Failure changes no core
    /// state beyond the message.
    pub(crate) fn ingest_explorer_operation_result(
        &mut self,
        id: u64,
        target: &ExplorerTarget,
        operation: &files::ExplorerOperation,
        result: Result<files::Applied, String>,
    ) -> bool {
        let Some(slot) = self.snapshot.explorer_operation.as_mut() else {
            return false;
        };
        if slot.id != id || slot.phase != "working" {
            return false;
        }
        let source = operation.source.clone();
        let mut destination = operation.destination.clone();
        let mut selection = operation.selection.clone();
        // A copy, or a drop, adds items and moves none: no path the core
        // carries (an expanded folder, an open tab) follows it anywhere.
        let adds_only = matches!(
            operation.kind,
            files::ExplorerOperationKind::PathCopy | files::ExplorerOperationKind::PathImport
        );
        match result {
            Ok(applied) => {
                // The host chose the new item's name when the wanted one was
                // taken, so the slot and the selection name what exists.
                if let Some(named) = applied.destination {
                    destination = named.clone();
                    selection = named;
                }
                slot.destination = destination.clone();
                slot.phase = "finished".to_owned();
                let in_checkout: Vec<String> = self
                    .snapshot
                    .editor
                    .tabs
                    .iter()
                    .filter(|tab| {
                        tab.kind == EditorTabKind::File
                            && tab.workspace_id == target.workspace_id
                            && tab.checkout_id == target.checkout_id
                    })
                    .map(|tab| tab.id.clone())
                    .collect();
                if source != destination && !adds_only {
                    for folder in self.expanded_paths_on(&target.device_id).iter_mut() {
                        if let Some(moved) = retarget_path(folder, &source, &destination) {
                            *folder = moved;
                        }
                    }
                    for tab in &mut self.snapshot.editor.tabs {
                        if !in_checkout.contains(&tab.id) {
                            continue;
                        }
                        let Some(moved) = retarget_path(&tab.path, &source, &destination) else {
                            continue;
                        };
                        tab.label = hide_platform::path::wire_name(&moved).to_owned();
                        tab.path = moved.clone();
                        if let Some(document) =
                            self.editor_documents.get_mut(&tab.id).map(Edited::edit)
                        {
                            document.path = moved.clone();
                        }
                        if let Some(place) = self.document_places.get_mut(&tab.id)
                            && let Ok(relative) = files::relative_under(&place.root.path, &moved)
                        {
                            place.relative = relative;
                        }
                    }
                    self.sync_active_editor_document();
                }
                if operation.kind == files::ExplorerOperationKind::PathTrash {
                    self.expanded_paths_on(&target.device_id).retain(|folder| {
                        folder != &source && !folder.starts_with(&format!("{source}/"))
                    });
                    let removed: Vec<String> = self
                        .snapshot
                        .editor
                        .tabs
                        .iter()
                        .filter(|tab| {
                            in_checkout.contains(&tab.id)
                                && retarget_path(&tab.path, &source, &source).is_some()
                        })
                        .map(|tab| tab.id.clone())
                        .collect();
                    for tab_id in removed {
                        if let Some(document) =
                            self.editor_documents.get_mut(&tab_id).map(Edited::edit)
                            && let Some(revision) = document.revision.clone()
                        {
                            document.conflict = Some(crate::model::EditorConflictSnapshot {
                                opened_revision: revision,
                                disk_revision: None,
                            });
                        }
                    }
                    self.sync_active_editor_document();
                }
                self.snapshot.ui_state.selected_path = Some(selection);
                self.push_diagnostic(
                    format!("explorer.{}", operation.kind.as_str()),
                    format!("{source} -> {destination}"),
                );
                // A created file opens as an editor tab from this same result,
                // with no second dispatch from the shell; its read runs on a
                // worker like every open, so the tab follows the selection.
                // New Folder, Rename and move open nothing. If the checkout
                // cannot serve the file it still exists there, so the reason
                // rides the finished slot's message and the tree keeps the
                // created file (B10 pattern).
                if operation.kind == files::ExplorerOperationKind::FileCreate {
                    match self.prepare_file_tab(
                        &target.workspace_id,
                        &target.checkout_id,
                        &destination,
                    ) {
                        Ok(prepared) => self.show_file_tab(
                            prepared,
                            &target.workspace_id,
                            &target.checkout_id,
                            &destination,
                            false,
                        ),
                        Err(message) => {
                            if let Some(slot) = self.snapshot.explorer_operation.as_mut() {
                                slot.message = Some(message.clone());
                            }
                            self.push_diagnostic(
                                "explorer.file_create.open_failed",
                                format!("{destination}: {message}"),
                            );
                        }
                    }
                }
                self.persist_current_ui_state();
            }
            Err(message) => {
                slot.phase = "failed".to_owned();
                slot.message = Some(message.clone());
                self.push_diagnostic(
                    format!("explorer.{}_failed", operation.kind.as_str()),
                    format!(
                        "operation {id} on {}: {source}: {message}",
                        target.device_id
                    ),
                );
            }
        }
        self.continue_explorer_queue();
        true
    }
    /// Keeps the first prompt and the CLI arguments for the agent a task
    /// starts. Only the most recent task has them; they are emptied when its
    /// start is settled, except after a definite failure, which Retry reuses.
    pub(super) fn set_task_agent_launch(
        &mut self,
        id: u64,
        prompt: Option<String>,
        args: Vec<String>,
        launch: hide_agent_adapter::LaunchChoice,
    ) {
        self.task_agent_launch = Some(crate::runtime::TaskAgentLaunch {
            task: id,
            prompt: prompt
                .map(|prompt| prompt.trim().to_owned())
                .filter(|prompt| !prompt.is_empty()),
            args,
            launch,
            resume_reference: None,
        });
    }

    /// Adds CLI arguments learned after the task began (a Home start's
    /// linked folders, known once its helper answered).
    pub(super) fn extend_task_agent_args(&mut self, id: u64, args: Vec<String>) {
        if let Some(launch) = self
            .task_agent_launch
            .as_mut()
            .filter(|launch| launch.task == id)
        {
            launch.args.extend(args);
        }
    }

    /// The agent the task's worker should now start: the created pane, the
    /// chosen kind, its first prompt and its CLI arguments, once the creation
    /// is settled.
    pub(crate) fn task_session_control_is_current(
        &self,
        id: u64,
        connector: &Arc<dyn hide_herdr_client::ApiConnector>,
        generation: Option<u64>,
    ) -> bool {
        let Some(operation) = self
            .snapshot
            .task_operation
            .as_ref()
            .filter(|operation| operation.id == id)
        else {
            return false;
        };
        match operation.device_id.as_deref() {
            Some(device) if device != self.node.as_str() => {
                self.remote_coordinator_is_current(device, connector)
            }
            _ => {
                generation == Some(self.live_generation)
                    && self
                        .live
                        .as_ref()
                        .is_some_and(|live| Arc::ptr_eq(&live.api_connector, connector))
            }
        }
    }

    pub(crate) fn task_session_control_generation(
        &self,
        id: u64,
        connector: &Arc<dyn hide_herdr_client::ApiConnector>,
    ) -> Option<u64> {
        self.task_session_control_is_current(id, connector, Some(self.live_generation))
            .then_some(self.live_generation)
    }

    pub(crate) fn pending_task_agent_start(&self, id: u64) -> Option<live::PendingAgentStart> {
        let operation = self.snapshot.task_operation.as_ref()?;
        if operation.id != id
            || operation.phase != "ready"
            || operation.agent_phase.as_deref() != Some("starting")
        {
            return None;
        }
        // Herdr knows a device's pane by its own id, not the scoped one.
        let pane_id = operation.pane_id.as_deref()?;
        let pane_id = match operation.device_id.as_deref() {
            Some(device) => super::remote_pane_source_id(device, pane_id)?,
            None => pane_id,
        };
        let launch = self
            .task_agent_launch
            .as_ref()
            .filter(|launch| launch.task == id);
        Some(live::PendingAgentStart {
            // Only these tasks make the worktree their agent starts in.
            made_worktree: matches!(
                operation.kind.as_str(),
                "worktree_create" | "branch_migrate"
            )
            .then(|| operation.path.clone())
            .flatten()
            .filter(|path| operation.repository_root.as_deref() != Some(path.as_str())),
            resume_reference: launch.and_then(|launch| launch.resume_reference.clone()),
            resume_scope: launch
                .filter(|launch| {
                    operation
                        .agent_kind
                        .as_deref()
                        .and_then(hide_agent_adapter::adapter)
                        .filter(|adapter| {
                            adapter
                                .session
                                .is_some_and(|format| format.requires_native_proof())
                        })
                        .and_then(|adapter| adapter.resume)
                        .is_some_and(|dialect| {
                            launch.args.first().map(String::as_str) == Some(dialect.resume_flag())
                        })
                })
                .and_then(|launch| {
                    Some(hide_session::SessionReadScope {
                        id: launch.args.get(1)?.clone(),
                        cwd: operation.path.clone()?,
                    })
                }),
            pane_id: pane_id.to_owned(),
            kind: operation.agent_kind.clone()?,
            prompt: launch.and_then(|launch| launch.prompt.clone()),
            args: launch.map(|launch| launch.args.clone()).unwrap_or_default(),
            codex_daemon: self
                .codex_daemon(operation.device_id.as_deref().unwrap_or(self.node.as_str())),
            shell: self
                .task_pane_shell
                .clone()
                .filter(|(task, _)| *task == id)
                .map_or(crate::agent_start::ShellStart::Plain, |(_, shell)| shell),
        })
    }

    pub(crate) fn ingest_task_agent_result(
        &mut self,
        id: u64,
        outcome: live::TaskAgentOutcome,
    ) -> bool {
        if !self
            .snapshot
            .task_operation
            .as_ref()
            .is_some_and(|operation| {
                operation.id == id && operation.agent_phase.as_deref() == Some("starting")
            })
        {
            return false;
        }
        // A definite failure keeps the prompt for Retry; any other answer
        // settles it, since the agent may be running.
        let launch = if !matches!(outcome, live::TaskAgentOutcome::Failed(_))
            && self
                .task_agent_launch
                .as_ref()
                .is_some_and(|launch| launch.task == id)
        {
            self.task_agent_launch.take().map(|launch| launch.launch)
        } else {
            None
        };
        // The pane keeps the choice its agent started with, for the restarts
        // Hide makes of it, under the name the start gave the agent: the task
        // worker names it from Herdr's own id for the pane.
        if let (live::TaskAgentOutcome::Started, Some(launch)) = (&outcome, &launch)
            && let Some((pane_id, kind, name)) =
                self.snapshot.task_operation.as_ref().and_then(|operation| {
                    let pane_id = operation.pane_id.clone()?;
                    let kind = operation.agent_kind.clone()?;
                    let herdr_pane_id = match operation.device_id.as_deref() {
                        Some(device) => super::remote_pane_source_id(device, &pane_id)?,
                        None => pane_id.as_str(),
                    };
                    let name = crate::fork::task_agent_name(&kind, herdr_pane_id);
                    Some((pane_id, kind, name))
                })
        {
            self.record_pane_launch(&pane_id, &kind, &name, launch, None);
        }
        let Some(operation) = self.snapshot.task_operation.as_mut() else {
            return false;
        };
        let (phase, message) = match outcome {
            live::TaskAgentOutcome::Started => ("started", None),
            live::TaskAgentOutcome::Failed(message) => ("failed", Some(message)),
            live::TaskAgentOutcome::Unknown(message) => ("unknown", Some(message)),
        };
        let started_pane = (phase == "started" && operation.device_id.is_none())
            .then(|| operation.pane_id.clone());
        crate::diagnostic!(serde_json::json!({
            "component": "task_operation",
            "kind": format!("agent.{phase}"),
            "id": id,
            "pane_id": operation.pane_id,
            "agent_kind": operation.agent_kind,
        }));
        operation.agent_phase = Some(phase.to_owned());
        operation.agent_message = message;
        // Keys typed while the agent's tab was being made reach the agent
        // once it has started, never the launch command.
        let origin = super::terminal_input::InputOrigin::Task(id);
        match started_pane.flatten() {
            Some(pane_id) => self.resolve_input_request(&origin, &pane_id),
            None => self.discard_input_request(&origin, phase),
        }
        self.save_group_agent_result(id, phase);
        true
    }

    /// Starts the chosen agent again in the pane the task already created.
    /// Only a definite failure is retried; an unknown answer may have started
    /// the agent, and starting it twice is not a recovery.
    pub(super) fn retry_task_agent(&mut self, id: u64) -> bool {
        let Some(operation) = self.snapshot.task_operation.as_ref().filter(|operation| {
            operation.id == id && operation.agent_phase.as_deref() == Some("failed")
        }) else {
            self.set_error(
                "task_operation.agent_retry_unavailable",
                "That agent start is no longer waiting for a retry",
                false,
            );
            return true;
        };
        let device = operation.device_id.clone();
        let workspaces = match device.as_deref() {
            Some(device) => self
                .snapshot
                .status
                .remote
                .iter()
                .find(|status| status.target_id == device)
                .and_then(|status| status.session.as_ref())
                .map(|session| session.workspaces.as_slice())
                .unwrap_or_default(),
            None => self.snapshot.navigator.workspaces.as_slice(),
        };
        let pane_listed = operation.pane_id.as_deref().is_some_and(|pane_id| {
            workspaces
                .iter()
                .flat_map(|workspace| &workspace.checkouts)
                .flat_map(|checkout| &checkout.tabs)
                .flat_map(|tab| &tab.panes)
                .any(|pane| pane.id == pane_id)
        });
        if !pane_listed {
            self.set_error(
                "task_operation.agent_retry_pane_gone",
                "The pane the agent was meant to start in is gone",
                false,
            );
            return true;
        }
        let context = match device.as_deref() {
            Some(device) => self.device_worktree_target(device),
            None => self.local_worktree_target(),
        };
        let context = match context {
            Ok(context) => context,
            Err(message) => {
                self.set_error("task_operation.agent_retry_offline", message, true);
                return true;
            }
        };
        let operation = self
            .snapshot
            .task_operation
            .as_mut()
            .expect("checked above");
        operation.agent_phase = Some("starting".into());
        operation.agent_message = None;
        if let Err(message) = live::spawn_task_agent_start(context, id) {
            return self.ingest_task_agent_result(id, live::TaskAgentOutcome::Failed(message));
        }
        true
    }

    pub(super) fn acknowledge_task_operation(&mut self, id: u64) -> bool {
        if self
            .snapshot
            .task_operation
            .as_ref()
            .is_some_and(|operation| {
                operation.id == id
                    && operation.phase != "working"
                    // The agent's answer is still on its way to this slot.
                    && operation.agent_phase.as_deref() != Some("starting")
            })
        {
            self.snapshot.task_operation = None;
            true
        } else {
            false
        }
    }
    pub fn ingest_local_control_result(
        &mut self,
        action: RemoteControlAction,
        result: Result<RemoteControlOutcome, String>,
        elapsed_ms: u128,
    ) -> bool {
        self.ingest_local_control_result_with_failure(
            action,
            result.map_err(live::ControlFailure::Definite),
            elapsed_ms,
        )
    }

    pub(crate) fn ingest_local_control_failure(
        &mut self,
        action: RemoteControlAction,
        result: Result<RemoteControlOutcome, live::ControlFailure>,
        elapsed_ms: u128,
    ) -> bool {
        self.ingest_local_control_result_with_failure(action, result, elapsed_ms)
    }

    fn ingest_local_control_result_with_failure(
        &mut self,
        action: RemoteControlAction,
        result: Result<RemoteControlOutcome, live::ControlFailure>,
        elapsed_ms: u128,
    ) -> bool {
        let action_kind = action.kind();
        let admission = match &action {
            RemoteControlAction::CreateTab {
                admission_id: Some(id),
                ..
            }
            | RemoteControlAction::OpenOwner {
                admission_id: Some(id),
                ..
            } => Some(*id),
            _ => None,
        };
        let timing_id = admission.map(super::op_timing::tab_create_op_id);
        if let Some(id) = admission {
            let origin = super::terminal_input::InputOrigin::TabCreate(id);
            match &result {
                Ok(RemoteControlOutcome::Acknowledged { .. }) => {}
                Ok(_) => self.discard_input_request(&origin, "no_pane"),
                Err(error) if error.is_ambiguous() => {
                    self.discard_input_request(&origin, "unknown")
                }
                Err(_) => self.discard_input_request(&origin, "refused"),
            }
        }
        if let Some(id) = timing_id.as_deref() {
            match &result {
                Ok(RemoteControlOutcome::Acknowledged {
                    created_tab_id,
                    created_pane_id,
                }) => {
                    self.op_timings.stamp(
                        id,
                        super::op_timing::Stage::HerdrAck,
                        std::time::Instant::now(),
                    );
                    self.op_timings.learn(
                        id,
                        created_tab_id.as_deref(),
                        created_pane_id.as_deref(),
                    );
                    if let Some(pane_id) = created_pane_id.as_deref() {
                        self.await_op_frame(id, pane_id, false);
                    }
                }
                Ok(_) => {}
                Err(error) if error.is_ambiguous() => self.op_timings.finish(id, "unknown"),
                Err(_) => self.op_timings.finish(id, "failed"),
            }
        }
        if !result.as_ref().is_err_and(|error| error.is_ambiguous()) {
            match &action {
                RemoteControlAction::CreateTab {
                    cwd,
                    admission_id: Some(id),
                    ..
                }
                | RemoteControlAction::OpenOwner {
                    cwd,
                    admission_id: Some(id),
                    ..
                } => self.finish_agent_admission(cwd, *id),
                _ => {}
            }
        }
        if let RemoteControlAction::RenameTab { request_id, .. } = &action {
            return self.ingest_tab_rename_result(
                request_id,
                result
                    .map(|_| ())
                    .map_err(|error| error.message().to_owned()),
            );
        }
        if let RemoteControlAction::MoveTab {
            checkout_id,
            tab_id,
            expected_order,
            generation,
            connection_generation,
            ..
        } = &action
        {
            return self.ingest_tab_move_result(
                TabMoveResultContext {
                    checkout_id,
                    tab_id,
                    expected_order,
                    generation: *generation,
                    connection_generation: *connection_generation,
                    elapsed_ms,
                },
                result,
            );
        }
        match result {
            Ok(RemoteControlOutcome::Acknowledged {
                created_tab_id,
                created_pane_id,
            }) => {
                if let (
                    Some(tab_id),
                    RemoteControlAction::CreateTab { cwd, .. }
                    | RemoteControlAction::OpenOwner { cwd, .. },
                ) = (created_tab_id.as_deref(), &action)
                {
                    self.record_created_tab_checkout(tab_id, cwd);
                }
                // The tab is drawn from this answer, as one pane with Herdr's
                // ids, until Herdr's layout replaces it (D-03, D-05). Opening
                // an owner may also make a workspace: keep the same pending
                // state, but wait for Herdr to name that workspace to draw it.
                let creation = match &action {
                    RemoteControlAction::CreateTab {
                        workspace_id,
                        cwd,
                        label,
                        ..
                    } => Some((Some(workspace_id.clone()), cwd, label)),
                    RemoteControlAction::OpenOwner { cwd, label, .. } => Some((None, cwd, label)),
                    _ => None,
                };
                if let (Some(tab_id), Some(pane_id), Some((workspace_id, cwd, label))) =
                    (created_tab_id.as_ref(), created_pane_id.as_ref(), creation)
                {
                    self.draw_created_tab(super::drawn_ahead::ProvisionalTab {
                        workspace_id,
                        tab_id: tab_id.clone(),
                        pane_id: pane_id.clone(),
                        cwd: cwd.clone(),
                        label: label.clone(),
                        timing_id: timing_id.clone(),
                        recorded_at_unix_ms: unix_milliseconds(),
                    });
                }
                if let Some(tab_id) = created_tab_id.as_ref() {
                    let placement = match &action {
                        RemoteControlAction::CreateTab {
                            cwd,
                            area_id: Some(area),
                            ..
                        }
                        | RemoteControlAction::OpenOwner {
                            cwd,
                            area_id: Some(area),
                            ..
                        } => Some((cwd.clone(), area.clone())),
                        _ => None,
                    };
                    if let Some((path, area)) = placement
                        && let Some(store) = self.workspace_views.as_mut()
                    {
                        if store.agent_placements.len() < 64 {
                            store.agent_placements.insert(
                                tab_id.clone(),
                                (
                                    (self.node.as_str().to_owned(), path.clone()),
                                    store
                                        .views
                                        .get(self.node.as_str(), &path)
                                        .and_then(|v| v.agent_groups.selected.clone()),
                                    area,
                                    None,
                                ),
                            );
                        } else {
                            self.push_diagnostic(
                                "agent_layout.placement_limit",
                                "Too many pending Agent tab placements",
                            );
                        }
                    }
                }
                let can_show_created = match (&action, created_tab_id.as_deref()) {
                    (
                        RemoteControlAction::CreateTab { cwd, .. }
                        | RemoteControlAction::OpenOwner { cwd, .. },
                        Some(id),
                    ) => self.agent_can_show_created(cwd, id),
                    _ => true,
                };
                // A focus the operator asked for after this creation was sent
                // names where they are now, and the creation's answer is the
                // older request: the new tab waits in the strip rather than
                // taking the screen back from the tab they chose.
                let chosen_since = created_tab_id.is_some()
                    && self
                        .control_lane
                        .running_serial()
                        .is_some_and(|serial| self.last_view_intent_serial > serial);
                if chosen_since && can_show_created {
                    crate::diagnostic!(serde_json::json!({
                        "component": "tab_control",
                        "kind": "tab.create.focus_superseded",
                        "action": action_kind,
                        "created_tab_id": created_tab_id,
                    }));
                    self.push_diagnostic(
                        "tab.create.focus_superseded",
                        "A tab was chosen after this one was created; the created tab waits in the strip",
                    );
                }
                let can_show_created = can_show_created && !chosen_since;
                if can_show_created
                    && matches!(
                        action,
                        RemoteControlAction::CreateTab { .. }
                            | RemoteControlAction::OpenOwner { .. }
                    )
                    && let Some(pane_id) = created_pane_id.as_ref()
                {
                    // tab.create returns the authoritative root pane before
                    // the ordered event projection catches up. Preserve that
                    // focus intent so the next snapshot cannot retain the old
                    // tab merely because its pane still exists.
                    self.snapshot.terminal.pane_id = Some(pane_id.clone());
                    self.snapshot.focused.surface = Surface::Terminal;
                    self.snapshot.focused.pane_id = Some(pane_id.clone());
                    self.snapshot.ui_state.selected_pane_id = Some(pane_id.clone());
                    // The tab was created with focus, so it is Hide's visible
                    // tab from this acknowledgment and Herdr's `tab_focused`
                    // is its confirmation. Without this the strip's active
                    // mark stayed where it was until the operator clicked the
                    // new tab a second time.
                    if let (Some(tab_id), Some(checkout_id)) = (
                        created_tab_id.as_ref(),
                        self.snapshot.navigator.focused_checkout_id.clone(),
                    ) {
                        self.visible_tab_ids
                            .insert(checkout_id.clone(), tab_id.clone());
                        self.send_created_tab(checkout_id, tab_id.clone());
                    }
                    self.yield_surface_to_terminal();
                    self.persist_current_ui_state();
                }
                // Keys typed since the tab was asked for follow it only when
                // it takes the screen, once its shell reads them; a tab that
                // waits in the strip (the operator chose another since) gets
                // none of them.
                if let Some(id) = admission {
                    let origin = super::terminal_input::InputOrigin::TabCreate(id);
                    match created_pane_id.as_deref() {
                        Some(pane_id) if can_show_created => {
                            self.resolve_input_request_at_shell(&origin, pane_id)
                        }
                        _ => self.discard_input_request(&origin, "not_shown"),
                    }
                }
                // Herdr publishes no event for focusing the tab it already
                // shows, so a tab focus that lands where Herdr already was is
                // settled by this answer; waiting for an event would hold
                // every move Herdr makes until the deadline.
                if let RemoteControlAction::FocusTab { tab_id } = &action {
                    self.answer_tab_focus(tab_id);
                }
                self.push_diagnostic(
                    "tab.control.ready",
                    format!(
                        "{action_kind} acknowledged in {elapsed_ms} ms; awaiting authoritative Herdr event"
                    ),
                );
                crate::diagnostic!(serde_json::json!({
                    "component": "tab_control",
                    "kind": "tab.control.ready",
                    "action": action_kind,
                    "created_tab_id": created_tab_id,
                    "created_pane_id": created_pane_id,
                    "duration_ms": elapsed_ms,
                }));
            }
            Err(error)
                if error.is_ambiguous()
                    && matches!(
                        action,
                        RemoteControlAction::CreateTab {
                            admission_id: Some(_),
                            ..
                        } | RemoteControlAction::OpenOwner {
                            admission_id: Some(_),
                            ..
                        }
                    ) =>
            {
                self.push_diagnostic(
                    "tab.create.unknown",
                    format!("{action_kind}: {}", error.message()),
                );
                self.set_error("agent_layout.display_limit",
                    "Tab creation is unconfirmed; its place stays reserved. Close a tab to make room.", false);
            }
            Err(error) => {
                let message = error.message().to_owned();
                // Hide keeps the tab it made visible. The refusal is reported
                // and the wait ends, so the next Herdr event naming another
                // tab is read as an external focus rather than a late answer.
                if let RemoteControlAction::FocusTab { tab_id } = &action {
                    self.refuse_tab_focus(tab_id, &message, !error.is_ambiguous());
                }
                self.set_error(
                    "tab.control.failed",
                    format!("{action_kind} failed: {message}"),
                    true,
                );
                // A failed creation names the checkout it was for (B3).
                let checkout_path = match &action {
                    RemoteControlAction::CreateTab { cwd, .. }
                    | RemoteControlAction::OpenOwner { cwd, .. } => Some(cwd.as_str()),
                    _ => None,
                };
                crate::diagnostic!(serde_json::json!({
                    "component": "tab_control",
                    "kind": "tab.control.failed",
                    "action": action_kind,
                    "checkout_path": checkout_path,
                    "message": message,
                    "duration_ms": elapsed_ms,
                }));
            }
            // `tab.move` is the only action that reports a tab order and it
            // is handled above, before this match.
            Ok(RemoteControlOutcome::TabsOrdered { .. }) => {
                self.set_error(
                    "tab.control.failed",
                    format!("{action_kind} returned a tab order it was not asked for"),
                    false,
                );
            }
        }
        true
    }
    /// Reads what Herdr did with a requested tab move.
    ///
    /// Herdr answers with the workspace's tab list in its new order, so the
    /// answer says whether the move landed as asked without waiting for the
    /// event. An answer that matches leaves the arrangement held until the
    /// order reaches the navigator; anything else drops it and says so, so a
    /// refused or differently-placed move is never a strip that quietly
    /// stayed where it was.
    pub(super) fn ingest_tab_move_result(
        &mut self,
        context: TabMoveResultContext<'_>,
        result: Result<RemoteControlOutcome, live::ControlFailure>,
    ) -> bool {
        let TabMoveResultContext {
            checkout_id,
            tab_id,
            expected_order,
            generation,
            connection_generation,
            elapsed_ms,
        } = context;
        if self
            .pending_tab_move
            .get(checkout_id)
            .is_none_or(|pending| {
                pending.generation != generation
                    || pending.connection_generation != connection_generation
            })
        {
            // The result belongs to an older drag. It is consumed without
            // touching the newer operation or publishing a refusal for it.
            return true;
        }
        let Some(pending) = self.pending_tab_move.get(checkout_id).cloned() else {
            return true;
        };
        if pending.phase != "transmitting" {
            // Once the answer has been accepted, timed out, or settled, the
            // ordered session projection is the only authority left for this
            // request. A duplicate or late answer cannot move the held strip.
            return true;
        }
        if pending
            .deadline_at_unix_ms
            .is_some_and(|deadline| deadline <= unix_milliseconds())
        {
            return self.mark_tab_move_unknown(
                checkout_id,
                generation,
                connection_generation,
                "the response arrived after its deadline".to_owned(),
            );
        }
        let outcome = match result {
            Ok(RemoteControlOutcome::TabsOrdered { tab_ids }) => Ok(tab_ids),
            Ok(RemoteControlOutcome::Acknowledged { .. }) => Err(live::ControlFailure::Ambiguous(
                "tab.move did not report the resulting tab order".to_owned(),
            )),
            Err(error) => Err(error),
        };
        match outcome {
            Ok(tab_ids) => {
                // The response lists the whole workspace, which can hold tabs
                // from sibling checkouts. Only the order of this checkout's
                // tabs was asked for, so only that is checked.
                let placed = tab_ids
                    .iter()
                    .filter(|candidate| expected_order.contains(candidate))
                    .cloned()
                    .collect::<Vec<_>>();
                if placed == expected_order {
                    if let Some(pending) = self.pending_tab_move.get_mut(checkout_id)
                        && pending.generation == generation
                        && pending.connection_generation == connection_generation
                    {
                        pending.phase = "awaiting_topology".to_owned();
                        pending.stage = "topology".to_owned();
                        pending.message = Some(
                            "Request accepted; waiting for the ordered Herdr event".to_owned(),
                        );
                        pending.retryable = false;
                    }
                    self.sync_async_operations();
                    self.push_diagnostic(
                        "tab.move.ready",
                        format!(
                            "Herdr placed tab {tab_id} as asked in {elapsed_ms} ms; awaiting the ordered event"
                        ),
                    );
                    crate::diagnostic!(serde_json::json!({
                        "component": "tab_control",
                        "kind": "tab.move.ready",
                        "checkout_id": checkout_id,
                        "tab_id": tab_id,
                        "order": placed,
                        "duration_ms": elapsed_ms,
                    }));
                    return true;
                }
                crate::diagnostic!(serde_json::json!({
                    "component": "tab_control",
                    "kind": "tab.move.diverged",
                    "checkout_id": checkout_id,
                    "tab_id": tab_id,
                    "requested": expected_order,
                    "placed": placed,
                    "duration_ms": elapsed_ms,
                }));
                self.abandon_tab_move(
                    checkout_id,
                    generation,
                    connection_generation,
                    format!("Herdr put tab {tab_id} somewhere else; the strip follows Herdr"),
                );
                true
            }
            Err(error) if error.is_ambiguous() => {
                let message = error.message().to_owned();
                crate::diagnostic!(serde_json::json!({
                    "component": "tab_control",
                    "kind": "tab.move.unknown",
                    "checkout_id": checkout_id,
                    "tab_id": tab_id,
                    "requested": expected_order,
                    "message": message,
                    "duration_ms": elapsed_ms,
                }));
                self.mark_tab_move_unknown(checkout_id, generation, connection_generation, message);
                true
            }
            Err(error) => {
                let message = error.message().to_owned();
                crate::diagnostic!(serde_json::json!({
                    "component": "tab_control",
                    "kind": "tab.move.failed",
                    "checkout_id": checkout_id,
                    "tab_id": tab_id,
                    "requested": expected_order,
                    "message": message,
                    "duration_ms": elapsed_ms,
                }));
                self.abandon_tab_move(
                    checkout_id,
                    generation,
                    connection_generation,
                    format!("Herdr refused to move tab {tab_id}: {message}"),
                );
                true
            }
        }
    }
    /// Rebuilds the navigator from registrations alone, as test setup; the
    /// product rebuilds it from each session publish and device rows with
    /// `rebuild_device_rows`.
    #[cfg(test)]
    pub(super) fn rebuild_catalog(&mut self) {
        let paths = self.ask_paths(workspace::PathIndex::wanted(
            &self.node,
            &self.snapshot.ui_state.workspace_registrations,
            &self.last_session_spaces,
            &self.worktree_catalog,
        ));
        let mut workspaces = workspace::build_catalog(
            &self.node,
            &self.snapshot.ui_state.workspace_registrations,
            &self.last_session_spaces,
            &self.worktree_catalog,
            &paths,
        );
        self.last_accepted_catalog = Some(workspaces.clone());
        self.catalog_paths = paths;
        Self::apply_workspace_expansion(
            &mut workspaces,
            &self.snapshot.ui_state.collapsed_workspace_ids,
        );
        crate::agent_state::sync_checkout_agent_summaries(
            &mut workspaces,
            &self.snapshot.navigator.agents,
        );
        let created_purpose_suppressions = self.unconfirmed_created_purpose_values();
        suppress_unconfirmed_created_purposes(
            &created_purpose_suppressions,
            &mut workspaces,
            &self.snapshot.navigator.agents,
        );
        self.snapshot.navigator.workspaces = workspaces;
        self.snapshot.navigator.devices = workspace::devices(
            &self.node,
            self.machine_name.as_deref(),
            &self.snapshot.ui_state,
        );
        self.refresh_device_snapshots();
        self.resync_navigator_focus();
    }
    pub(super) fn apply_workspace_expansion(
        workspaces: &mut [crate::model::WorkspaceSnapshot],
        collapsed_workspace_ids: &[String],
    ) {
        let collapsed = collapsed_workspace_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        for workspace in workspaces {
            workspace.expanded = !collapsed.contains(workspace.id.as_str());
        }
    }
    pub(super) fn persist_current_ui_state(&mut self) {
        self.snapshot.ui_state.focused_device_id =
            self.snapshot.navigator.focused_device_id.clone();
        self.snapshot.ui_state.focused_checkout_id =
            self.snapshot.navigator.focused_checkout_id.clone();
        self.persist_ui_state();
    }
    /// Everything one clicked path changes on screen, decided in one event.
    ///
    /// The checkout comes forward, the right panel opens on Explorer, the tree
    /// expands every ancestor and selects the path, and a file also takes an
    /// editor tab. They are one event because the shell's dispatch is
    /// fire-and-forget: sent as separate events the operator would watch the
    /// checkout switch, then the panel appear, then the tree move, and a
    /// refusal partway would leave the screen in a state nobody asked for.
    pub(super) fn reveal_path(&mut self, payload: RevealPathPayload) -> bool {
        match self.reveal_checkout(&payload.workspace_id, &payload.checkout_id) {
            None => {
                self.set_error(
                    "reveal.unknown_checkout",
                    format!(
                        "Checkout {} is not registered, so {} was not revealed",
                        payload.checkout_id, payload.path
                    ),
                    false,
                );
                return true;
            }
            Some(RevealCheckout {
                comes_forward: false,
                device,
                ..
            }) if self.front_checkout()
                != Some((payload.workspace_id.as_str(), payload.checkout_id.as_str())) =>
            {
                self.reveal_when_in_front(device, payload);
                return true;
            }
            Some(_) => {}
        }
        if payload.is_directory {
            self.settle_reveal_path(&payload, None);
            return true;
        }
        if let Some(key) = self.workspace_key(&payload.workspace_id, &payload.checkout_id) {
            self.expect_view_opener(
                &key,
                &payload.path,
                crate::view_layout::DisplayKind::File,
                payload.opener_pane.as_deref(),
            );
        }
        // A revealed file opens into a display, so a Workspace at its display
        // cap refuses it before it is read.
        if self.separate_view_areas()
            && let Some(key) = self.workspace_key(&payload.workspace_id, &payload.checkout_id)
            && !self.admit_view_open(
                &key,
                &payload.path,
                crate::view_layout::DisplayKind::File,
                None,
                false,
                false,
            )
        {
            return true;
        }
        // Nothing moves before the file is read: a reveal settles the whole
        // screen at once, and a file that cannot be read must not leave the
        // checkout focused and the tree expanded around a document that never
        // arrived. The read runs on a worker, so the moves wait for it.
        match self.prepare_file_tab(&payload.workspace_id, &payload.checkout_id, &payload.path) {
            Ok(super::editor::PreparedFileTab::Reading { root, channel }) => {
                let front = self.front_checkout_owned();
                self.start_document_open(
                    root,
                    channel,
                    documents::OpenRequestFields {
                        workspace_id: payload.workspace_id,
                        checkout_id: payload.checkout_id,
                        path: payload.path,
                        preview: false,
                        reload: false,
                        reveal: Some(documents::PendingReveal { front }),
                        restore: false,
                        placement: None,
                    },
                );
            }
            Ok(prepared) => self.settle_reveal_path(&payload, Some(prepared)),
            Err(message) => self.set_error("file.open_failed", message, true),
        }
        true
    }

    /// The checkout a reveal names, wherever the catalog carries it: one the
    /// navigator lists comes forward with the reveal (`focus_checkout`); one
    /// of a device's Herdr session, a node's own checkout among them, is that
    /// session's to bring forward, so the reveal moves no focus of the core's
    /// and one behind waits for that Herdr (`reveal_when_in_front`).
    fn reveal_checkout(&self, workspace_id: &str, checkout_id: &str) -> Option<RevealCheckout> {
        let (workspace, checkout) = self.catalog_checkout(workspace_id, checkout_id)?;
        let comes_forward = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .any(|workspace| workspace.id == workspace_id);
        Some(RevealCheckout {
            path: checkout.path.clone(),
            device: workspace.device_id.clone(),
            comes_forward,
        })
    }

    /// A reveal into a device session's checkout that is not in front there,
    /// as a path clicked on the Overview names: the device's Herdr is asked
    /// to bring the checkout forward, with the device, and the reveal runs
    /// once the sync sees it land (`attach_reveal`), so the click ends where
    /// one on this machine's checkout does. It dies with the request
    /// (`PendingChoice`); without View areas nothing waits, and the reveal is
    /// refused before anything moves.
    fn reveal_when_in_front(&mut self, device: String, payload: RevealPathPayload) {
        if !self.separate_view_areas() {
            self.set_error(
                "reveal.not_in_front",
                format!(
                    "Checkout {} is not in front on its device, so {} was not revealed",
                    payload.checkout_id, payload.path
                ),
                false,
            );
            return;
        }
        let request_id = format!("reveal:{}:{}", unix_milliseconds(), payload.checkout_id);
        let error_before = self.snapshot.status.last_error.clone();
        self.request_remote_control(super::RemoteControlPayload {
            target_id: device.clone(),
            request_id: request_id.clone(),
            report_pane_focus_outcome: false,
            focus_device: true,
            request: super::RemoteControlRequest::FocusWorkspace {
                workspace_id: payload.workspace_id.clone(),
                checkout_id: Some(payload.checkout_id.clone()),
            },
        });
        let checkout = payload.checkout_id.clone();
        if !self.attach_reveal(&device, &request_id, payload)
            && self.snapshot.status.last_error == error_before
        {
            self.push_diagnostic(
                "reveal.not_waiting",
                format!("The reveal into checkout {checkout} on {device} was not kept waiting"),
            );
        }
    }

    /// A revealed file's read landed: the reveal settles now.
    pub(super) fn settle_reveal(
        &mut self,
        workspace_id: &str,
        checkout_id: &str,
        path: &str,
        prepared: Option<super::editor::PreparedFileTab>,
    ) {
        let payload = RevealPathPayload {
            path: path.to_owned(),
            workspace_id: workspace_id.to_owned(),
            checkout_id: checkout_id.to_owned(),
            is_directory: false,
            opener_pane: None,
        };
        self.settle_reveal_path(&payload, prepared);
    }

    fn settle_reveal_path(
        &mut self,
        payload: &RevealPathPayload,
        prepared: Option<super::editor::PreparedFileTab>,
    ) {
        let Some(RevealCheckout {
            path: checkout_path,
            comes_forward,
            ..
        }) = self.reveal_checkout(&payload.workspace_id, &payload.checkout_id)
        else {
            self.set_error(
                "reveal.unknown_checkout",
                format!(
                    "Checkout {} is no longer registered, so {} was not revealed",
                    payload.checkout_id, payload.path
                ),
                false,
            );
            return;
        };
        // A link the operator clicked while this Workspace's files tool
        // showed the tree alone keeps the tree beside the file (B11, B12).
        let lone_tree = self.front_lone_tree().filter(|key| {
            self.workspace_key(&payload.workspace_id, &payload.checkout_id)
                .as_ref()
                == Some(key)
        });
        if comes_forward
            && self.snapshot.navigator.focused_checkout_id.as_deref()
                != Some(payload.checkout_id.as_str())
        {
            self.focus_checkout(&payload.workspace_id, &payload.checkout_id);
        }
        self.snapshot.ui_state.right_panel_visible = true;
        self.snapshot.ui_state.right_panel_section = RightPanelSection::Explorer;
        for expanded in reveal_expansion_paths(&checkout_path, &payload.path, payload.is_directory)
        {
            if !self.snapshot.ui_state.expanded_paths.contains(&expanded) {
                self.snapshot.ui_state.expanded_paths.push(expanded);
            }
        }
        self.snapshot.ui_state.selected_path = Some(payload.path.clone());
        let error_before = self.snapshot.status.last_error.clone();
        if let Some(prepared) = prepared {
            // A revealed path was named on purpose: a terminal link or a
            // Markdown link opens an ordinary tab, not the preview (D-09).
            self.show_file_tab(
                prepared,
                &payload.workspace_id,
                &payload.checkout_id,
                &payload.path,
                false,
            );
        }
        // The reveal calls the lane of its own checkout's Workspace, once it
        // has settled there, not of whichever was in front when it was asked,
        // and not when its file was refused as it landed (PRD right-lane
        // D-13). A file opens in the files tool; a folder shows only in the
        // tree, so it opens the files tool with the tree on (B16).
        if self.separate_view_areas()
            && self.snapshot.status.last_error == error_before
            && let Some(key) = self.workspace_key(&payload.workspace_id, &payload.checkout_id)
        {
            let intent = if payload.is_directory {
                super::workspace_view::AreaIntent::RevealFolder
            } else {
                super::workspace_view::AreaIntent::Files
            };
            self.apply_area_intent_to(&key, intent);
            if lone_tree.as_ref() == Some(&key) {
                self.keep_lone_tree(&key);
            }
        }
        self.push_diagnostic(
            "path.revealed",
            format!(
                "Revealed {} {} in checkout {}",
                if payload.is_directory {
                    "folder"
                } else {
                    "file"
                },
                payload.path,
                payload.checkout_id
            ),
        );
        self.persist_current_ui_state();
    }
    pub(super) fn focus_checkout(&mut self, workspace_id: &str, checkout_id: &str) -> bool {
        // The checkout comes forward on the tab it was showing, with the
        // keyboard on the pane the operator last had there. Its first pane
        // is only for a checkout Hide has never shown.
        let selected_layout = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .find(|workspace| workspace.id == workspace_id)
            .and_then(|workspace| {
                workspace
                    .checkouts
                    .iter()
                    .find(|checkout| checkout.id == checkout_id)
            })
            .and_then(|checkout| {
                self.workspace_views
                    .as_ref()?
                    .views
                    .get(self.node.as_str(), &checkout.path)
            })
            .map(|view| view.selected_agent_layout());
        // An explicit empty collection or draft owns the surface as much as
        // a live tab does. Only a checkout without stored placement falls
        // back to the first topology pane.
        let layout_owned = selected_layout.is_some();
        let visible_tab_id = selected_layout
            .and_then(|layout| layout.active().map(str::to_owned))
            .or_else(|| {
                (!layout_owned)
                    .then(|| self.visible_tab_ids.get(checkout_id).cloned())
                    .flatten()
            });
        let Some((checkout_path, first_pane_id, has_herdr_tab)) = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .find(|workspace| workspace.id == workspace_id)
            .and_then(|workspace| {
                workspace
                    .checkouts
                    .iter()
                    .find(|checkout| checkout.id == checkout_id)
                    .map(|checkout| {
                        let visible_tab = visible_tab_id.as_deref().and_then(|tab_id| {
                            checkout
                                .tabs
                                .iter()
                                .find(|tab| tab.id.as_deref() == Some(tab_id))
                        });
                        let first_pane_id = visible_tab
                            .and_then(|tab| tab.panes.first())
                            .or_else(|| {
                                (!layout_owned)
                                    .then(|| {
                                        checkout.tabs.iter().flat_map(|tab| tab.panes.iter()).next()
                                    })
                                    .flatten()
                            })
                            .map(|pane| pane.id.clone());
                        (
                            checkout.path.clone(),
                            first_pane_id,
                            !checkout.tabs.is_empty(),
                        )
                    })
            })
        else {
            let workspace_exists = self
                .snapshot
                .navigator
                .workspaces
                .iter()
                .any(|workspace| workspace.id == workspace_id);
            let (kind, message) = if workspace_exists {
                (
                    "checkout.unknown",
                    format!("Checkout {checkout_id} is not available in {workspace_id}"),
                )
            } else {
                (
                    "checkout.unknown_workspace",
                    format!("Workspace {workspace_id} is not registered"),
                )
            };
            self.set_error(kind, message, false);
            return true;
        };
        let next_pane_id = match visible_tab_id.as_deref() {
            Some(tab_id) => self.tab_focus_pane_id(tab_id, first_pane_id),
            None => first_pane_id,
        };
        self.snapshot.navigator.focused_workspace_id = Some(workspace_id.to_owned());
        self.snapshot.navigator.focused_checkout_id = Some(checkout_id.to_owned());
        self.snapshot.navigator.root_path = Some(checkout_path);
        self.sync_changes_root_path();
        self.refresh_inactive_groups();
        // Selecting a checkout measures it, and selecting the one already
        // selected measures it again - that is the card's cheapest refresh
        // for a number that moves whenever a build runs (R8).
        self.remeasure_disk();
        self.reset_terminal_projection(next_pane_id.clone());
        self.sync_active_tab_projection();
        self.align_visible_tab_with_selected_pane();
        // The operator chose a checkout, not a pane: the record stops
        // following the pane just left, and nothing is raised for the pane
        // the checkout came forward on. A sidebar row click dispatches this
        // and then a pane focus, and raising the remembered pane here for
        // that one frame cleared a question nobody had read.
        self.operator_focused_pane_id = None;
        self.refresh_pane_read_state();
        self.yield_surface_to_terminal();
        // With separate View areas the alignment above already chose the
        // Workspace's remembered View tab; only the shared canvas needs a
        // document to show in place of a missing terminal.
        if !has_herdr_tab
            && !self.separate_view_areas()
            && let Some(file_tab_id) = self
                .snapshot
                .editor
                .tabs
                .iter()
                .rev()
                .find(|tab| tab.workspace_id == workspace_id && tab.checkout_id == checkout_id)
                .map(|tab| tab.id.clone())
            && let Err(message) = self.activate_editor_tab(&file_tab_id)
        {
            self.set_error("file.focus_failed", message, false);
        }
        self.persist_current_ui_state();
        if self.snapshot.ui_state.right_panel_visible
            && matches!(
                self.snapshot.ui_state.right_panel_section,
                RightPanelSection::Sessions
            )
        {
            self.request_sessions_refresh();
        }
        if let (Some(context), Some(pane_id)) = (self.live.as_ref().cloned(), next_pane_id)
            && let Err(message) =
                live::spawn_pane_control(context, PaneControlAction::Project { pane_id })
        {
            self.set_error("pane.projection_worker_failed", message, true);
        }
        true
    }
    pub fn ingest_workspace_creation(
        &mut self,
        request_path: &str,
        result: Result<live::WorkspaceCreationOutcome, String>,
        elapsed_ms: u128,
    ) -> bool {
        self.workspace_creations_in_flight.remove(request_path);
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(message) => {
                self.set_error("workspace.create_failed", message, false);
                return true;
            }
        };
        // A path the front-door check could not match (a symlink, a
        // different spelling) can still land on an id whose removal is
        // closing panes. The worker has already opened this project's first
        // pane, so the removal is the request that gives way: dropping it
        // from the in-flight set makes the close's late answer authorize
        // nothing, and the banner says which request won.
        if self
            .workspace_removals_in_flight
            .remove(&outcome.registration.id)
        {
            self.set_error(
                "workspace.remove_cancelled",
                format!(
                    "{} was added again while its removal was closing panes; the project stays registered",
                    outcome.registration.label
                ),
                false,
            );
        }
        let catalog_inputs_match =
            self.snapshot.ui_state.workspace_registrations == outcome.base_registrations;
        if catalog_inputs_match {
            self.snapshot.ui_state.workspace_registrations = outcome.registrations;
        } else if !self
            .snapshot
            .ui_state
            .workspace_registrations
            .iter()
            .any(|registration| registration.id == outcome.registration.id)
        {
            self.snapshot
                .ui_state
                .workspace_registrations
                .push(outcome.registration.clone());
            self.push_diagnostic(
                "workspace.catalog.refresh_pending",
                "Workspace registrations changed during creation; session sync will refresh the catalog",
            );
        }
        self.snapshot.navigator.focused_device_id = Some(self.node.as_str().to_owned());
        self.reconcile_remote_terminal_selection();
        let target_checkout = outcome
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.checkouts.iter())
            .find(|checkout| checkout.path == outcome.registration.path);
        if let Some(checkout) = target_checkout {
            self.snapshot.navigator.focused_checkout_id = Some(checkout.id.clone());
            self.snapshot.ui_state.focused_checkout_id = Some(checkout.id.clone());
        }
        let target_pane_id = outcome.created_pane_id.clone().or_else(|| {
            target_checkout
                .and_then(|checkout| checkout.tabs.first())
                .and_then(|tab| tab.panes.first())
                .map(|pane| pane.id.clone())
        });
        if catalog_inputs_match {
            self.reset_terminal_projection(target_pane_id);
            self.ingest_session_with_catalog(
                Ok(outcome.session),
                Some(session_sync::PrecomputedCatalog {
                    registrations: self.snapshot.ui_state.workspace_registrations.clone(),
                    worktrees: self.worktree_catalog.clone(),
                    workspaces: outcome.workspaces,
                    paths: outcome.paths,
                }),
            );
        } else {
            self.push_diagnostic(
                "workspace.session.refresh_pending",
                "Workspace creation completed after registrations changed; session sync will publish the authoritative catalog",
            );
        }
        self.persist_current_ui_state();
        let git_init_failed = outcome.git_init_error.is_some();
        if let Some(message) = outcome.git_init_error.as_ref() {
            self.set_error(
                "workspace.git_init_failed",
                format!("Workspace was registered, but Git initialization failed: {message}"),
                true,
            );
        }
        self.push_diagnostic(
            "workspace.registered",
            format!(
                "Registered workspace {} in {elapsed_ms} ms",
                outcome.registration.path
            ),
        );
        crate::diagnostic!(serde_json::json!({
            "component": "workspace",
            "kind": "workspace.registered",
            "path": outcome.registration.path,
            "duration_ms": elapsed_ms,
                "git_init": if git_init_failed { "failed" } else { "complete_or_skipped" },
        }));
        true
    }
}

impl Runtime {
    /// Remembers which checkout a created tab was asked for. When the session
    /// already placed the tab under another checkout, the local coordinator is
    /// asked to republish, because nothing else would place it again until an
    /// unrelated change.
    pub(super) fn record_created_tab_checkout(&mut self, tab_id: &str, path: &str) {
        // The same tab acknowledged twice is one record: a second one would
        // start its window again and read an entered folder as a birth value.
        if self.created_tab_checkouts.contains_key(tab_id) {
            return;
        }
        if self.created_tab_checkouts.len() >= CREATED_TAB_CHECKOUT_LIMIT
            && !self.created_tab_checkouts.contains_key(tab_id)
        {
            crate::diagnostic!(serde_json::json!({
                "component": "session",
                "kind": "created_tab.checkout_limit",
                "tab_id": tab_id,
                "checkout_path": path,
                "limit": CREATED_TAB_CHECKOUT_LIMIT,
            }));
            return;
        }
        self.created_tab_checkouts.insert(
            tab_id.to_owned(),
            CreatedTabCheckout {
                path: path.to_owned(),
                seen: false,
                settled: false,
                recorded_at: (self.birth_clock)(),
            },
        );
        let misplaced = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .flat_map(|workspace| &workspace.checkouts)
            .any(|checkout| {
                checkout.path != path
                    && checkout
                        .tabs
                        .iter()
                        .any(|tab| tab.id.as_deref() == Some(tab_id))
            });
        // The session placed the tab under another checkout before this
        // acknowledgment, so it is read again now to move it.
        if misplaced {
            self.request_republish();
        }
    }

    /// Notes which created tabs report a cwd inside their checkout, read off
    /// the session as Herdr sent it: from then on the cwd is where the shell
    /// is, not where it was born.
    /// Returns the clamps of the tabs still on a birth value.
    fn settle_created_tabs(
        &mut self,
        payload: &SessionSnapshotPayload,
        clamps: Vec<CreatedTabClamp>,
        paths: &workspace::PathIndex,
    ) -> Vec<CreatedTabClamp> {
        let now = (self.birth_clock)();
        clamps
            .into_iter()
            .filter(|clamp| {
                let Some(created) = self.created_tab_checkouts.get_mut(&clamp.tab_id) else {
                    return false;
                };
                let mut carried = false;
                let mut inside = false;
                for pane in payload
                    .layouts
                    .iter()
                    .filter(|layout| layout.tab_id == clamp.tab_id)
                    .flat_map(|layout| &layout.panes)
                {
                    carried = true;
                    if let Some(cwd) = Self::pane_cwd(payload, &pane.pane_id) {
                        inside |= clamp.holds(&cwd, paths);
                    }
                }
                created.settled =
                    inside || (carried && created.birth_window_over(now, &clamp.tab_id));
                !created.settled
            })
            .collect()
    }

    /// The created tabs whose pane cwd is still a birth value, with the
    /// checkout each was created for.
    pub(crate) fn created_tab_clamps(&self) -> Vec<CreatedTabClamp> {
        self.created_tab_checkouts
            .iter()
            .filter(|(_, created)| !created.settled)
            .map(|(tab_id, created)| CreatedTabClamp {
                tab_id: tab_id.clone(),
                path: created.path.clone(),
                rules: PathRules::Local,
                repositories: Arc::clone(&self.local_worktree_paths),
            })
            .collect()
    }

    /// Reads a created tab's panes as inside the checkout the tab was created
    /// for until their cwd has settled there. Every reader of the session sees
    /// the intended folder: the project and checkout rows, the purpose mirror,
    /// the pane's cwd and the ports attributed by it. A birth cwd outside the
    /// checkout would otherwise make a project row for the parent folder and
    /// claim the listeners of its siblings (#400).
    pub(crate) fn clamp_created_tab_cwds(
        payload: &mut SessionSnapshotPayload,
        clamps: &[CreatedTabClamp],
        paths: &workspace::PathIndex,
    ) {
        for clamp in clamps {
            let pane_ids = payload
                .layouts
                .iter()
                .filter(|layout| layout.tab_id == clamp.tab_id)
                .flat_map(|layout| layout.panes.iter().map(|pane| pane.pane_id.clone()))
                .collect::<Vec<_>>();
            for pane_id in pane_ids {
                let outside = |cwd: &Option<String>| {
                    cwd.as_deref().is_none_or(|cwd| !clamp.holds(cwd, paths))
                };
                for pane in payload
                    .panes
                    .iter_mut()
                    .filter(|pane| pane.pane_id == pane_id)
                {
                    if outside(&pane.cwd) {
                        pane.cwd = Some(clamp.path.clone());
                    }
                }
                for agent in payload.agents.iter_mut().filter(|agent| {
                    agent.pane_id.as_deref().or(agent.id.as_deref()) == Some(pane_id.as_str())
                }) {
                    if outside(&agent.cwd) {
                        agent.cwd = Some(clamp.path.clone());
                    }
                }
            }
        }
    }

    /// Remembers which folder a tab created on a device was asked for, for
    /// the same reason as `record_created_tab_checkout`: the device's helper
    /// groups a tab by its first pane's cwd, which is a birth value. When the
    /// device's session already carried the tab, the session is read again
    /// now, because nothing else would regroup it before the device's next
    /// publish.
    pub(super) fn record_created_device_tab(&mut self, target_id: &str, tab_id: &str, path: &str) {
        let key = (target_id.to_owned(), tab_id.to_owned());
        // See `record_created_tab_checkout`: one record per tab.
        if self.created_device_tabs.contains_key(&key) {
            return;
        }
        if self.created_device_tabs.len() >= CREATED_TAB_CHECKOUT_LIMIT
            && !self.created_device_tabs.contains_key(&key)
        {
            crate::diagnostic!(serde_json::json!({
                "component": "session",
                "kind": "created_tab.checkout_limit",
                "target": target_id,
                "tab_id": tab_id,
                "checkout_path": path,
                "limit": CREATED_TAB_CHECKOUT_LIMIT,
            }));
            return;
        }
        self.created_device_tabs.insert(
            key,
            CreatedTabCheckout {
                path: path.to_owned(),
                seen: false,
                settled: false,
                recorded_at: (self.birth_clock)(),
            },
        );
        // The stored session was clamped for the records that existed when it
        // arrived, so the other tabs' panes already read as their folder and
        // would look settled; only the new record reads it.
        if let Some(mut raw) = self.device_raw_sessions.remove(target_id) {
            self.clamp_device_created_tabs(target_id, &mut raw, Some(tab_id));
            self.device_raw_sessions.insert(target_id.to_owned(), raw);
            self.refresh_device_catalog(target_id);
        }
    }

    /// The worktrees of each repository a device's helper listed, as the
    /// helper spelled them.
    fn device_worktree_paths(&self, target_id: &str) -> Arc<Vec<Vec<String>>> {
        let paths = self
            .device_worktrees
            .get(target_id)
            .into_iter()
            .flat_map(|listed| listed.projects.values())
            .map(|project| {
                project
                    .worktrees
                    .iter()
                    .map(|worktree| worktree.path.clone())
                    .collect()
            })
            .collect();
        Arc::new(paths)
    }

    /// Reads a device tab Hide created as in the folder it was created for
    /// until a pane of it reports a cwd there, before the device's projects
    /// are grouped, and drops the record of a tab the device no longer has.
    ///
    /// `only` limits the clamp to one tab's record.
    fn clamp_device_created_tabs(
        &mut self,
        target_id: &str,
        raw: &mut RemoteSessionSnapshot,
        only: Option<&str>,
    ) {
        self.created_device_tabs
            .retain(|(target, tab_id), created| {
                if target != target_id {
                    return true;
                }
                let present = raw.workspaces.iter().any(|workspace| {
                    workspace.checkouts.iter().any(|checkout| {
                        checkout.tabs.iter().any(|tab| {
                            tab.id
                                .as_deref()
                                .and_then(|id| remote_tab_source_id(target_id, id))
                                == Some(tab_id.as_str())
                        })
                    })
                });
                created.seen |= present;
                present || !created.seen
            });
        let now = (self.birth_clock)();
        let repositories = self.device_worktree_paths(target_id);
        let clamps = self
            .created_device_tabs
            .iter()
            .filter(|((target, tab_id), created)| {
                target == target_id && !created.settled && only.is_none_or(|only| only == tab_id)
            })
            .map(|((_, tab_id), created)| CreatedTabClamp {
                tab_id: tab_id.clone(),
                path: created.path.clone(),
                rules: PathRules::Device,
                repositories: Arc::clone(&repositories),
            })
            .map(|clamp| (clamp.tab_id.clone(), clamp))
            .collect::<BTreeMap<_, _>>();
        for workspace in &mut raw.workspaces {
            for checkout in &mut workspace.checkouts {
                for tab in &mut checkout.tabs {
                    let Some(native) = tab
                        .id
                        .as_deref()
                        .and_then(|id| remote_tab_source_id(target_id, id))
                    else {
                        continue;
                    };
                    let Some(clamp) = clamps.get(native) else {
                        continue;
                    };
                    let Some(created) = self
                        .created_device_tabs
                        .get_mut(&(target_id.to_owned(), native.to_owned()))
                    else {
                        continue;
                    };
                    created.settled = tab
                        .panes
                        .iter()
                        .any(|pane| clamp.holds(&pane.cwd, &workspace::PathIndex::NONE))
                        || (!tab.panes.is_empty() && created.birth_window_over(now, native));
                    if created.settled {
                        continue;
                    }
                    for pane in &mut tab.panes {
                        if !clamp.holds(&pane.cwd, &workspace::PathIndex::NONE) {
                            pane.cwd = clamp.path.clone();
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn set_republish_waker(&mut self, waker: session_sync::RepublishWaker) {
        self.republish_waker = Some(waker);
    }

    /// Asks the local coordinator to read the session again now: the core
    /// drew a created tab or a predicted pane geometry ahead of Herdr, or
    /// took one back, and the overlay rides the next ingest.
    pub(super) fn request_republish(&mut self) {
        #[cfg(test)]
        {
            self.republish_requested = true;
        }
        if let Some(waker) = &self.republish_waker {
            waker.wake();
        }
    }

    #[cfg(test)]
    pub(crate) fn take_republish_request(&mut self) -> bool {
        std::mem::take(&mut self.republish_requested)
    }
}
