use super::*;
use crate::model::GithubReadOutcome;

const MINIMUM_PURPOSE_HERDR_VERSION: (u64, u64, u64) = (0, 9, 1);

/// How long a project's pull requests are trusted before the core asks again.
const GITHUB_REREAD: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// How long a project waits after its first failed read; each further failure
/// in a row doubles it, up to `GITHUB_REREAD`.
const GITHUB_RETRY_FIRST: std::time::Duration = std::time::Duration::from_secs(30);

/// How old a good answer must be before the operator looking at its project
/// asks for another (`read_when_looked_at`): a burst of window focuses inside
/// it adds no read.
const GITHUB_LOOK_AFTER: std::time::Duration = std::time::Duration::from_secs(30);

/// A project's place in the re-read cycle. The wait runs from the answer, not
/// from the ask: counted from the ask, a pass over many projects that
/// outlasts it would be overtaken by its own next ask and never land.
///
/// `failures` is how many reads in a row ended without the project's pull
/// requests or its issues (`GithubReadOutcome`), so a read that failed is
/// asked again sooner than one that worked and a success starts the count
/// over.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GithubClock {
    /// A read is asked for and has not answered.
    Waiting { failures: u32 },
    /// The read answered at this instant; `pull_requests_read` is whether
    /// it held the pull requests, which a sighted address is read against.
    Answered {
        at: std::time::Instant,
        failures: u32,
        pull_requests_read: bool,
    },
    /// The read answered that the repository has no GitHub remote, and the
    /// catalog knows its remotes. Nothing asks again on the clock: only a
    /// change in the repository's remotes (`PathIndex::remotes`) or an
    /// explicit refresh does.
    NoRemote,
}

impl GithubClock {
    fn failures(self) -> u32 {
        match self {
            Self::Waiting { failures } | Self::Answered { failures, .. } => failures,
            Self::NoRemote => 0,
        }
    }
}

/// The wait after an answer that came after `failures` failed reads in a row:
/// the full re-read for a success, else 30 s doubling per failure and stopping
/// at the full re-read.
fn github_wait(failures: u32) -> std::time::Duration {
    match failures {
        0 => GITHUB_REREAD,
        failures => GITHUB_RETRY_FIRST
            .saturating_mul(1u32.checked_shl(failures - 1).unwrap_or(u32::MAX))
            .min(GITHUB_REREAD),
    }
}

/// The most sighted addresses remembered at once as having set off their read;
/// one past it sets off nothing and is a reported shortfall.
const SIGHTED_ASKED_LIMIT: usize = 64;

/// A pull request address a session printed that set off a read, or waits to
/// (`read_sighted_pull_requests`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Sighted {
    /// Until when it is recent enough to read for, on the clock the re-read
    /// runs on.
    fresh_until: std::time::Instant,
    /// The projects whose ask was unanswered when it was printed, each owed a
    /// read once that answer lands if the answer does not hold it.
    waiting: BTreeSet<String>,
    /// The projects whose read of it has not landed: the turn that printed
    /// it is held in progress until each answers (status-marks D-09).
    owed: BTreeSet<String>,
}

/// What a sighting or a look may do to one project now.
enum SightedRead {
    /// The project's last read answered and worked, and nothing has asked
    /// since: read it again now.
    Now,
    /// An ask is outstanding (a read in flight, the first not yet answered,
    /// or an answer the clock has not taken yet): asking again would overtake
    /// it, so wait for its answer.
    AfterAnswer,
    /// The last read failed; the project's own retry comes within five
    /// minutes and reads the pull request then.
    Never,
}

/// What a look at a project did (`read_when_looked_at`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LookRead {
    /// The project was asked for again.
    Asked,
    /// An ask is outstanding or the answer is too young; it would be asked
    /// once that passes.
    Later,
    /// Nothing for a look to add: the project is not read, its repository has
    /// no GitHub remote, or its last read failed and its own retry is coming.
    Never,
}

/// The most local Git projects the core reads GitHub for. Every registered one
/// is read, and each costs `gh` calls every `GITHUB_REREAD`; a count past this
/// is a reported shortfall, not a larger number.
const GITHUB_PROJECT_LIMIT: usize = 64;

/// How long a merge Hide saw succeed reads as merged without GitHub's list
/// saying so.
const MERGED_HERE_LIFETIME_MS: u64 = 10 * 60 * 1000;

fn herdr_version_supports_purpose(version: Option<&str>) -> bool {
    parse_herdr_version(version.unwrap_or_default())
        .is_some_and(|version| version >= MINIMUM_PURPOSE_HERDR_VERSION)
}

fn parse_herdr_version(version: &str) -> Option<(u64, u64, u64)> {
    let version = version.strip_prefix('v').unwrap_or(version);
    let numeric = version.split(['-', '+']).next()?;
    let mut parts = numeric.split('.');
    let parsed = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    parts.next().is_none().then_some(parsed)
}

fn remote_purpose_unavailable_reason(version: Option<&str>) -> String {
    match version {
        Some(version) => format!(
            "Set purpose requires Herdr 0.9.1 or newer on the remote device; {version} is installed"
        ),
        None => "Set purpose requires Herdr 0.9.1 or newer on the remote device; its version is unavailable"
            .to_owned(),
    }
}

/// Puts the measured sizes on a repository's worktrees and its shared Git
/// directory, matched by path, and sums them for its facts line. A device's
/// repositories take their own device's rows only, so a path that names
/// another machine's folder never matches.
pub(super) fn measure_worktrees(
    project: &mut crate::model::ProjectWorktreesSnapshot,
    disk_usage: &[crate::model::DiskUsageSnapshot],
) {
    for worktree in &mut project.worktrees {
        worktree.disk = disk_usage
            .iter()
            .find(|disk| disk.path.as_deref() == Some(worktree.path.as_str()))
            .cloned()
            .unwrap_or_default();
    }
    project.shared_git_disk = disk_usage
        .iter()
        .find(|d| d.path.is_some() && d.path == project.shared_git_path)
        .cloned()
        .unwrap_or_default();
    let components: Vec<_> = project
        .worktrees
        .iter()
        .map(|w| &w.disk)
        .chain(std::iter::once(&project.shared_git_disk))
        .collect();
    project.disk_total_bytes = project
        .shared_git_path
        .as_ref()
        .and_then(|_| components.iter().map(|d| d.total_bytes).sum());
    let confirmed: Vec<_> = components.iter().filter_map(|d| d.total_bytes).collect();
    project.disk_confirmed_bytes = (!confirmed.is_empty()).then(|| confirmed.iter().sum());
    project.linked_disk_bytes = project
        .worktrees
        .iter()
        .filter(|w| !w.is_main)
        .map(|w| w.disk.total_bytes)
        .sum();
    project.disk_unavailable_reason = components
        .iter()
        .find_map(|d| d.unavailable_reason.clone())
        .or_else(|| {
            project
                .shared_git_path
                .is_none()
                .then(|| "Shared Git directory is unavailable. Refresh Overview.".into())
        });
}

/// A project's size as its facts line reads it: the catalog's sum once every
/// part is measured, the reason when a part could not be, and `measuring`
/// while the project is the one named for measuring and neither has come
/// back yet.
pub(super) fn project_disk(
    project: Option<&crate::model::ProjectWorktreesSnapshot>,
    (measuring, queued): (bool, bool),
) -> crate::model::ProjectDiskSnapshot {
    let total_bytes = project.and_then(|project| project.disk_total_bytes);
    let unavailable_reason = project.and_then(|project| project.disk_unavailable_reason.clone());
    let free_bytes = project.and_then(|project| {
        project
            .worktrees
            .iter()
            .map(|worktree| &worktree.disk)
            .chain(std::iter::once(&project.shared_git_disk))
            .find_map(|disk| disk.volume_free_bytes)
    });
    let volume_total_bytes = project.and_then(|project| {
        project
            .worktrees
            .iter()
            .map(|worktree| &worktree.disk)
            .chain(std::iter::once(&project.shared_git_disk))
            .find_map(|disk| disk.volume_total_bytes)
    });
    // The layers of every checkout that was measured: one that could not be
    // read leaves the subtotal, not the whole breakdown.
    let layers = project.and_then(|project| {
        let mut sum = crate::model::ProjectDiskLayersSnapshot {
            shared_git: project.shared_git_disk.total_bytes.unwrap_or(0),
            ..Default::default()
        };
        let mut any = false;
        for layers in project
            .worktrees
            .iter()
            .filter_map(|worktree| worktree.disk.layers.as_ref())
        {
            any = true;
            sum.build_cache += layers.build_cache.bytes;
            sum.dependencies += layers.dependencies.bytes;
            sum.other += layers.other.bytes;
            sum.source += layers.source_bytes;
        }
        any.then_some(sum)
    });
    crate::model::ProjectDiskSnapshot {
        measuring,
        queued,
        total_bytes,
        unavailable_reason,
        free_bytes,
        volume_total_bytes,
        confirmed_bytes: project.and_then(|project| project.disk_confirmed_bytes),
        layers,
    }
}

impl Runtime {
    /// What the changes view needs read, or `None` while nothing on screen
    /// shows it. The checkout in front may be on this machine or on a device;
    /// either is read by its own host. The diffs the front Workspace's View
    /// areas show ride the same read (PRD S7 A5).
    pub fn changes_request(&mut self) -> Option<crate::changes::ChangesRequest> {
        let (workspace_id, checkout_id, root_path) = self.changes_target()?;
        let root = self.document_root(&workspace_id, &checkout_id).ok()?;
        let channel = self
            .changes_channel(&root.device_id)
            .map(crate::changes::ChannelRef);
        let (selected_path, selected_committed) = self.changes_selection();
        // The base comes from the checkout row, so the committed group and
        // the card's `↑A ↓B` are measured against the same branch; a device
        // checkout has none, and its host uses the repository default.
        let base_branch = self
            .catalog_checkout(&workspace_id, &checkout_id)
            .and_then(|(_, checkout)| checkout.base_branch.clone());
        Some(crate::changes::ChangesRequest {
            root,
            channel,
            root_path,
            selected_path,
            selected_committed,
            base_branch,
            diffs: self.visible_view_diffs(),
        })
    }

    /// The row whose diff the next read takes, and its group: the diff in
    /// front, otherwise the row History has selected.
    fn changes_selection(&self) -> (Option<String>, bool) {
        let active_diff = self.active_diff_tab();
        let selected_path = active_diff
            .map(|tab| tab.path.clone())
            .or_else(|| self.snapshot.changes.selected_path.clone());
        let selected_committed = active_diff
            .and_then(|tab| tab.diff_committed)
            .unwrap_or(self.snapshot.changes.selected_committed);
        (selected_path, selected_committed)
    }

    /// The device and folder the changes view is about now, the fence every
    /// answer passes.
    pub(crate) fn changes_key(&self) -> Option<crate::changes::ChangesKey> {
        self.changes_target()?;
        self.front_changes_key()
    }

    /// The device and folder History describes for the checkout in front,
    /// whether or not anything shows it now.
    fn front_changes_key(&self) -> Option<crate::changes::ChangesKey> {
        let (workspace_id, checkout_id) = self.front_checkout()?;
        let (workspace, _) = self.catalog_checkout(workspace_id, checkout_id)?;
        Some(crate::changes::ChangesKey {
            device_id: workspace.device_id.clone(),
            root_path: self.focused_changes_root_path()?,
        })
    }

    fn active_diff_tab(&self) -> Option<&EditorTabSnapshot> {
        self.snapshot
            .editor
            .active_tab_id
            .as_deref()
            .and_then(|tab_id| {
                self.snapshot
                    .editor
                    .tabs
                    .iter()
                    .find(|tab| tab.id == tab_id && tab.kind == EditorTabKind::Diff)
            })
    }

    fn changes_target(&self) -> Option<(String, String, String)> {
        // With no renderer attached nothing draws History, the Explorer's
        // Git marks or a diff; the selection the operator left stays in the
        // UI state, so the first renderer back reads the same checkout again.
        if !self.ui_attached {
            return None;
        }
        let section_visible = |section| {
            self.snapshot.ui_state.right_panel_visible
                && self.snapshot.ui_state.right_panel_section == section
        };
        if !section_visible(RightPanelSection::Changes)
            && !section_visible(RightPanelSection::Explorer)
            && self.active_diff_tab().is_none()
            && self.visible_view_diffs().is_empty()
            && !self.new_tab_visible()
        {
            return None;
        }
        let (workspace_id, checkout_id) = self.front_checkout_owned()?;
        let root_path = self.focused_changes_root_path()?;
        Some((workspace_id, checkout_id, root_path))
    }

    /// The host the changes view reads through. A device's helper is asked
    /// for once when the view first needs it; a helper that failed is not
    /// asked again on every refresh, and the view says why until the
    /// operator's next action retries it.
    pub(super) fn changes_channel(
        &mut self,
        device_id: &str,
    ) -> Result<Arc<dyn crate::node_access::NodeLink>, String> {
        if device_id != self.node.as_str() && !self.device_hosts.contains_key(device_id) {
            self.start_device_host(device_id);
        }
        match self.device_hosts.get(device_id).map(|host| &host.phase) {
            _ if device_id == self.node.as_str() => Ok(Arc::clone(&self.own_node)),
            Some(hosts::HostPhase::Ready { host, .. }) if host.closed_reason().is_none() => {
                Ok(Arc::clone(host))
            }
            _ => Err(self
                .host_snapshot(device_id)
                .message
                .unwrap_or_else(|| "The device helper is not ready".to_owned())),
        }
    }

    /// A registration can name a folder inside its Git checkout. The
    /// navigator workspace path may have been projected as the repository
    /// root after Herdr occupies it, so use the registration's own path.
    /// Linked worktree rows use their own checkout root.
    pub(super) fn focused_changes_root_path(&self) -> Option<String> {
        let (workspace_id, checkout_id) = self.front_checkout()?;
        let (workspace, checkout) = self.catalog_checkout(workspace_id, checkout_id)?;
        if workspace.registered
            && let Some(registered) = self
                .snapshot
                .ui_state
                .workspace_registrations
                .iter()
                .find(|registration| registration.id == workspace.id)
            && hide_platform::path::wire_relative(&checkout.path, &registered.path).is_ok()
        {
            return Some(registered.path.clone());
        }
        Some(checkout.path.clone())
    }

    /// Keep the History identity in the same snapshot frame as checkout focus.
    /// All focus routes call this after assigning the focused workspace and
    /// checkout; remote checkouts clear the identity immediately.
    pub(super) fn sync_changes_root_path(&mut self) {
        self.snapshot.navigator.changes_root_path = self.focused_changes_root_path();
        // The same folder on another device is another checkout: a projection
        // read on the one left behind is dropped in this same frame rather
        // than shown under the new device until the next read lands (B22).
        let front = self.front_changes_key();
        if self.changes_published_key.is_some() && self.changes_published_key != front {
            self.snapshot
                .changes
                .set(crate::model::ChangesSnapshot::default());
            self.changes_published_key = None;
        }
    }

    pub fn worktrees_request(&self) -> crate::worktrees::WorktreeRequest {
        let projects = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .filter(|workspace| workspace.remote_target_id.is_none() && workspace.is_git)
            .map(|workspace| crate::worktrees::WorktreeProjectRequest {
                root_path: PathBuf::from(&workspace.path),
                generation: self
                    .worktree_project_generations
                    .get(&workspace.path)
                    .copied()
                    .unwrap_or(0),
                base_override: crate::project_settings::base_choice(
                    &self.snapshot.ui_state.project_settings,
                    &self.snapshot.ui_state.project_base_branches,
                    self.node.as_str(),
                    &workspace.path,
                )
                .map(str::to_owned),
                remote_default: self
                    .github
                    .project(&workspace.path)
                    .and_then(|project| project.issues.default_branch.clone()),
                bases: self
                    .github
                    .project(&workspace.path)
                    .map(|project| {
                        crate::github::preferred_per_branch(&project.pull_requests)
                            .into_iter()
                            .map(|pull_request| {
                                (
                                    pull_request.head_branch.clone(),
                                    pull_request.base_branch.clone(),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .collect();
        crate::worktrees::WorktreeRequest {
            projects,
            generation: self.worktree_generation,
            removals: self.worktree_removals,
        }
    }

    /// Stores a catalog read that started when `removals` local removals had
    /// settled. A checkout a later removal dropped stays dropped: the read
    /// ran before Git forgot it, and only a read that started after the
    /// removal is news about that path.
    pub fn ingest_worktrees(
        &mut self,
        catalog: crate::model::WorktreeCatalogSnapshot,
        removals: u64,
    ) -> bool {
        self.ingest_worktrees_answer(catalog, removals, true)
    }

    pub(crate) fn ingest_worktrees_answer(
        &mut self,
        mut catalog: crate::model::WorktreeCatalogSnapshot,
        removals: u64,
        current: bool,
    ) -> bool {
        // Keep the visible values on an Overview refresh. An initial read has
        // no values to preserve, so accept its slightly old answer and let the
        // already queued current read correct it; otherwise a moving request
        // can starve the first catalog entirely.
        if !current && !self.worktree_catalog.projects.is_empty() {
            return false;
        }
        self.removed_worktrees
            .retain(|(settled, _)| *settled > removals);
        for project in &mut catalog.projects {
            project.worktrees.retain(|worktree| {
                !self
                    .removed_worktrees
                    .iter()
                    .any(|(_, path)| *path == worktree.path)
            });
        }
        let loading = self.snapshot.git_worktrees_loading && !current;
        let changed =
            self.worktree_catalog != catalog || self.snapshot.git_worktrees_loading != loading;
        self.local_worktree_paths = Arc::new(
            catalog
                .projects
                .iter()
                .map(|project| {
                    project
                        .worktrees
                        .iter()
                        .map(|worktree| worktree.path.clone())
                        .collect()
                })
                .collect(),
        );
        self.worktree_catalog = catalog;
        self.snapshot.git_worktrees_loading = loading;
        // A project the Projects tab watched before Git listed it is measured now.
        self.advance_disk_sweep(None);
        self.refresh_worktree_projection();
        // A HEAD the reader moved decides again which pull request a checkout
        // holds, and what hangs off that (its issues, its request rows).
        self.apply_pull_requests();
        self.refresh_worktree_projection();
        // A local merge can bring a mission's outside branch into main (B19).
        let mut followed = false;
        if changed {
            let device = self.node.as_str().to_owned();
            for project in self.outside_projects(&device) {
                followed |= self.follow_outside_stored(&device, &project);
            }
        }
        changed || followed
    }

    /// The Git project a cleanup names: one of this machine's, or one of a
    /// node that dials this core with that node's id; `None` for anything
    /// else, a device this core dials among them.
    fn cleanup_workspace(&self, workspace_id: &str) -> Option<(Option<&str>, &WorkspaceSnapshot)> {
        let own = self.snapshot.navigator.workspaces.iter().find(|workspace| {
            workspace.id == workspace_id && workspace.remote_target_id.is_none() && workspace.is_git
        });
        if let Some(workspace) = own {
            return Some((None, workspace));
        }
        self.node_sessions().find_map(|(node, session)| {
            session
                .workspaces
                .iter()
                .find(|workspace| workspace.id == workspace_id && workspace.is_git)
                .map(|workspace| (Some(node), workspace))
        })
    }

    /// The in-use facts of a project's checkouts. A node's panes are named
    /// as its own Herdr names them, which the worker asks; a pane this core
    /// cannot name there leaves its checkout unverified, never idle.
    fn checkout_facts(
        &self,
        node: Option<&str>,
        workspace: &WorkspaceSnapshot,
    ) -> Vec<live::cleanup::CheckoutFacts> {
        let mut facts = crate::agent_state::cleanup_facts(&self.snapshot, workspace);
        let Some(node) = node else {
            return facts;
        };
        for checkout in &mut facts {
            let mut unnamed = 0;
            let mut on_node = |panes: &mut Vec<String>| {
                *panes = panes
                    .iter()
                    .filter_map(|pane| {
                        let named = super::remote_pane_source_id(node, pane).map(str::to_owned);
                        unnamed += usize::from(named.is_none());
                        named
                    })
                    .collect();
            };
            on_node(&mut checkout.terminal_panes);
            on_node(&mut checkout.panes);
            checkout.agent_unknown += unnamed;
        }
        facts
    }

    /// Only the in-use facts of a Git project's checkouts, for a read that
    /// happens once per move: none of the folder or size maps
    /// [`Self::cleanup_input`] copies.
    pub(crate) fn cleanup_facts(
        &self,
        workspace_id: &str,
    ) -> Option<Vec<live::cleanup::CheckoutFacts>> {
        let (node, workspace) = self.cleanup_workspace(workspace_id)?;
        Some(self.checkout_facts(node, workspace))
    }

    /// What the cleanup worker needs to know about a Git project's
    /// checkouts, copied under the lock so the worker reads nothing of the
    /// runtime while it inspects the disk and Herdr, with the node the
    /// project is on (`None` for this machine).
    pub(crate) fn cleanup_input(
        &self,
        workspace_id: &str,
    ) -> Option<(Option<String>, live::cleanup::ReviewInput)> {
        let (node, workspace) = self.cleanup_workspace(workspace_id)?;
        let listed = match node {
            None => self.worktree_catalog.project(&workspace.path).cloned(),
            // A node's worktrees as its read listed them, with the sizes it
            // measured, as its rows show them.
            Some(node) => self
                .device_worktrees
                .get(node)
                .and_then(|listed| listed.projects.get(&workspace.path))
                .map(|project| {
                    let mut project = project.clone();
                    if let Some(disk) = self.device_disk.get(node) {
                        measure_worktrees(&mut project, disk);
                    }
                    project
                }),
        };
        // The pull request list the sidebar judges merges from; a node's
        // project this core has not read asks per branch (`MergeBasis::heads`).
        let base = listed
            .as_ref()
            .and_then(|project| project.base_branch.clone());
        let merge = live::cleanup::MergeBasis {
            heads: match node {
                None => crate::worktrees::merged_heads(
                    self.github.project(&workspace.path),
                    base.as_deref(),
                ),
                Some(node) => crate::worktrees::merged_heads(
                    self.device_github
                        .get(node)
                        .and_then(|github| github.project(&workspace.path)),
                    base.as_deref(),
                ),
            },
            base,
        };
        let worktrees = listed.iter().flat_map(|project| &project.worktrees);
        let current = match node {
            None => self
                .focused_local_checkout()
                .filter(|(focused, _)| focused.id == workspace.id)
                .map(|(_, checkout)| PathBuf::from(&checkout.path)),
            Some(_) => self
                .focused_node_checkout()
                .filter(|(_, focused, _)| focused.id == workspace.id)
                .map(|(_, _, checkout)| PathBuf::from(&checkout.path)),
        };
        let input = live::cleanup::ReviewInput {
            workspace_id: workspace.id.clone(),
            repository_root: workspace.path.clone(),
            root: PathBuf::from(&workspace.path),
            current,
            checkouts: self.checkout_facts(node, workspace),
            folders: worktrees
                .clone()
                .filter(|worktree| !worktree.disk.folders.is_empty())
                .map(|worktree| (PathBuf::from(&worktree.path), worktree.disk.folders.clone()))
                .collect(),
            bytes: worktrees
                .filter_map(|worktree| {
                    Some((PathBuf::from(&worktree.path), worktree.disk.total_bytes?))
                })
                .collect(),
            merge,
        };
        Some((node.map(str::to_owned), input))
    }

    /// Where a cleanup of `root` runs: this machine's Herdr and node, or
    /// node `node`'s Herdr and link, with its merge proofs read by the
    /// repository's name here (D-16).
    fn cleanup_place(
        &mut self,
        node: Option<&str>,
        root: &str,
    ) -> Result<live::cleanup::CleanupPlace, String> {
        // A node's project whose GitHub repository is not named has no
        // squash-merge proof to read, and its rows say why; its worktrees
        // merged by Git still go.
        let github = self
            .github_place(node, root)
            .map(|place| crate::live::GhPlace {
                node: self.own_node(),
                place,
            });
        let Some(node) = node else {
            let context = self.live.clone().ok_or_else(|| {
                "A live Herdr connection is required to verify what is in use. Connect and review again.".to_owned()
            })?;
            return Ok(live::cleanup::CleanupPlace::local(context, github));
        };
        let control = self.remote_controls.get(node).cloned().ok_or_else(|| {
            "The node's Herdr connection is unavailable. Reconnect and review again.".to_owned()
        })?;
        Ok(live::cleanup::CleanupPlace::node(
            &control,
            self.node_link(node)?,
            github,
        ))
    }

    /// The node the project `workspace_id` is on, when it is a node's.
    pub(super) fn cleanup_node(&self, workspace_id: &str) -> Option<String> {
        self.cleanup_workspace(workspace_id)?.0.map(str::to_owned)
    }

    /// Publishes the cleanup on the project it names: this machine's rows,
    /// and the session of the node the project is on, and takes a cleanup it
    /// replaced off the node whose session carried that one.
    pub(super) fn publish_cleanup(&mut self, node: Option<&str>) {
        self.refresh_worktree_projection();
        let shown = self
            .cleanup
            .is_some()
            .then(|| node.map(str::to_owned))
            .flatten();
        let previous = std::mem::replace(&mut self.cleanup_published_node, shown);
        if let Some(previous) = previous.filter(|previous| Some(previous.as_str()) != node) {
            self.refresh_device_catalog(&previous);
        }
        if let Some(node) = node {
            self.refresh_device_catalog(node);
        }
    }

    /// The worker of cleanup `id` has ended, however it ended.
    pub(crate) fn cleanup_worker_finished(&mut self, id: u64) {
        if self.cleanup_worker == Some(id) {
            self.cleanup_worker = None;
        }
    }

    pub(crate) fn ingest_cleanup(&mut self, answer: live::cleanup::CleanupSnapshot) -> bool {
        if self
            .cleanup
            .as_ref()
            .is_none_or(|current| current.id != answer.id)
        {
            return false;
        }
        // What was removed is news for the catalog and the sizes once the run
        // has finished, not on every progress step.
        let removed = answer.phase == "complete"
            && (answer
                .rows
                .iter()
                .any(|row| row.result.as_deref() == Some("removed"))
                || answer
                    .cell_results
                    .iter()
                    .any(|cell| cell.outcome == "removed"));
        let node = self.cleanup_node(&answer.workspace_id);
        let proved = self.keep_review_merge_proofs(node.as_deref(), &answer);
        self.cleanup = Some(answer);
        if proved && !removed {
            match node.as_deref() {
                None => {
                    self.refresh_worktree_projection();
                }
                Some(node) => {
                    self.refresh_device_catalog(node);
                }
            }
        }
        if removed {
            match node.as_deref() {
                None => self.refresh_worktrees(),
                Some(node) => {
                    self.request_device_worktrees(node, true);
                }
            }
            self.remeasure_disk();
        }
        self.publish_cleanup(node.as_deref());
        true
    }

    /// Keeps the heads review `answer` proved merged by branch, so the
    /// sidebar's landed mark reads them too; whether any was new.
    fn keep_review_merge_proofs(
        &mut self,
        node: Option<&str>,
        answer: &live::cleanup::CleanupSnapshot,
    ) -> bool {
        if answer.merge_proofs.is_empty() {
            return false;
        }
        // Proofs of a project that left the catalog or whose default branch
        // changed are dropped; a review judged against a base that is no
        // longer the project's keeps nothing.
        let proofs = std::mem::take(&mut self.review_merge_proofs);
        self.review_merge_proofs = proofs
            .into_iter()
            .filter(|((node, root, base), _)| {
                self.listed_base(node.as_deref(), root).as_ref() == Some(base)
            })
            .collect();
        let Some(base) = answer
            .merge_base
            .clone()
            .filter(|base| self.listed_base(node, &answer.repository_root).as_ref() == Some(base))
        else {
            return false;
        };
        let kept = self
            .review_merge_proofs
            .entry((
                node.map(str::to_owned),
                answer.repository_root.clone(),
                base,
            ))
            .or_default();
        let before = kept.len();
        for head in &answer.merge_proofs {
            if kept.len() >= REVIEW_PROOF_LIMIT {
                crate::diagnostic!(serde_json::json!({
                    "component": "cleanup",
                    "kind": "review.merge_proofs_full",
                    "project": answer.repository_root,
                    "limit": REVIEW_PROOF_LIMIT,
                }));
                break;
            }
            kept.insert(head.clone());
        }
        kept.len() != before
    }

    /// The default branch the worktree read of `root` judged merges
    /// against, on this machine or on node `node`.
    fn listed_base(&self, node: Option<&str>, root: &str) -> Option<String> {
        match node {
            None => self.worktree_catalog.project(root),
            Some(node) => self
                .device_worktrees
                .get(node)
                .and_then(|listed| listed.projects.get(root)),
        }
        .and_then(|project| project.base_branch.clone())
    }

    pub(super) fn review_cleanup(&mut self, workspace_id: &str) -> bool {
        if self.cleanup_worker.is_some()
            || self
                .cleanup
                .as_ref()
                .is_some_and(|r| matches!(r.phase.as_str(), "loading" | "removing"))
        {
            return false;
        }
        let Some((node, input)) = self.cleanup_input(workspace_id) else {
            crate::diagnostic!(serde_json::json!({
                "component": "cleanup",
                "kind": "review.not_local_git",
                "workspace_id": workspace_id,
            }));
            return false;
        };
        self.next_cleanup_id = self.next_cleanup_id.wrapping_add(1).max(1);
        let mut review = live::cleanup::CleanupSnapshot {
            id: self.next_cleanup_id,
            workspace_id: workspace_id.to_owned(),
            repository_root: input.repository_root.clone(),
            phase: "loading".into(),
            ..Default::default()
        };
        self.cleanup = Some(review.clone());
        // The sizes the sheet shows come from the Projects tab's sweep;
        // reviewing again measures that project first.
        self.measure_disk_first(node.as_deref(), &input.repository_root.clone());
        let root = input.repository_root.clone();
        let started = self
            .cleanup_place(node.as_deref(), &root)
            .and_then(|place| live::cleanup::spawn_review(place, review.id, input));
        if started.is_ok() {
            self.cleanup_worker = Some(review.id);
        }
        if let Err(message) = started {
            review.phase = "failed".into();
            review.usage_error = Some(message.clone());
            review.message = Some(message);
            self.cleanup = Some(review);
        }
        self.publish_cleanup(node.as_deref());
        true
    }

    pub(super) fn confirm_cleanup(&mut self, payload: CleanupConfirmPayload) -> bool {
        // A review worker that has published its answer may not have ended
        // yet; a confirmation waits for it, so its end can never free the
        // lane of the worker this one starts.
        if self.cleanup_worker.is_some() {
            return false;
        }
        let Some(review) = self.cleanup.clone().filter(|r| {
            r.id == payload.id && r.phase == "review" && r.usage_error.is_none() && r.usage_ready
        }) else {
            return false;
        };
        // A confirmation names at most every row and each row's two layers;
        // anything past that is not from this review, and is not filtered.
        let limit = review.rows.len().saturating_mul(2);
        let mut paths: Vec<_> = payload
            .paths
            .into_iter()
            .take(limit)
            .filter(|path| {
                review
                    .rows
                    .iter()
                    .any(|row| row.path == *path && row.exclusion.is_none() && row.in_use.is_none())
            })
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        paths.sort();
        // A worktree goes with everything in it, so its own cells are not
        // emptied one by one.
        let mut cells: Vec<live::cleanup::CellChoice> = Vec::new();
        for cell in payload.cells.into_iter().take(limit) {
            let Some(layer) = hide_node_link::disk::Layer::from_code(&cell.layer) else {
                continue;
            };
            let choice = live::cleanup::CellChoice {
                path: cell.path,
                layer,
            };
            if !paths.contains(&choice.path)
                && !cells.contains(&choice)
                && review
                    .rows
                    .iter()
                    .any(|row| row.path == choice.path && row.in_use.is_none())
            {
                cells.push(choice);
            }
        }
        if paths.is_empty() && cells.is_empty() {
            return false;
        }
        let Some((node, input)) = self.cleanup_input(&review.workspace_id) else {
            return false;
        };
        self.cleanup.as_mut().unwrap().phase = "removing".into();
        let started = self
            .cleanup_place(node.as_deref(), &review.repository_root)
            .and_then(|place| {
                live::cleanup::spawn_confirm(place, review.clone(), input, paths, cells)
            });
        if started.is_ok() {
            self.cleanup_worker = Some(review.id);
        }
        if let Err(message) = started {
            let active = self.cleanup.as_mut().unwrap();
            active.phase = "failed".into();
            active.message = Some(message);
        }
        self.publish_cleanup(node.as_deref());
        true
    }

    pub(super) fn refresh_worktree_projection(&mut self) -> bool {
        let before_catalog = self.worktree_catalog.clone();
        let before_navigator = self.snapshot.navigator.clone();
        let before_git = self.snapshot.git_worktrees.clone();
        let before_remote = self.snapshot.git_worktrees_remote;

        let pane_rows = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.checkouts.iter())
            .map(|checkout| {
                (
                    checkout.path.clone(),
                    checkout
                        .tabs
                        .iter()
                        .flat_map(|tab| tab.panes.iter())
                        .map(|pane| pane.id.clone())
                        .collect::<HashSet<_>>(),
                )
            })
            .collect::<HashMap<_, _>>();
        // The agent line reads the rows the sidebar already projected and the
        // instrumentation the pane header already resolved, so Overview
        // repeats neither judgement (PRD B34, B35, engineering rule 7).
        let agent_chips = self
            .snapshot
            .navigator
            .agents
            .iter()
            .map(|agent| (agent.pane_id.clone(), crate::sidebar::agent_chip(agent)))
            .collect::<HashMap<_, _>>();
        let pane_instrumentation = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.checkouts.iter())
            .flat_map(|checkout| checkout.tabs.iter())
            .flat_map(|tab| tab.panes.iter())
            .filter_map(|pane| Some((pane.id.clone(), pane.children.clone()?)))
            .collect::<HashMap<_, _>>();
        let requested_github: HashSet<_> = self
            .github_request()
            .projects
            .iter()
            .map(|p| hide_platform::path::to_wire_lossy(&p.root))
            .collect();
        let github = self.github.clone();
        let disk_usage = self.disk_usage.clone();
        let review_proofs = self.review_merge_proofs.clone();

        for project in &mut self.worktree_catalog.projects {
            let github_project = github.project(&project.root_path);
            project.github = github_project.map(|g| g.status.clone()).unwrap_or_else(|| {
                crate::model::GithubStatusSnapshot {
                    loading: requested_github.contains(&project.root_path),
                    ..Default::default()
                }
            });
            // One per branch, as the catalog always carried; a checkout's own
            // pull request is on its row (`associate_pull_requests`).
            project.pull_requests = github_project
                .map(|g| {
                    crate::github::preferred_per_branch(&g.pull_requests)
                        .into_iter()
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            project.pull_request_window = crate::github::PULL_REQUEST_LIMIT.into();
            let merged_heads = with_review_proofs(
                crate::worktrees::merged_heads(github_project, project.base_branch.as_deref()),
                &review_proofs,
                None,
                &project.root_path,
                project.base_branch.as_deref(),
            );
            for worktree in &mut project.worktrees {
                worktree.merged = crate::worktrees::merged(
                    worktree.in_base,
                    worktree.head_sha.as_deref(),
                    merged_heads.as_ref(),
                );
                let panes = pane_rows.get(&worktree.path);
                worktree.pane_count = panes.map_or(0, HashSet::len);
                worktree.agent_line = worktree_agent_line(
                    panes,
                    &agent_chips,
                    &pane_instrumentation,
                    &self.snapshot.navigator.agents,
                );
                worktree.github = github_project
                    .map(|project| project.status.clone())
                    .unwrap_or_default();
                worktree.pull_request = github_project.and_then(|project| {
                    crate::github::pull_request_for_checkout(
                        &project.pull_requests,
                        worktree.branch.as_deref(),
                        worktree.head_sha.as_deref(),
                    )
                    .cloned()
                });
                worktree.deletion_gate = crate::worktrees::deletion_gate(
                    worktree,
                    worktree.branch == project.base_branch && project.base_branch.is_some(),
                    worktree.pane_count,
                );
            }
            measure_worktrees(project, &disk_usage);
            project.worktrees.sort_by(|left, right| {
                right
                    .is_main
                    .cmp(&left.is_main)
                    .then_with(|| (right.pane_count > 0).cmp(&(left.pane_count > 0)))
                    .then_with(|| {
                        right
                            .last_commit_unix_seconds
                            .unwrap_or(0)
                            .cmp(&left.last_commit_unix_seconds.unwrap_or(0))
                    })
                    .then_with(|| left.path.cmp(&right.path))
            });
        }

        let disk_marks: HashMap<String, (bool, bool)> = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .filter(|workspace| workspace.remote_target_id.is_none())
            .map(|workspace| {
                let marks = self.disk_sweep_marks(None, &workspace.path);
                (workspace.path.clone(), marks)
            })
            .collect();
        let link_summaries = self
            .snapshot
            .link_summaries
            .as_ref()
            .map(|summaries| &summaries.projects);
        for workspace in &mut self.snapshot.navigator.workspaces {
            if workspace.remote_target_id.is_some() {
                continue;
            }
            workspace::apply_worktrees(workspace, &self.worktree_catalog, &self.catalog_paths);
            // The commit a checkout is on is what ties a settled pull request
            // to it, so a catalog read that moved a HEAD decides again here,
            // and whether its work landed follows the same read.
            let project = github.project(&workspace.path);
            associate_pull_requests(workspace, project);
            super::links::associate_landed(
                workspace,
                link_summaries.and_then(|summaries| summaries.get(&workspace.id)),
            );
            super::links::associate_pr_sessions(
                &mut workspace.pull_requests,
                link_summaries.and_then(|summaries| summaries.get(&workspace.id)),
            );
            // Git refreshes the persistent purpose sources. Restore the
            // agent/PR fallback in this same projection before publishing it.
            crate::sidebar::sync_checkout_purposes(
                std::slice::from_mut(workspace),
                &self.snapshot.navigator.agents,
            );
            let marks = disk_marks.get(&workspace.path).copied().unwrap_or_default();
            workspace.disk = match workspace.is_git {
                true => project_disk(self.worktree_catalog.project(&workspace.path), marks),
                false => super::disk_sweep::folder_disk(
                    &self.disk_usage,
                    &workspace.path,
                    marks.0,
                    marks.1,
                ),
            };
            workspace.cleanup = self
                .cleanup
                .as_ref()
                .filter(|review| review.workspace_id == workspace.id)
                .cloned();
        }
        crate::project_context::sort_projects(
            &mut self.snapshot.navigator.workspaces,
            &self.snapshot.navigator.agents,
        );
        self.refresh_unowned_dormant_rows();
        self.refresh_inactive_groups();

        let focused_project = self
            .snapshot
            .navigator
            .focused_checkout_id
            .as_deref()
            .and_then(|focused_id| {
                self.snapshot.navigator.workspaces.iter().find(|workspace| {
                    workspace
                        .checkouts
                        .iter()
                        .any(|checkout| checkout.id == focused_id)
                })
            });
        self.snapshot.git_worktrees_remote =
            focused_project.is_some_and(|workspace| workspace.remote_target_id.is_some());
        self.snapshot.git_worktrees = focused_project
            .filter(|workspace| workspace.remote_target_id.is_none())
            .and_then(|workspace| self.worktree_catalog.project(&workspace.path))
            .cloned();
        self.refresh_card();
        self.refresh_browser_inventory_scope()
            || before_catalog != self.worktree_catalog
            || before_navigator != self.snapshot.navigator
            || before_git != self.snapshot.git_worktrees
            || before_remote != self.snapshot.git_worktrees_remote
    }

    pub fn worktree_catalog(&self) -> crate::model::WorktreeCatalogSnapshot {
        self.worktree_catalog.clone()
    }

    /// Every local Git project, whether or not a screen that shows its pull
    /// requests is open: the sidebar row is such a screen. At most
    /// `GITHUB_PROJECT_LIMIT` of them: the project in front first, then those
    /// a screen named, then the rest, each group in path order. The chosen are
    /// returned in path order so the request does not move when the projects'
    /// activity order does; the second value is how many local Git projects
    /// there are in all.
    pub(super) fn github_projects(&self) -> (Vec<&WorkspaceSnapshot>, usize) {
        let in_front = self
            .focused_local_checkout()
            .map(|(workspace, _)| workspace.path.as_str());
        let mut projects: Vec<&WorkspaceSnapshot> = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .filter(|workspace| workspace.remote_target_id.is_none() && workspace.is_git)
            .collect();
        let total = projects.len();
        projects.sort_by(|left, right| {
            let rank = |workspace: &WorkspaceSnapshot| {
                if Some(workspace.path.as_str()) == in_front {
                    0
                } else if self.github_wanted.contains(&workspace.path) {
                    1
                } else {
                    2
                }
            };
            (rank(left), &left.path).cmp(&(rank(right), &right.path))
        });
        projects.truncate(GITHUB_PROJECT_LIMIT);
        projects.sort_by(|left, right| left.path.cmp(&right.path));
        (projects, total)
    }

    pub fn github_request(&self) -> crate::github::GithubRequest {
        crate::github::GithubRequest {
            projects: self
                .github_projects()
                .0
                .into_iter()
                .map(|workspace| crate::github::GithubProjectRequest {
                    links: workspace
                        .checkouts
                        .iter()
                        .filter_map(|checkout| self.issue_candidates.get(&checkout.id))
                        .map(|candidate| candidate.reference.clone())
                        .chain(
                            self.snapshot
                                .link_summaries
                                .as_ref()
                                .and_then(|summaries| summaries.projects.get(&workspace.id))
                                .into_iter()
                                .flat_map(|summary| summary.record_issues.iter().cloned()),
                        )
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .take(crate::issues::ISSUE_LIMIT)
                        .collect(),
                    root: PathBuf::from(&workspace.path),
                    repository: crate::project_settings::get(
                        &self.snapshot.ui_state.project_settings,
                        self.node.as_str(),
                        &workspace.path,
                    )
                    .and_then(|record| match record.repository.as_ref() {
                        Some(crate::project_settings::RepositoryConnection::Repository {
                            repository,
                        }) => Some(repository.clone()),
                        _ => None,
                    }),
                    generation: self
                        .github_generations
                        .get(&workspace.path)
                        .copied()
                        .unwrap_or(0),
                    remotes: self
                        .catalog_paths
                        .remotes(&workspace.path)
                        .map(<[String]>::to_vec),
                })
                .collect(),
        }
    }

    /// Asks again for each project whose last answer is `github_wait` old (the full
    /// re-read after a success, sooner after a failed read),
    /// by moving its generation; the reader re-reads exactly the projects
    /// whose generation moved. A project is not asked again while its last ask
    /// is unanswered, so a slow pass is never overtaken by the next one. A
    /// project seen for the first time is not asked for here: the reader has
    /// no answer for it, so its first read is already due.
    pub(crate) fn reread_stale_github(&mut self, now: std::time::Instant) {
        let (projects, total) = self.github_projects();
        let paths: Vec<String> = projects
            .into_iter()
            .map(|workspace| workspace.path.clone())
            .collect();
        let over_limit = total.saturating_sub(GITHUB_PROJECT_LIMIT);
        if over_limit != self.github_over_limit {
            self.github_over_limit = over_limit;
            if over_limit > 0 {
                crate::diagnostic!(serde_json::json!({
                    "component": "github",
                    "kind": "projects.over_limit",
                    "limit": GITHUB_PROJECT_LIMIT,
                    "local_git_projects": total,
                    "not_read": over_limit,
                }));
            }
        }
        let registered: HashSet<&str> = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .map(|workspace| workspace.path.as_str())
            .collect();
        self.github_wanted
            .retain(|path| registered.contains(path.as_str()));
        let known = |path: &String| paths.contains(path);
        self.github_clock.retain(|path, _| known(path));
        self.github_polled.retain(|path| known(path));
        self.github_answered.retain(|path, _| known(path));
        self.github_remotes.retain(|path, _| known(path));
        self.github_read_generation.retain(|path, _| known(path));
        self.github_settled.retain(known);
        self.merged_here.retain(|(path, _), _| known(path));
        // A sighting that grew old while its project's answer was owed (no
        // window attached, no read) sets off nothing when the answer lands.
        self.github_sighted
            .retain(|_, kept| now <= kept.fresh_until);
        for kept in self.github_sighted.values_mut() {
            kept.waiting.retain(known);
        }
        for path in paths {
            let before = self.github_clock.get(&path).copied();
            if self.github_remotes_moved(&path) {
                crate::diagnostic!(serde_json::json!({
                    "component": "github",
                    "kind": "read.remotes_changed",
                    "project": path,
                }));
                self.github_answered.remove(&path);
                self.ask_github_again(&path);
                continue;
            }
            if let Some(outcome) = self.github_answered.remove(&path) {
                let failures = match outcome {
                    GithubReadOutcome::Read | GithubReadOutcome::NoRemote => 0,
                    GithubReadOutcome::IssuesFailed | GithubReadOutcome::Failed => {
                        before.map_or(0, GithubClock::failures).saturating_add(1)
                    }
                };
                // Remotes the catalog does not know (an included config, an
                // older node) can gain a GitHub one unseen, so `gh`'s word
                // that there is none is asked again on the full wait.
                let clock = if outcome == GithubReadOutcome::NoRemote
                    && self.catalog_paths.remotes(&path).is_some()
                {
                    GithubClock::NoRemote
                } else {
                    GithubClock::Answered {
                        at: now,
                        failures,
                        pull_requests_read: outcome.pull_requests_read(),
                    }
                };
                self.github_clock.insert(path.clone(), clock);
                if self.sighted_owed_after_answer(&path, !outcome.pull_requests_read()) {
                    self.ask_github_again(&path);
                }
                continue;
            }
            match before {
                None => {
                    self.github_clock
                        .insert(path, GithubClock::Waiting { failures: 0 });
                }
                Some(GithubClock::Answered { at, failures, .. })
                    if now.duration_since(at) >= github_wait(failures) =>
                {
                    if failures > 0 {
                        crate::diagnostic!(serde_json::json!({
                            "component": "github",
                            "kind": "read.retry",
                            "project": path,
                            "attempt": failures,
                            "delay_ms": github_wait(failures).as_millis(),
                        }));
                    }
                    self.github_clock
                        .insert(path.clone(), GithubClock::Waiting { failures });
                    self.bump_github_generation(&path);
                }
                Some(_) => {}
            }
        }
    }

    /// Reads GitHub again for each project a session just printed the address
    /// of a pull request in that the last answer does not hold, so a pull
    /// request an agent opens reaches its row without waiting for the
    /// five-minute re-read. The read is the existing one: the project's
    /// generation moves and its clock waits for the answer, as the stale
    /// re-read does, so a project is still never asked again while an ask is
    /// unanswered; a sighting that finds one waits for that answer and reads
    /// only if the answer does not hold it, and a project whose last read
    /// failed is left to its own retry. An address sets off its read once
    /// while it is recent; one the read still does not return (another
    /// repository, past the newest 200) is not asked for again. The read is
    /// quiet: the row does not show `loading` for it.
    pub(crate) fn read_sighted_pull_requests(
        &mut self,
        sighted: &[crate::labels::worker::SightedPullRequest],
        (now, now_unix_ms): (std::time::Instant, u64),
    ) {
        self.github_sighted
            .retain(|_, kept| now <= kept.fresh_until);
        let candidates = self.sighted_candidates();
        let mut read = BTreeSet::new();
        for sighting in sighted {
            let address = (sighting.repository.clone(), sighting.number);
            // Every pull request read is keyed here by its address, so a
            // sighting it holds needs no read.
            let Some(fresh_for) =
                crate::labels::worker::sighting_fresh_for(sighting.at_unix_ms, now_unix_ms)
            else {
                continue;
            };
            if self.pull_request_times.contains_key(&address)
                || self.github_sighted.contains_key(&address)
            {
                continue;
            }
            let projects = self.sighted_projects(&candidates, sighting);
            if projects.is_empty() {
                // Not remembered: an address no read project owns must not
                // take a place a project's own pull request needs.
                crate::diagnostic!(serde_json::json!({
                    "component": "github",
                    "kind": "read.sighted_unowned",
                    "repository": sighting.repository,
                    "number": sighting.number,
                    "pane_id": sighting.pane_id,
                }));
                continue;
            }
            if self.github_sighted.len() >= SIGHTED_ASKED_LIMIT {
                crate::diagnostic!(serde_json::json!({
                    "component": "github",
                    "kind": "read.sighted_over_limit",
                    "limit": SIGHTED_ASKED_LIMIT,
                    "repository": sighting.repository,
                    "number": sighting.number,
                }));
                continue;
            }
            let mut kept = Sighted {
                fresh_until: now + fresh_for,
                waiting: BTreeSet::new(),
                owed: BTreeSet::new(),
            };
            let mut read_now = Vec::new();
            for path in projects {
                match self.sighted_read(&path) {
                    SightedRead::Now => {
                        kept.owed.insert(path.clone());
                        read_now.push(path);
                    }
                    SightedRead::AfterAnswer => {
                        kept.owed.insert(path.clone());
                        kept.waiting.insert(path);
                    }
                    SightedRead::Never => {}
                }
            }
            crate::diagnostic!(serde_json::json!({
                "component": "github",
                "kind": "read.sighted",
                "repository": sighting.repository,
                "number": sighting.number,
                "pane_id": sighting.pane_id,
                "read": read_now,
                "after_answer": kept.waiting,
            }));
            read.extend(read_now);
            self.github_sighted.insert(address, kept);
        }
        for path in read {
            self.ask_github_again(&path);
        }
    }

    /// The read local projects by path, each with the repository its pull
    /// requests name, if it has one yet.
    fn sighted_candidates(&self) -> Vec<(String, Option<String>)> {
        self.github_projects()
            .0
            .into_iter()
            .map(|workspace| {
                let repository = self.github.project(&workspace.path).and_then(|project| {
                    project
                        .pull_requests
                        .iter()
                        .find_map(|pull_request| pull_request_address(&pull_request.url))
                        .map(|(repository, _)| repository)
                });
                (workspace.path.clone(), repository)
            })
            .collect()
    }

    /// The read local projects a sighted pull request belongs to: those whose
    /// pull requests are in its repository; when none is, the project of the
    /// pane whose session printed it, unless that project's pull requests
    /// name another repository. A repository's first pull request has no
    /// earlier one to name it, and the session that made it works in it; a
    /// fork's checkout whose agent opens a pull request upstream is therefore
    /// not read for it.
    fn sighted_projects(
        &self,
        candidates: &[(String, Option<String>)],
        sighting: &crate::labels::worker::SightedPullRequest,
    ) -> Vec<String> {
        let matching: Vec<String> = candidates
            .iter()
            .filter(|(_, repository)| repository.as_deref() == Some(sighting.repository.as_str()))
            .map(|(path, _)| path.clone())
            .collect();
        if !matching.is_empty() {
            return matching;
        }
        let printed_in = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .filter(|workspace| workspace.remote_target_id.is_none())
            .find(|workspace| {
                workspace.checkouts.iter().any(|checkout| {
                    checkout
                        .tabs
                        .iter()
                        .any(|tab| tab.panes.iter().any(|pane| pane.id == sighting.pane_id))
                })
            })
            .map(|workspace| workspace.path.as_str());
        candidates
            .iter()
            .find(|(path, repository)| Some(path.as_str()) == printed_in && repository.is_none())
            .map(|(path, _)| vec![path.clone()])
            .unwrap_or_default()
    }

    fn sighted_read(&self, path: &str) -> SightedRead {
        // Whoever asked (the clock, a refresh, the request view, a sighting),
        // an ask is outstanding while the generation asked for is not the one
        // last answered, and until the clock takes an answer that landed.
        let asked = self.github_generations.get(path).copied().unwrap_or(0);
        let outstanding = self.github_read_generation.get(path) != Some(&asked)
            || self.github_answered.contains_key(path);
        match self.github_clock.get(path) {
            _ if outstanding => SightedRead::AfterAnswer,
            Some(GithubClock::Answered {
                pull_requests_read: true,
                ..
            }) if self.github_settled.contains(path) => SightedRead::Now,
            // A read whose pull requests failed is left to its own retry, and
            // a repository with no GitHub remote holds no pull request.
            Some(GithubClock::Answered {
                pull_requests_read: false,
                ..
            })
            | Some(GithubClock::NoRemote) => SightedRead::Never,
            _ => SightedRead::AfterAnswer,
        }
    }

    /// Whether the read of a pull request a session printed at `address` is
    /// still owed: a read it set off has not answered, so a turn that just
    /// printed it is not a stop yet (status-marks D-09, B14).
    pub(super) fn sighted_read_owed(&self, address: &(String, u64)) -> bool {
        !self.pull_request_times.contains_key(address)
            && self
                .github_sighted
                .get(address)
                .is_some_and(|kept| !kept.owed.is_empty())
    }

    /// An answer for `path` landed: the sightings it owed a read stop holding
    /// their turns, unless the answer was for an ask already out when the
    /// address was printed, worked and does not hold it, which the clock
    /// reads again for (`sighted_owed_after_answer`).
    fn release_sighted_reads(&mut self, path: &str, outcome: GithubReadOutcome) {
        let times = &self.pull_request_times;
        for (address, kept) in &mut self.github_sighted {
            let read_again = kept.waiting.contains(path)
                && outcome.pull_requests_read()
                && !times.contains_key(address);
            if !read_again {
                kept.owed.remove(path);
            }
        }
    }

    /// Takes `path` off every sighting waiting for its answer, and whether one
    /// of them is owed a read: the answer worked and does not hold it.
    fn sighted_owed_after_answer(&mut self, path: &str, failed: bool) -> bool {
        let times = &self.pull_request_times;
        let mut owed = false;
        for (address, kept) in &mut self.github_sighted {
            if kept.waiting.remove(path) && !failed && !times.contains_key(address) {
                owed = true;
            }
        }
        owed
    }

    /// Whether the remotes of the repository at `path` moved since the
    /// catalog last said, so its answer may have: a remote added to a
    /// repository that had none is read at once rather than never. The first
    /// value seen is remembered, not a move, and a catalog that does not say
    /// (its path read failed, or an older node) moves nothing.
    fn github_remotes_moved(&mut self, path: &str) -> bool {
        let Some(remotes) = self.catalog_paths.remotes(path) else {
            return false;
        };
        match self.github_remotes.get(path) {
            Some(known) if known.as_slice() == remotes => false,
            known => {
                let moved = known.is_some();
                self.github_remotes
                    .insert(path.to_owned(), remotes.to_vec());
                moved
            }
        }
    }

    /// Reads `path` again because the operator is looking at it (the window
    /// came forward, or the project did), when its last read worked and is
    /// `GITHUB_LOOK_AFTER` old. It is the stale re-read's ask: the clock waits
    /// for the answer, nothing is asked while an ask is outstanding or in
    /// failure backoff, a repository with no GitHub remote is never asked,
    /// and `loading` is left alone, so a project that already has an answer
    /// does not flicker. `now` is passed in so a test moves it without
    /// sleeping.
    pub(super) fn read_when_looked_at(&mut self, path: &str, now: std::time::Instant) -> LookRead {
        let at = match self.github_clock.get(path) {
            Some(GithubClock::Answered {
                at,
                failures: 0,
                pull_requests_read: true,
            }) => *at,
            Some(GithubClock::Waiting { .. }) => return LookRead::Later,
            _ => return LookRead::Never,
        };
        if !matches!(self.sighted_read(path), SightedRead::Now)
            || now.duration_since(at) < GITHUB_LOOK_AFTER
        {
            return LookRead::Later;
        }
        self.ask_github_again(path);
        LookRead::Asked
    }

    /// The path of the local Git project with this workspace id, if the
    /// core reads GitHub for one.
    pub(super) fn local_git_project_path(&self, workspace_id: &str) -> Option<String> {
        self.snapshot
            .navigator
            .workspaces
            .iter()
            .find(|workspace| {
                workspace.id == workspace_id
                    && workspace.is_git
                    && workspace.remote_target_id.is_none()
            })
            .map(|workspace| workspace.path.clone())
    }

    /// Asks for `path` again and waits for that answer, as the stale re-read
    /// does, so nothing asks again while it is unanswered.
    fn ask_github_again(&mut self, path: &str) {
        self.github_clock
            .insert(path.to_owned(), GithubClock::Waiting { failures: 0 });
        self.bump_github_generation(path);
    }

    /// Whether `path` is read for GitHub and no read has answered for it yet.
    fn github_awaiting(&self, path: &str) -> bool {
        !self.github_settled.contains(path)
            && self
                .github_projects()
                .0
                .iter()
                .any(|workspace| workspace.path == path)
    }

    /// Restores what the last run saved, as stale until a read replaces it,
    /// and keeps the store to save every later answer into.
    pub(crate) fn install_github_store(
        &mut self,
        store: crate::github_store::GithubStore,
        restored: Option<crate::model::GithubSnapshot>,
    ) {
        if let Some(mut restored) = restored {
            // A saved provider cache may predate the private connection choice.
            // Reuse it only when its repository agrees with that choice.
            restored.projects.retain(|project| {
                crate::project_settings::get(
                    &self.snapshot.ui_state.project_settings,
                    self.node.as_str(),
                    &project.root_path,
                )
                .and_then(|record| record.repository.as_ref())
                .is_none_or(|connection| match connection {
                    crate::project_settings::RepositoryConnection::Repository { repository } => {
                        project.issues.repository.as_ref() == Some(repository)
                    }
                    crate::project_settings::RepositoryConnection::Auto => false,
                })
            });
            self.github = restored;
            self.apply_pull_requests();
            self.refresh_worktree_projection();
            self.track_memory_promotions();
        }
        self.github_store = Some(store);
    }

    pub fn ingest_github(&mut self, github: crate::model::GithubSnapshot) -> bool {
        self.ingest_github_answer(github, true)
    }

    pub(crate) fn ingest_github_answer(
        &mut self,
        github: crate::model::GithubSnapshot,
        current: bool,
    ) -> bool {
        if !current {
            return false;
        }
        // An empty answer to an empty request says nothing: the navigator has
        // not arrived yet, and taking it would erase the answer restored from
        // the last run and the file it was saved in.
        let requested = self.github_request();
        if requested.projects.is_empty() && github.projects.is_empty() {
            return false;
        }
        // The answer is for the request as it stands now, so every project in
        // it has been read once, including one that returned nothing.
        let mut newly_settled = false;
        let mut answered = Vec::new();
        for project in requested.projects {
            let path = project.root.to_string_lossy().into_owned();
            // The read's own provenance, taken before a failed one is filled
            // in from the previous answer below.
            let outcome = github.project(&path).map_or(
                GithubReadOutcome::Read,
                crate::model::GithubProjectSnapshot::read_outcome,
            );
            // The reader hands back its cached entry for every project in the
            // request, so only a project whose generation moved since the
            // last answer was actually read; counting the others would
            // restart their wait, or climb a failing project's backoff,
            // because a neighbour was asked.
            if self
                .github_read_generation
                .insert(path.clone(), project.generation)
                != Some(project.generation)
            {
                self.github_answered.insert(path.clone(), outcome);
                answered.push((path.clone(), outcome));
            }
            newly_settled |= self.github_settled.insert(path);
        }
        let mut merged = github;
        keep_previous_reads(&mut merged, &self.github);
        self.read_merges_made_here(&mut merged);
        // A mission's pull request that merged, was asked for changes or got
        // a new commit wakes its Conductor (PRD software-factory-conductor
        // D-39); each node keeps what it was last told, so an unchanged
        // answer wakes nothing.
        let node = self.node.as_str().to_owned();
        self.note_mission_pulls(&node, &merged);
        if self.github == merged {
            // Nothing to show changed, but a project that returned nothing
            // stops loading.
            if newly_settled {
                self.apply_pull_requests();
            }
            return self.settle_answered_reads(&answered) | newly_settled;
        }
        self.github = merged;
        if let Some(store) = &self.github_store {
            store.save(self.github.clone());
        }
        self.apply_pull_requests();
        self.refresh_worktree_projection();
        // A promoted card's pull request may have opened, merged or closed.
        self.track_memory_promotions();
        // A check that failed or a merge after a review's changes is a
        // suggestion record (PRD hide-ai-suggestions B4).
        self.feed_suggestion_prs();
        self.track_suggestion_attempts();
        self.settle_answered_reads(&answered);
        true
    }

    /// The reads that answered release the turns they held, and every row's
    /// checks are laid again on what the answer says (B11, B13, B14).
    fn settle_answered_reads(&mut self, answered: &[(String, GithubReadOutcome)]) -> bool {
        for (path, outcome) in answered {
            self.release_sighted_reads(path, *outcome);
        }
        self.refresh_checks_waits()
    }

    /// Marks the pull requests Hide merged as merged in `github` until a read
    /// says so itself: Hide knows its own merge went through, so the pull
    /// request leaves Needs You at once instead of waiting in an unknown merge
    /// state for the next poll. The time shown is the one the merge was seen.
    /// An entry is let go when a fresh read of its project shows the pull
    /// request merged, closed or gone, and after `MERGED_HERE_LIFETIME_MS`
    /// whatever the reads say, so a merge GitHub did not complete (a queue
    /// that ejected it) does not read as merged for good. A read that failed
    /// carries the previous list, overlay included, and releases nothing.
    pub(super) fn read_merges_made_here(&mut self, github: &mut crate::model::GithubSnapshot) {
        let now = unix_milliseconds();
        self.merged_here.retain(|(root, number), merged_at| {
            if now.saturating_sub(*merged_at) > MERGED_HERE_LIFETIME_MS {
                return false;
            }
            let Some(project) = github.projects.iter_mut().find(|p| &p.root_path == root) else {
                return true;
            };
            let fresh = !project.status.stale && project.status.unavailable_reason.is_none();
            let Some(pull_request) = project
                .pull_requests
                .iter_mut()
                .find(|p| p.number == *number)
            else {
                return !fresh;
            };
            if fresh
                && matches!(
                    pull_request.badge,
                    crate::model::PullRequestBadge::Merged | crate::model::PullRequestBadge::Closed
                )
            {
                return false;
            }
            pull_request.badge = crate::model::PullRequestBadge::Merged;
            pull_request.review = None;
            pull_request.merged_at_unix_ms = Some(*merged_at);
            pull_request.closed_at_unix_ms = Some(*merged_at);
            true
        });
    }

    fn bump_github_generation(&mut self, project_path: &str) {
        let generation = self
            .github_generations
            .entry(project_path.to_owned())
            .or_insert(0);
        *generation = generation.wrapping_add(1);
    }

    pub fn refresh_pull_requests(&mut self, project_path: &str) {
        self.bump_github_generation(project_path);
        if let Some(cached) = self
            .github
            .projects
            .iter_mut()
            .find(|p| p.root_path == project_path)
        {
            cached.status.loading = true;
        }
    }

    pub fn refresh_worktrees(&mut self) {
        self.worktree_generation = self.worktree_generation.wrapping_add(1);
        self.snapshot.git_worktrees_loading = true;
    }

    pub(super) fn refresh_project_worktrees(&mut self, project_path: &str) {
        let generation = self
            .worktree_project_generations
            .entry(project_path.to_owned())
            .or_insert(0);
        *generation = generation.wrapping_add(1);
        self.snapshot.git_worktrees_loading = true;
    }

    pub(super) fn open_git_worktree(&mut self, checkout_path: String) -> bool {
        let target = self.worktree_catalog.projects.iter().find_map(|project| {
            project
                .worktrees
                .iter()
                .find(|worktree| worktree.path == checkout_path)
                .map(|worktree| (project.root_path.clone(), worktree.clone()))
        });
        let Some((repository_root, worktree)) = target else {
            self.set_error(
                "worktree.open_unknown",
                format!("Worktree is no longer listed: {checkout_path}"),
                true,
            );
            self.refresh_worktrees();
            return true;
        };
        if worktree.missing {
            self.ingest_worktree_open_result(
                checkout_path,
                Err("Worktree is missing on disk".to_owned()),
            );
            return true;
        }
        let newest_pane = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.checkouts.iter())
            .filter(|checkout| checkout.path == checkout_path)
            .flat_map(|checkout| checkout.tabs.iter())
            .flat_map(|tab| tab.panes.iter())
            .max_by_key(|pane| pane.activity_at_unix_ms.unwrap_or(0))
            .map(|pane| pane.id.clone());
        let Some(context) = self.live.as_ref().cloned() else {
            self.ingest_worktree_open_result(
                checkout_path,
                Err("Opening a worktree needs a live Herdr connection".to_owned()),
            );
            return true;
        };
        if let Err(message) =
            live::spawn_worktree_open(context, checkout_path.clone(), repository_root, newest_pane)
        {
            self.ingest_worktree_open_result(checkout_path, Err(message));
        }
        true
    }

    pub(super) fn set_git_worktree_base(&mut self, project_path: String, branch: String) -> bool {
        let listed = self
            .worktree_catalog
            .project(&project_path)
            .is_some_and(|project| {
                project
                    .worktrees
                    .iter()
                    .any(|worktree| worktree.branch.as_deref() == Some(branch.as_str()))
            });
        if !listed {
            self.set_error(
                "worktree.base_unavailable",
                format!("Branch {branch} is no longer checked out in this repository"),
                true,
            );
            self.refresh_worktrees();
            return true;
        }
        if self
            .snapshot
            .ui_state
            .project_base_branches
            .insert(project_path, branch)
            .is_some()
        {
            // Replacing and inserting both persist below; the return value is
            // deliberately not used as the change detector because the same
            // branch is an idempotent no-op at the reader boundary.
        }
        self.persist_ui_state();
        self.refresh_worktrees();
        true
    }

    pub(super) fn remove_git_worktree(&mut self, payload: RemoveWorktreePayload) -> bool {
        if let Some(removal) = self.snapshot.worktree_removal.as_ref()
            && matches!(removal.phase.as_str(), "checking" | "closing" | "removing")
        {
            if removal.checkout_path == payload.checkout_path {
                return false;
            }
            self.set_error(
                "worktree.remove_busy",
                format!(
                    "Finish removing {} before deleting another worktree",
                    removal.checkout_path
                ),
                true,
            );
            return true;
        }
        if let Some(device) = payload
            .device_id
            .clone()
            .filter(|device| device != self.node.as_str())
        {
            return self.remove_device_worktree(&device, payload);
        }
        let target = self.worktree_catalog.projects.iter().find_map(|project| {
            project
                .worktrees
                .iter()
                .find(|worktree| worktree.path == payload.checkout_path)
                .map(|worktree| {
                    (
                        project.root_path.clone(),
                        project.base_branch.clone(),
                        worktree.clone(),
                    )
                })
        });
        let Some((repository_root, protected_base_branch, worktree)) = target else {
            self.set_error(
                "worktree.remove_unknown",
                format!("Worktree is no longer listed: {}", payload.checkout_path),
                true,
            );
            self.refresh_worktrees();
            return true;
        };
        if !self.worktree_removal_allowed(&worktree, &payload) {
            return true;
        }
        let id = self.begin_worktree_removal(
            None,
            repository_root,
            protected_base_branch,
            worktree,
            &payload,
        );
        self.start_worktree_preflight(id, None, payload.close_descendant_pane_ids)
    }

    /// `remove_worktree` for a device's linked worktree (PRD S5.5 B28, B29):
    /// the same gate, receipt and phases as this machine's, with the device's
    /// Herdr closing its panes and its helper rechecking and removing.
    fn remove_device_worktree(&mut self, device: &str, payload: RemoveWorktreePayload) -> bool {
        let target = self.device_worktrees.get(device).and_then(|worktrees| {
            worktrees.projects.values().find_map(|project| {
                project
                    .worktrees
                    .iter()
                    .find(|worktree| worktree.path == payload.checkout_path)
                    .map(|worktree| {
                        (
                            project.root_path.clone(),
                            project.base_branch.clone(),
                            worktree.clone(),
                        )
                    })
            })
        });
        let Some((repository_root, protected_base_branch, worktree)) = target else {
            self.set_error(
                "worktree.remove_unknown",
                format!("Worktree is no longer listed: {}", payload.checkout_path),
                true,
            );
            self.request_device_worktrees(device, true);
            return true;
        };
        if !self.worktree_removal_allowed(&worktree, &payload) {
            return true;
        }
        if let Err(message) = self.device_worktree_target(device) {
            self.set_error(
                "worktree.remove_unavailable",
                format!("Deleting a worktree on this device needs its connection: {message}"),
                true,
            );
            return true;
        }
        let id = self.begin_worktree_removal(
            Some(device),
            repository_root,
            protected_base_branch,
            worktree,
            &payload,
        );
        self.start_worktree_preflight(
            id,
            Some(device.to_owned()),
            payload.close_descendant_pane_ids,
        )
    }

    fn start_worktree_preflight(
        &mut self,
        id: u64,
        device: Option<String>,
        outside: Vec<String>,
    ) -> bool {
        let context = match device.as_deref() {
            Some(device) => self.device_worktree_target(device),
            None => self.local_worktree_target(),
        };
        let result =
            context.and_then(|context| live::spawn_worktree_preflight(context, id, outside));
        if let Err(reason) = result {
            self.ingest_worktree_preflight_result(
                id,
                Vec::new(),
                Err(live::RemovalFailure::new(
                    crate::model::WorktreeRemovalNotice::WorkerUnavailable,
                    reason,
                )),
            );
        }
        true
    }

    /// Only a successful host measurement admits any descendant or local close.
    pub(crate) fn ingest_worktree_preflight_result(
        &mut self,
        id: u64,
        outside: Vec<String>,
        result: Result<(), live::RemovalFailure>,
    ) -> bool {
        let Some(removal) = self
            .snapshot
            .worktree_removal
            .as_mut()
            .filter(|row| row.id == id && row.phase == "checking")
        else {
            return false;
        };
        if let Err(failure) = result {
            crate::diagnostic!(serde_json::json!({
                "component": "worktree_removal", "kind": "preflight_failed",
                "id": id, "detail": failure.detail,
            }));
            removal.phase = "failed".to_owned();
            removal.notice = Some(failure.notice);
            let device = removal.device_id.clone();
            match device.as_deref() {
                Some(device) => {
                    self.request_device_worktrees(device, true);
                }
                None => self.refresh_worktrees(),
            };
            return true;
        }
        removal.phase = "closing".to_owned();
        let device = removal.device_id.clone();
        let path = removal.checkout_path.clone();
        self.close_worktree_then_remove(id, device, path, outside)
    }

    /// The panes of a checkout a removal closes, by the ids its own Herdr
    /// uses: this machine's as they are, a device's without their scope.
    fn checkout_pane_ids(&self, device: Option<&str>, checkout_path: &str) -> Vec<String> {
        let workspaces = match device {
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
        workspaces
            .iter()
            .flat_map(|workspace| &workspace.checkouts)
            .filter(|checkout| checkout.path == checkout_path)
            .flat_map(|checkout| &checkout.tabs)
            .flat_map(|tab| &tab.panes)
            .filter_map(|pane| match device {
                Some(device) => super::remote_pane_source_id(device, &pane.id).map(str::to_owned),
                None => Some(pane.id.clone()),
            })
            .collect()
    }

    /// After the confirmed removal is recorded: the operator's chosen
    /// descendants outside the checkout close first when there are any
    /// (PRD close-agent-subtree D-36), then the checkout's own panes.
    fn close_worktree_then_remove(
        &mut self,
        id: u64,
        device: Option<String>,
        checkout_path: String,
        outside: Vec<String>,
    ) -> bool {
        if outside.is_empty() {
            return self.close_worktree_panes(id, device, checkout_path);
        }
        let inside = self
            .checkout_pane_ids(device.as_deref(), &checkout_path)
            .into_iter()
            .map(|pane| match device.as_deref() {
                Some(device) => crate::session_sync::remote_pane_id(device, &pane),
                None => pane,
            })
            .collect::<HashSet<_>>();
        self.close_descendants_before_removal(
            &inside,
            outside,
            super::tree_close::TreeFinal::Worktree {
                removal_id: id,
                device,
                checkout_path,
            },
        )
    }

    /// Closes a recorded removal's checkout panes on the close worker, which
    /// then carries the removal out.
    pub(super) fn close_worktree_panes(
        &mut self,
        id: u64,
        device: Option<String>,
        checkout_path: String,
    ) -> bool {
        let pane_ids = self.checkout_pane_ids(device.as_deref(), &checkout_path);
        crate::diagnostic!(serde_json::json!({
            "component": "worktree_removal",
            "kind": "close_requested",
            "target": device.as_deref().unwrap_or(self.node.as_str()),
            "id": id,
            "pane_count": pane_ids.len(),
        }));
        let context = match device.as_deref() {
            Some(device) => self.device_worktree_target(device),
            None => self.local_worktree_target(),
        };
        let context = match context {
            Ok(context) => context,
            Err(message) => {
                return self.ingest_worktree_close_result(
                    id,
                    &[],
                    Err(live::RemovalFailure::new(
                        crate::model::WorktreeRemovalNotice::CloseFailed,
                        format!("Deleting a worktree needs a live Herdr connection: {message}"),
                    )),
                );
            }
        };
        if let Err(message) = live::spawn_worktree_close(context, id, checkout_path, pane_ids) {
            self.ingest_worktree_close_result(
                id,
                &[],
                Err(live::RemovalFailure::new(
                    crate::model::WorktreeRemovalNotice::WorkerUnavailable,
                    message,
                )),
            );
        }
        true
    }

    /// The gate at the moment of confirmation: the main worktree is refused,
    /// and a folder whose loss the operator did not accept is refused with
    /// what changed, since the confirmation they saw may be older than the row.
    fn worktree_removal_allowed(
        &mut self,
        worktree: &crate::model::WorktreeSnapshot,
        payload: &RemoveWorktreePayload,
    ) -> bool {
        if let Some(blocked) = worktree.deletion_gate.blocked.as_ref() {
            self.set_error(
                "worktree.remove_blocked",
                format!("Not deleted: the deletion gate refuses it ({blocked:?})"),
                true,
            );
            return false;
        }
        if worktree.ignored_repositories != payload.expected_ignored_repositories {
            self.set_error("worktree.remove_ignored_changed", "Not deleted: the ignored repository list changed. Refresh and review every repository before choosing Discard.".to_owned(), true);
            return false;
        }
        if let Some(discard) = worktree.deletion_gate.discard.as_ref()
            && !payload.discard_changes
        {
            self.set_error(
                "worktree.remove_discard_unconfirmed",
                format!("Not deleted: deleting it would lose work the operator did not accept losing ({discard:?})"),
                true,
            );
            return false;
        }
        true
    }

    /// Records the confirmed removal the close worker then carries out, with
    /// the operator's choices as the gate allowed them.
    fn begin_worktree_removal(
        &mut self,
        device: Option<&str>,
        repository_root: String,
        protected_base_branch: Option<String>,
        worktree: crate::model::WorktreeSnapshot,
        payload: &RemoveWorktreePayload,
    ) -> u64 {
        let gate = &worktree.deletion_gate;
        let delete_branch = payload.delete_branch && gate.can_delete_branch;
        self.next_worktree_removal_id = self.next_worktree_removal_id.wrapping_add(1).max(1);
        let id = self.next_worktree_removal_id;
        self.snapshot.worktree_removal = Some(crate::model::WorktreeRemovalSnapshot {
            id,
            device_id: device.map(str::to_owned),
            repository_root,
            checkout_path: payload.checkout_path.clone(),
            expected_head_sha: worktree.head_sha.clone(),
            expected_branch: worktree.branch.clone(),
            // A worktree already on the base was confirmed with that warning;
            // the recheck guards only against the branch becoming the base.
            protected_base_branch: protected_base_branch
                .filter(|base| worktree.branch.as_deref() != Some(base.as_str())),
            delete_branch,
            force_delete_branch: delete_branch && gate.branch_loss.is_some(),
            discard_changes: payload.discard_changes && gate.discard.is_some(),
            expected_ignored_repositories: payload.expected_ignored_repositories.clone(),
            branch: worktree.branch,
            phase: "checking".to_owned(),
            notice: None,
        });
        id
    }

    /// One local registration choice; no filesystem or Herdr work under the lock.
    pub(super) fn set_primary_checkout(&mut self, payload: SetPrimaryCheckoutPayload) -> bool {
        let registration_index = self
            .snapshot
            .ui_state
            .workspace_registrations
            .iter()
            .position(|row| row.id == payload.workspace_id);
        let project = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .find(|row| row.id == payload.workspace_id);
        let refusal = match (registration_index, project) {
            (Some(index), _)
                if self.snapshot.ui_state.workspace_registrations[index].device_id
                    != self.node.as_str() =>
            {
                Some("remote_read_only")
            }
            (None, _) => Some("unregistered"),
            (_, None) => Some("missing_workspace"),
            (_, Some(project)) if !project.is_git => Some("plain_folder"),
            (_, Some(project))
                if !project
                    .checkouts
                    .iter()
                    .any(|checkout| checkout.id == payload.checkout_id && checkout.exists) =>
            {
                Some("missing_checkout")
            }
            _ => None,
        };
        if let Some(reason) = refusal {
            self.push_diagnostic(
                format!("workspace.primary_{reason}"),
                format!(
                    "Project {} checkout {}",
                    payload.workspace_id, payload.checkout_id
                ),
            );
            return true;
        }
        if project.is_some_and(|project| {
            project
                .checkouts
                .iter()
                .any(|checkout| checkout.id == payload.checkout_id && checkout.is_primary)
        }) {
            return false;
        }
        let registration = &mut self.snapshot.ui_state.workspace_registrations
            [registration_index.expect("validated registration")];
        if registration.primary_checkout_id.as_ref() == Some(&payload.checkout_id) {
            return false;
        }
        registration.primary_checkout_id = Some(payload.checkout_id.clone());
        for project in self
            .snapshot
            .navigator
            .workspaces
            .iter_mut()
            .chain(self.last_accepted_catalog.iter_mut().flatten())
            .filter(|project| project.id == payload.workspace_id)
        {
            workspace::apply_primary_checkout(project, Some(&payload.checkout_id));
        }
        crate::project_context::sort_projects(
            &mut self.snapshot.navigator.workspaces,
            &self.snapshot.navigator.agents,
        );
        self.refresh_inactive_groups();
        self.persist_ui_state();
        true
    }

    /// Pins or unpins a project (D-07, D-08). The row moves at once and the
    /// registration is persisted on the existing off-lock save; the same
    /// value again changes nothing and writes nothing. A row Herdr shows
    /// without a registration is registered by its pin (PRD
    /// sidebar-context-menus D-14), so pinning it stays one event.
    pub(super) fn set_workspace_pinned(&mut self, payload: WorkspacePinSetPayload) -> bool {
        if !self
            .snapshot
            .ui_state
            .workspace_registrations
            .iter()
            .any(|registration| registration.id == payload.workspace_id)
            && let Some(registration) = self.unregistered_row_registration(&payload.workspace_id)
        {
            if !payload.pinned {
                // Nothing is pinned without a registration: already the target state.
                return false;
            }
            return self.register_pinned(registration);
        }
        let Some(registration) = self
            .snapshot
            .ui_state
            .workspace_registrations
            .iter_mut()
            .find(|registration| registration.id == payload.workspace_id)
        else {
            self.set_error(
                "workspace.pin_unregistered",
                format!(
                    "Project {} is not registered, so it cannot be pinned",
                    payload.workspace_id
                ),
                false,
            );
            return true;
        };
        if registration.pinned == payload.pinned {
            return false;
        }
        registration.pinned = payload.pinned;
        let device = registration.device_id.clone();
        if device != self.node.as_str() {
            // A device's rows are derived from its registrations, so the row
            // moves when its session is derived again.
            self.refresh_device_catalog(&device);
        }
        for workspace in self
            .snapshot
            .navigator
            .workspaces
            .iter_mut()
            .chain(self.last_accepted_catalog.iter_mut().flatten())
            .filter(|workspace| workspace.id == payload.workspace_id)
        {
            workspace.pinned = payload.pinned;
        }
        // Re-sort in place, the way an activity change does: the catalog
        // carries the flag on its next rebuild, but the row moves now.
        let agents = self.snapshot.navigator.agents.clone();
        crate::project_context::sort_projects(&mut self.snapshot.navigator.workspaces, &agents);
        self.refresh_inactive_groups();
        self.persist_ui_state();
        self.push_diagnostic(
            if payload.pinned {
                "workspace.pinned"
            } else {
                "workspace.unpinned"
            },
            format!("Project {}", payload.workspace_id),
        );
        true
    }

    /// The registration a row Herdr shows without one would take: the id,
    /// label, root and device the row already carries, so nothing is read
    /// from disk under the lock and the next catalog rebuild adopts it under
    /// the same id. A device's row qualifies only once its helper grouped it
    /// under its root (`device_catalog::project_id`); before that its id
    /// names one Herdr workspace, which no registration can keep.
    fn unregistered_row_registration(
        &self,
        workspace_id: &str,
    ) -> Option<Result<crate::model::WorkspaceRegistration, String>> {
        let row_registration = |row: &crate::model::WorkspaceSnapshot, device_id: &str| {
            crate::model::WorkspaceRegistration {
                primary_checkout_id: None,
                id: row.id.clone(),
                label: row.label.clone(),
                path: row.path.clone(),
                device_id: device_id.to_owned(),
                pinned: true,
                home: false,
            }
        };
        if let Some(row) =
            self.snapshot.navigator.workspaces.iter().find(|row| {
                row.id == workspace_id && !row.registered && row.remote_target_id.is_none()
            })
        {
            return Some(Ok(row_registration(row, self.node.as_str())));
        }
        self.snapshot.status.remote.iter().find_map(|status| {
            let row = status
                .session
                .as_ref()?
                .workspaces
                .iter()
                .find(|row| row.id == workspace_id && !row.registered)?;
            let grouped =
                crate::device_catalog::project_id(&status.target_id, Path::new(&row.path));
            Some(if grouped == row.id {
                Ok(row_registration(row, &status.target_id))
            } else {
                Err(status.target_id.clone())
            })
        })
    }

    /// Registers a row Herdr shows without a registration and pins it, in
    /// the event that asked for the pin. A device's row its helper has not
    /// grouped yet stays as it is, with the reason in the log: the operator
    /// can pin it again once the device's catalog is ready.
    fn register_pinned(
        &mut self,
        registration: Result<crate::model::WorkspaceRegistration, String>,
    ) -> bool {
        let registration = match registration {
            Ok(registration) => registration,
            Err(device) => {
                self.push_diagnostic(
                    "workspace.pin_ungrouped",
                    format!("A project row on {device} is not grouped by its helper yet, so it was not registered"),
                );
                return true;
            }
        };
        // A removal still closing this row's panes would retire the row
        // this registration keeps; the two requests contradict each other.
        if self.workspace_removals_in_flight.contains(&registration.id) {
            self.push_diagnostic(
                "workspace.pin_during_removal",
                format!(
                    "Project {} is being removed, so it was not pinned",
                    registration.id
                ),
            );
            return true;
        }
        let id = registration.id.clone();
        let device = registration.device_id.clone();
        self.snapshot
            .ui_state
            .workspace_registrations
            .push(registration);
        if device != self.node.as_str() {
            self.refresh_device_catalog(&device);
        }
        for row in self
            .snapshot
            .navigator
            .workspaces
            .iter_mut()
            .chain(self.last_accepted_catalog.iter_mut().flatten())
            .filter(|row| row.id == id)
        {
            row.registered = true;
            row.temporary = false;
            row.pinned = true;
            for checkout in &mut row.checkouts {
                checkout.temporary = false;
            }
        }
        let agents = self.snapshot.navigator.agents.clone();
        crate::project_context::sort_projects(&mut self.snapshot.navigator.workspaces, &agents);
        self.refresh_inactive_groups();
        self.persist_ui_state();
        crate::diagnostic!(serde_json::json!({
            "component": "registration", "kind": "workspace.registered_by_pin",
            "workspace_id": id, "target": device,
        }));
        self.push_diagnostic("workspace.pinned", format!("Project {id}"));
        true
    }

    /// `Remove project…` (D-09, D-11). A project with no pane loses its
    /// registration at once, as before. One with panes has them closed on a
    /// worker thread that waits for Herdr to confirm; only that confirmation
    /// removes the registration, so a timeout leaves the project registered
    /// with the reason in the error banner and a retry starts from whatever
    /// panes remain. A row Herdr shows without a registration (PRD
    /// sidebar-context-menus D-14) has only its panes to close; the row then
    /// leaves with Herdr's workspace.
    pub(super) fn remove_workspace(&mut self, payload: RemoveWorkspacePayload) -> bool {
        let registered = self
            .snapshot
            .ui_state
            .workspace_registrations
            .iter()
            .find(|registration| registration.id == payload.workspace_id)
            .map(|registration| registration.device_id.clone());
        let Some(owner) =
            registered.or_else(|| self.unregistered_row_device(&payload.workspace_id))
        else {
            // Already gone: the target state is reached, and a repeat of a
            // completed removal stays quiet (docs/UI_BEHAVIOR.md, registration removal).
            return false;
        };
        if self
            .workspace_removals_in_flight
            .contains(&payload.workspace_id)
        {
            return false;
        }
        // The mirror of the create-side refusal: a creation still running
        // for this folder will push the registration back after the retire.
        if self.workspace_creation_in_flight_for(&payload.workspace_id) {
            self.set_error(
                "workspace.create_in_flight",
                "This project is still being added; wait for its first pane, then remove it",
                false,
            );
            return true;
        }
        let device = Some(owner).filter(|device| device != self.node.as_str());
        if !payload.close_descendant_pane_ids.is_empty() {
            // The operator's chosen descendants outside the project close
            // first; the removal starts only once they have (PRD
            // close-agent-subtree D-36). The mark keeps a repeat quiet.
            let (_, inside) = self.project_panes(&payload.workspace_id, device.as_deref());
            let inside = inside
                .into_iter()
                .map(|pane| match device.as_deref() {
                    Some(device) => crate::session_sync::remote_pane_id(device, &pane),
                    None => pane,
                })
                .collect::<HashSet<_>>();
            self.workspace_removals_in_flight
                .insert(payload.workspace_id.clone());
            return self.close_descendants_before_removal(
                &inside,
                payload.close_descendant_pane_ids,
                super::tree_close::TreeFinal::Project {
                    workspace_id: payload.workspace_id,
                    device,
                },
            );
        }
        self.close_project_panes(payload.workspace_id, device)
    }

    /// A project's checkout folders and the panes in them, by the ids its
    /// own Herdr uses.
    fn project_panes(
        &self,
        workspace_id: &str,
        device: Option<&str>,
    ) -> (Vec<String>, Vec<String>) {
        // A device's project lists its panes in that device's session, by
        // scoped ids; Herdr there closes them by its own.
        let rows = match device {
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
        rows.iter()
            .filter(|workspace| workspace.id == workspace_id)
            .flat_map(|workspace| &workspace.checkouts)
            .fold(
                (Vec::new(), Vec::new()),
                |(mut paths, mut panes), checkout| {
                    paths.push(checkout.path.clone());
                    panes.extend(checkout.tabs.iter().flat_map(|tab| &tab.panes).filter_map(
                        |pane| match device {
                            Some(device) => {
                                super::remote_pane_source_id(device, &pane.id).map(str::to_owned)
                            }
                            None => Some(pane.id.clone()),
                        },
                    ));
                    (paths, panes)
                },
            )
    }

    /// Closes a project's panes on the close worker, which answers whether
    /// Herdr confirmed them gone; only that removes the registration.
    pub(super) fn close_project_panes(
        &mut self,
        workspace_id: String,
        device: Option<String>,
    ) -> bool {
        let (checkout_paths, pane_ids) = self.project_panes(&workspace_id, device.as_deref());
        if pane_ids.is_empty() {
            return self.retire_workspace_registration(&workspace_id);
        }
        crate::diagnostic!(serde_json::json!({
            "component": "registration", "kind": "remove.close_requested",
            "workspace_id": workspace_id, "pane_ids": pane_ids,
            "target": device.as_deref().unwrap_or(self.node.as_str()),
        }));
        let context = match device.as_deref() {
            Some(device) => self.device_worktree_target(device),
            None => self.local_worktree_target(),
        };
        let context = match context {
            Ok(context) => context,
            Err(message) => {
                self.set_error(
                    "workspace.remove_failed",
                    format!("Removing a project with open panes needs its device's Herdr connection: {message}"),
                    true,
                );
                return true;
            }
        };
        self.workspace_removals_in_flight
            .insert(workspace_id.clone());
        if let Err(message) =
            live::spawn_workspace_close(context, workspace_id.clone(), checkout_paths, pane_ids)
        {
            self.ingest_workspace_close_result(&workspace_id, Err(message));
        }
        true
    }

    /// The device of a row Herdr shows without a registration, when one has
    /// this id: this machine's navigator first, then each device's session.
    fn unregistered_row_device(&self, workspace_id: &str) -> Option<String> {
        if self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .any(|row| row.id == workspace_id && !row.registered && row.remote_target_id.is_none())
        {
            return Some(self.node.as_str().to_owned());
        }
        self.snapshot.status.remote.iter().find_map(|status| {
            status
                .session
                .as_ref()?
                .workspaces
                .iter()
                .any(|row| row.id == workspace_id && !row.registered)
                .then(|| status.target_id.clone())
        })
    }

    /// The worker's answer to `remove_workspace`: Herdr confirmed every pane
    /// gone, or it did not in time. Only the first removes the registration.
    pub fn ingest_workspace_close_result(
        &mut self,
        workspace_id: &str,
        result: Result<(), String>,
    ) -> bool {
        if !self.workspace_removals_in_flight.remove(workspace_id) {
            return false;
        }
        match result {
            Ok(()) => self.retire_workspace_registration(workspace_id),
            Err(message) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "registration", "kind": "remove.close_failed",
                    "workspace_id": workspace_id, "error": message,
                }));
                let registered = self
                    .snapshot
                    .ui_state
                    .workspace_registrations
                    .iter()
                    .any(|registration| registration.id == workspace_id);
                self.set_error(
                    "workspace.remove_failed",
                    if registered {
                        format!("{message}. The project stays registered; remove it again to close the panes that remain.")
                    } else {
                        format!("{message}. Remove the project again to close the panes that remain.")
                    },
                    true,
                );
                true
            }
        }
    }

    /// The registration id a removal is still closing panes for, when
    /// `path` names that same folder. Ids are path-keyed, so the folder and
    /// the registration cannot be told apart by id alone. The registration is
    /// still listed while the close runs, so its stored path is the
    /// comparison; nothing is resolved on disk under the lock.
    pub(super) fn workspace_removal_in_flight_for(&self, path: &str) -> Option<String> {
        if self.workspace_removals_in_flight.is_empty() {
            return None;
        }
        let requested = Path::new(path);
        self.snapshot
            .ui_state
            .workspace_registrations
            .iter()
            // Only this machine's registrations: a device's project at the
            // same absolute path is another folder (B2).
            .filter(|registration| registration.device_id == self.node.as_str())
            .filter(|registration| self.workspace_removals_in_flight.contains(&registration.id))
            .find(|registration| Path::new(&registration.path) == requested)
            .map(|registration| registration.id.clone())
    }

    /// Whether a creation still running names the folder `workspace_id`
    /// registers; creation is keyed by the requested path, not the id.
    pub(super) fn workspace_creation_in_flight_for(&self, workspace_id: &str) -> bool {
        if self.workspace_creations_in_flight.is_empty() {
            return false;
        }
        self.snapshot
            .ui_state
            .workspace_registrations
            .iter()
            // Creations in flight are this machine's folders only (B2).
            .filter(|registration| {
                registration.id == workspace_id && registration.device_id == self.node.as_str()
            })
            .any(|registration| {
                self.workspace_creations_in_flight
                    .iter()
                    .any(|path| Path::new(path) == Path::new(&registration.path))
            })
    }

    /// Drops the registration and its row. Files, worktrees and Herdr
    /// workspaces are never touched here.
    pub(super) fn retire_workspace_registration(&mut self, workspace_id: &str) -> bool {
        let local_path = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .find(|workspace| workspace.id == workspace_id && workspace.remote_target_id.is_none())
            .map(|workspace| workspace.path.clone());
        let device = self
            .snapshot
            .ui_state
            .workspace_registrations
            .iter()
            .find(|registration| registration.id == workspace_id)
            .map(|registration| registration.device_id.clone())
            .filter(|device| device != self.node.as_str());
        let registered = self
            .snapshot
            .ui_state
            .workspace_registrations
            .iter()
            .find(|registration| registration.id == workspace_id)
            .map(|registration| (registration.device_id.clone(), registration.path.clone()));
        let before = self.snapshot.ui_state.workspace_registrations.len();
        self.snapshot
            .ui_state
            .workspace_registrations
            .retain(|registration| registration.id != workspace_id);
        if before == self.snapshot.ui_state.workspace_registrations.len() {
            return false;
        }
        if let Some((device_id, path)) = &registered {
            self.forget_project_guardrails(device_id, path);
        }
        let asleep = crate::project_context::asleep_fold_key(workspace_id);
        self.snapshot
            .ui_state
            .session_open_folds
            .retain(|key| key != workspace_id && key != &asleep);
        // The project's checkouts leave the recent list with it; their ids
        // are read before the catalog drops the rows that name them. A
        // device that is not connected has no rows to read, so the
        // registered folder's own checkout is named by its folder as well.
        let mut checkout_ids: Vec<String> = self
            .catalog_workspaces()
            .filter(|workspace| workspace.id == workspace_id)
            .flat_map(|workspace| &workspace.checkouts)
            .map(|checkout| checkout.id.clone())
            .collect();
        if let Some((device_id, path)) =
            registered.filter(|(device, _)| device != self.node.as_str())
        {
            checkout_ids.push(crate::device_catalog::checkout_id(&device_id, &path));
        }
        self.forget_recent_checkouts(|held| checkout_ids.contains(&held.checkout_id));
        if let Some(path) = local_path {
            self.worktree_project_generations.remove(&path);
        }
        if let Some(device) = device {
            self.refresh_device_catalog(&device);
        }
        // The focused checkout and the selected pane leave with the project;
        // kept, they would name a checkout no catalog carries and the sync
        // would report `pane.projection_unavailable` every tick instead of
        // moving to the next project, exactly as after a Herdr restart
        // (`consume_restore_hint`).
        let focus_leaves_with_workspace = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .filter(|workspace| workspace.id == workspace_id)
            .flat_map(|workspace| workspace.checkouts.iter())
            .any(|checkout| {
                Some(checkout.id.as_str()) == self.snapshot.navigator.focused_checkout_id.as_deref()
            });
        // Retire the accepted projection directly. Rebuilding the
        // filesystem catalog here ran git while holding the mutex.
        self.snapshot
            .navigator
            .workspaces
            .retain(|workspace| workspace.id != workspace_id);
        if focus_leaves_with_workspace {
            self.snapshot.ui_state.focused_checkout_id = None;
            self.snapshot.navigator.focused_checkout_id = None;
            self.snapshot.ui_state.selected_pane_id = None;
            self.snapshot.terminal.pane_id = None;
            self.snapshot.focused.pane_id = None;
            self.clear_selected_terminal_state();
            if self
                .snapshot
                .status
                .last_error
                .as_ref()
                .is_some_and(|error| error.kind == "pane.projection_unavailable")
            {
                self.snapshot.status.last_error = None;
            }
        }
        if let Some(catalog) = &mut self.last_accepted_catalog {
            catalog.retain(|workspace| workspace.id != workspace_id);
        }
        self.snapshot
            .ui_state
            .collapsed_workspace_ids
            .retain(|id| id != workspace_id);
        // The sidebar's lists narrowed to the project show every project again.
        if self
            .snapshot
            .ui_state
            .sidebar_project
            .as_ref()
            .is_some_and(|project| project.workspace_id == workspace_id)
        {
            self.snapshot.ui_state.sidebar_project = None;
        }
        self.refresh_inactive_groups();
        self.resync_navigator_focus();
        self.persist_current_ui_state();
        self.push_diagnostic(
            "workspace.unregistered",
            format!("Unregistered workspace {workspace_id} without touching its files"),
        );
        true
    }

    /// `closed_pane_ids` are the panes Herdr confirmed gone; the navigator can
    /// still list them until Herdr's close events are applied, so only a pane
    /// outside that set counts as one that appeared during the confirmation.
    pub fn ingest_worktree_close_result(
        &mut self,
        id: u64,
        closed_pane_ids: &[String],
        result: Result<(), live::RemovalFailure>,
    ) -> bool {
        let Some(active) = self.snapshot.worktree_removal.as_ref() else {
            return false;
        };
        if active.id != id || active.phase != "closing" {
            return false;
        }
        let mut result = result;
        if result.is_ok() {
            let device = active.device_id.as_deref();
            let catalog = match device {
                Some(device) => self
                    .device_worktrees
                    .get(device)
                    .map(|worktrees| worktrees.projects.values().collect::<Vec<_>>())
                    .unwrap_or_default(),
                None => self.worktree_catalog.projects.iter().collect(),
            };
            // The catalog is the reader's latest answer. Closing a checkout's
            // last pane can end the only workspace that asked for its
            // repository, and the next answer then has no project for it,
            // which says nothing about the worktree, so only a repository the
            // reader still answers for is compared. The host checks the
            // registration, HEAD and branch again before it removes anything.
            let identity_changed = catalog
                .into_iter()
                .find(|project| project.root_path == active.repository_root)
                .is_some_and(|project| {
                    project
                        .worktrees
                        .iter()
                        .find(|worktree| worktree.path == active.checkout_path)
                        .is_none_or(|worktree| {
                            worktree.head_sha != active.expected_head_sha
                                || worktree.branch != active.expected_branch
                                || worktree.deletion_gate.blocked.is_some()
                        })
                });
            // The close worker names a device's panes by Herdr's own ids.
            let workspaces = match device {
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
            let pane_reappeared = workspaces
                .iter()
                .flat_map(|workspace| &workspace.checkouts)
                .filter(|checkout| checkout.path == active.checkout_path)
                .flat_map(|checkout| &checkout.tabs)
                .flat_map(|tab| &tab.panes)
                .any(|pane| {
                    let id = match device {
                        Some(device) => super::remote_pane_source_id(device, &pane.id),
                        None => Some(pane.id.as_str()),
                    };
                    id.is_none_or(|id| !closed_pane_ids.iter().any(|closed| closed == id))
                });
            if identity_changed || pane_reappeared {
                result = Err(if pane_reappeared {
                    live::RemovalFailure::new(
                        crate::model::WorktreeRemovalNotice::PaneAppeared,
                        "A pane appeared in the worktree while deletion was being confirmed",
                    )
                } else {
                    live::RemovalFailure::new(
                        crate::model::WorktreeRemovalNotice::ChangedWhileClosing,
                        "The worktree identity or deletion gate changed while panes were closing",
                    )
                });
            }
        }
        let removal = self.snapshot.worktree_removal.as_mut().unwrap();
        match result {
            Ok(()) => {
                removal.phase = "removing".to_owned();
                removal.notice = None;
            }
            Err(failure) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "worktree_removal", "kind": "close_failed",
                    "id": id, "detail": failure.detail,
                }));
                removal.phase = "failed".to_owned();
                removal.notice = Some(failure.notice);
            }
        }
        true
    }

    pub fn ingest_worktree_open_result(
        &mut self,
        checkout_path: String,
        result: Result<(), String>,
    ) -> bool {
        let message = result.err();
        let mut changed = false;
        for project in &mut self.worktree_catalog.projects {
            if let Some(worktree) = project
                .worktrees
                .iter_mut()
                .find(|worktree| worktree.path == checkout_path)
                && worktree.open_error != message
            {
                worktree.open_error = message.clone();
                changed = true;
            }
        }
        for workspace in &mut self.snapshot.navigator.workspaces {
            for checkout in &mut workspace.checkouts {
                if checkout.path == checkout_path
                    && let Some(worktree) = checkout.worktree.as_mut()
                    && worktree.open_error != message
                {
                    worktree.open_error = message.clone();
                    changed = true;
                }
            }
        }
        if message.is_some() {
            self.refresh_worktrees();
        }
        changed
    }

    /// The removal the close worker may now execute: the active request, once
    /// every pane is confirmed gone and the identity still matched.
    pub(crate) fn confirmed_worktree_removal(
        &self,
        id: u64,
    ) -> Option<hide_node_link::worktrees::ConfirmedRemoval> {
        self.worktree_removal_request(id, "removing")
    }

    pub(crate) fn worktree_preflight_request(
        &self,
        id: u64,
    ) -> Option<hide_node_link::worktrees::ConfirmedRemoval> {
        self.worktree_removal_request(id, "checking")
    }

    fn worktree_removal_request(
        &self,
        id: u64,
        phase: &str,
    ) -> Option<hide_node_link::worktrees::ConfirmedRemoval> {
        let removal = self.snapshot.worktree_removal.as_ref()?;
        (removal.id == id && removal.phase == phase).then(|| {
            hide_node_link::worktrees::ConfirmedRemoval {
                repository_root: removal.repository_root.clone(),
                checkout_path: removal.checkout_path.clone(),
                expected_head_sha: removal.expected_head_sha.clone(),
                expected_branch: removal.expected_branch.clone(),
                protected_base_branch: removal.protected_base_branch.clone(),
                delete_branch: removal
                    .delete_branch
                    .then(|| removal.branch.clone())
                    .flatten(),
                force_delete_branch: removal.force_delete_branch,
                discard_changes: removal.discard_changes,
                expected_ignored_repositories: removal.expected_ignored_repositories.clone(),
            }
        })
    }

    /// Settles the active removal with what Git did. Only the worker that ran
    /// the removal calls this, so a result can never name another request.
    pub(crate) fn ingest_worktree_removal_result(
        &mut self,
        id: u64,
        result: Result<hide_node_link::worktrees::Removed, live::RemovalFailure>,
    ) -> bool {
        let Some(removal) = self
            .snapshot
            .worktree_removal
            .as_mut()
            .filter(|removal| removal.id == id && removal.phase == "removing")
        else {
            crate::diagnostic!(serde_json::json!({
                "component": "worktree_removal",
                "kind": "result_without_request",
                "id": id,
            }));
            return false;
        };
        let (phase, notice, detail) = match result {
            Ok(removed) => (
                "finished",
                crate::model::WorktreeRemovalNotice::Removed {
                    branch: removed.branch,
                },
                removed.message,
            ),
            Err(failure) => ("failed", failure.notice, failure.detail),
        };
        crate::diagnostic!(serde_json::json!({
            "component": "worktree_removal", "kind": "removal_answered",
            "id": id, "phase": phase, "detail": detail,
        }));
        removal.phase = phase.to_owned();
        removal.notice = Some(notice);
        let checkout_path = removal.checkout_path.clone();
        let removal_device = removal.device_id.clone();
        if phase == "finished" {
            // The removed checkout leaves the recent list in the frame its
            // row leaves the catalog.
            let device = removal_device
                .clone()
                .unwrap_or_else(|| self.node.as_str().to_owned());
            let gone = self.catalog_checkout_ids_at(&device, &checkout_path);
            self.forget_recent_checkouts(|held| gone.contains(&held.checkout_id));
        }
        match removal_device {
            Some(device) => {
                self.request_device_worktrees(&device, true);
            }
            // The row leaves in this same frame. Git dropped the registration
            // in the repository's own Git directory, which is that project's
            // freshness key, so the reader re-reads that project alone; every
            // other project keeps its last answer.
            None if phase == "finished" => self.drop_removed_worktree(checkout_path),
            // A refused removal re-reads too: whatever stopped it (a moved
            // HEAD, a dirty file) is news the catalog should show.
            None => self.refresh_worktrees(),
        }
        true
    }

    /// Takes a removed checkout out of the catalog and the rows built on it,
    /// and keeps it out of any read that started before the removal settled.
    fn drop_removed_worktree(&mut self, checkout_path: String) {
        self.worktree_removals = self.worktree_removals.wrapping_add(1);
        for project in &mut self.worktree_catalog.projects {
            project
                .worktrees
                .retain(|worktree| worktree.path != checkout_path);
        }
        self.removed_worktrees
            .push((self.worktree_removals, checkout_path));
        self.refresh_worktree_projection();
    }

    /// This machine's Herdr and file host, for a worktree task here.
    pub(super) fn local_worktree_target(&self) -> Result<live::WorktreeTarget, String> {
        self.live
            .as_ref()
            .map(|context| live::WorktreeTarget::local(context, Arc::clone(&self.own_node)))
            .ok_or_else(|| "A live Herdr connection is required".to_owned())
    }

    /// A device's Herdr and file helper, for a worktree task there. Both
    /// have to be up: Herdr creates and closes the panes, the helper checks
    /// and removes on the device's disk.
    pub(super) fn device_worktree_target(
        &mut self,
        device: &str,
    ) -> Result<live::WorktreeTarget, String> {
        let control = self
            .remote_controls
            .get(device)
            .cloned()
            .ok_or_else(|| "The device's Herdr connection is unavailable".to_owned())?;
        let host = self.node_link(device)?;
        Ok(live::WorktreeTarget::device(&control, host))
    }

    pub fn disk_request(&self) -> crate::disk::DiskRequest {
        let mut shared_git = Vec::new();
        let paths = if self.snapshot.ui_state.right_panel_visible
            && matches!(
                self.snapshot.ui_state.right_panel_section,
                RightPanelSection::Overview
            ) {
            self.focused_local_checkout()
                .and_then(|(workspace, _)| {
                    // A folder project has no worktree list, and it still has
                    // a size the Overview states (PRD B16): the folder itself.
                    if !workspace.is_git {
                        return Some(vec![PathBuf::from(&workspace.path)]);
                    }
                    let project = self.worktree_catalog.project(&workspace.path)?;
                    let mut paths: Vec<_> = project
                        .worktrees
                        .iter()
                        .map(|w| PathBuf::from(&w.path))
                        .collect();
                    if let Some(shared) = &project.shared_git_path {
                        paths.push(PathBuf::from(shared));
                        shared_git.push(PathBuf::from(shared));
                    }
                    Some(paths)
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        crate::disk::DiskRequest {
            paths,
            shared_git,
            generation: self.disk_generation,
        }
    }

    /// The repositories of node `device_id` read for GitHub, as (root,
    /// `owner/name`) in path order, so the request does not move with the
    /// focus: those whose `origin` names one, at most `GITHUB_PROJECT_LIMIT`,
    /// the one in front of the Overview first past it. The second value is
    /// how many the node names in all.
    pub(super) fn node_github_repositories(
        &self,
        device_id: &str,
    ) -> (Vec<(String, String)>, usize) {
        let Some(listed) = self.device_worktrees.get(device_id) else {
            return (Vec::new(), 0);
        };
        let mut repositories: Vec<(String, String)> = listed
            .projects
            .iter()
            .filter_map(|(root, project)| {
                let explicit = crate::project_settings::get(
                    &self.snapshot.ui_state.project_settings,
                    device_id,
                    root,
                )
                .and_then(|record| match record.repository.as_ref() {
                    Some(crate::project_settings::RepositoryConnection::Repository {
                        repository,
                    }) => Some(repository.clone()),
                    _ => None,
                });
                Some((
                    root.clone(),
                    explicit.or_else(|| project.repository.clone())?,
                ))
            })
            .collect();
        let total = repositories.len();
        let in_front = self
            .focused_node_checkout()
            .filter(|(node, _, _)| *node == device_id)
            .map(|(_, workspace, _)| workspace.path.as_str());
        repositories.sort_by(|left, right| {
            (Some(left.0.as_str()) != in_front)
                .cmp(&(Some(right.0.as_str()) != in_front))
                .then_with(|| left.cmp(right))
        });
        repositories.truncate(GITHUB_PROJECT_LIMIT);
        repositories.sort();
        (repositories, total)
    }

    /// The repositories of a node that dials this core whose `origin` names
    /// a GitHub repository, read with this machine's login by that name
    /// (D-16): each once, then again every five minutes, as this machine's
    /// are. Past the limit the one in front of the Overview is read first.
    pub(crate) fn device_github_request(
        &mut self,
        device_id: &str,
        now: std::time::Instant,
    ) -> crate::github::GithubRequest {
        let (repositories, total) = self.node_github_repositories(device_id);
        let over_limit = total.saturating_sub(GITHUB_PROJECT_LIMIT);
        let logged = self
            .device_github_over_limit
            .insert(device_id.to_owned(), over_limit);
        if over_limit > 0 && logged != Some(over_limit) {
            crate::diagnostic!(serde_json::json!({
                "component": "github",
                "kind": "projects.over_limit",
                "target": device_id,
                "limit": GITHUB_PROJECT_LIMIT,
                "device_git_projects": total,
            }));
        }
        let listed = |(device, root): &(String, String)| {
            device != device_id || repositories.iter().any(|(listed, _)| listed == root)
        };
        self.device_github_reads.retain(|key, _| listed(key));
        self.device_github_read_generation
            .retain(|key, _| listed(key));
        let answered = self.device_github.get(device_id);
        // The links the node's rows were derived with; a wake reads them and
        // derives nothing.
        let mut links = self
            .snapshot
            .status
            .remote
            .iter()
            .find(|status| status.target_id == device_id)
            .and_then(|status| status.session.as_ref())
            .map(|session| session.issue_links.clone())
            .unwrap_or_default();
        for workspace in self
            .snapshot
            .status
            .remote
            .iter()
            .find(|status| status.target_id == device_id)
            .and_then(|status| status.session.as_ref())
            .into_iter()
            .flat_map(|session| session.workspaces.iter())
        {
            if let Some(summary) = self
                .snapshot
                .link_summaries
                .as_ref()
                .and_then(|summaries| summaries.projects.get(&workspace.id))
            {
                let held = links.entry(workspace.path.clone()).or_default();
                held.extend(summary.record_issues.iter().cloned());
                held.sort();
                held.dedup();
                held.truncate(crate::issues::ISSUE_LIMIT);
            }
        }
        let projects = repositories
            .into_iter()
            .map(|(root, repository)| {
                // A read whose pull requests failed is asked again sooner, as
                // this machine's first retry is; failed issues alone keep the
                // full wait, since this clock does not climb.
                let failed = answered
                    .and_then(|github| github.project(&root))
                    .is_some_and(|read| read.read_outcome() == GithubReadOutcome::Failed);
                let wait = if failed {
                    GITHUB_RETRY_FIRST
                } else {
                    GITHUB_REREAD
                };
                let links = links.remove(&root).unwrap_or_default();
                let read = self
                    .device_github_reads
                    .entry((device_id.to_owned(), root.clone()))
                    .or_insert_with(|| DeviceGithubRead {
                        generation: 0,
                        asked_at: now,
                        links: links.clone(),
                    });
                if now.duration_since(read.asked_at) >= wait || read.links != links {
                    read.generation = read.generation.wrapping_add(1);
                    read.asked_at = now;
                    read.links = links.clone();
                }
                crate::github::GithubProjectRequest {
                    links,
                    root: PathBuf::from(&root),
                    repository: Some(repository),
                    generation: read.generation,
                    remotes: None,
                }
            })
            .collect();
        crate::github::GithubRequest { projects }
    }

    /// Asks a node's repository at `root` on the next read, whatever its
    /// wait; answers the generation that read carries.
    pub(super) fn refresh_device_pull_requests(&mut self, device_id: &str, root: &str) -> u64 {
        let now = std::time::Instant::now();
        let read = self
            .device_github_reads
            .entry((device_id.to_owned(), root.to_owned()))
            .or_insert_with(|| DeviceGithubRead {
                generation: 0,
                asked_at: now,
                links: Vec::new(),
            });
        read.generation = read.generation.wrapping_add(1);
        read.asked_at = now;
        read.generation
    }

    /// The generation of the last answered read of a node's repository.
    pub(super) fn device_github_read(&self, device_id: &str, root: &str) -> Option<u64> {
        self.device_github_read_generation
            .get(&(device_id.to_owned(), root.to_owned()))
            .copied()
    }

    /// The pull requests and issues of a node's repositories, read for
    /// `request`: its rows carry them, as this machine's do, and a failed
    /// read keeps the previous ones.
    pub(crate) fn ingest_device_github(
        &mut self,
        device_id: &str,
        request: &crate::github::GithubRequest,
        github: crate::model::GithubSnapshot,
    ) -> bool {
        if !self
            .link_origin(device_id)
            .is_some_and(LinkOrigin::reports_own_facts)
        {
            return false;
        }
        let (repositories, _) = self.node_github_repositories(device_id);
        if request.projects.iter().any(|project| {
            let root = project.root.to_string_lossy();
            let generation = self
                .device_github_reads
                .get(&(device_id.to_owned(), root.to_string()))
                .map(|read| read.generation)
                .unwrap_or(0);
            project.generation != generation
                || !repositories.iter().any(|(path, repository)| {
                    path == root.as_ref() && project.repository.as_ref() == Some(repository)
                })
        }) {
            return false;
        }
        for project in &request.projects {
            self.device_github_read_generation.insert(
                (
                    device_id.to_owned(),
                    project.root.to_string_lossy().into_owned(),
                ),
                project.generation,
            );
        }
        let mut merged = github;
        if let Some(previous) = self.device_github.get(device_id) {
            keep_previous_reads(&mut merged, previous);
        }
        self.note_mission_pulls(device_id, &merged);
        if self.device_github.get(device_id) == Some(&merged) {
            return false;
        }
        self.device_github.insert(device_id.to_owned(), merged);
        self.feed_suggestion_prs();
        self.track_suggestion_attempts();
        self.refresh_device_catalog(device_id)
    }

    /// What a node that dials this core measures on its own machine: the
    /// worktrees of its project in front of the Overview and of the one a
    /// web Overview named, as `disk_request` asks of this machine's.
    pub(crate) fn device_disk_request(&self, device_id: &str) -> crate::disk::DiskRequest {
        let mut roots = Vec::new();
        if self.snapshot.ui_state.right_panel_visible
            && matches!(
                self.snapshot.ui_state.right_panel_section,
                RightPanelSection::Overview
            )
            && let Some(focused) = self.snapshot.navigator.focused_checkout_id.as_deref()
            && let Some(workspace) = self
                .snapshot
                .status
                .remote
                .iter()
                .filter(|status| status.target_id == device_id)
                .filter_map(|status| status.session.as_ref())
                .flat_map(|session| session.workspaces.iter())
                .find(|workspace| {
                    workspace
                        .checkouts
                        .iter()
                        .any(|checkout| checkout.id == focused)
                })
        {
            roots.push(workspace.path.clone());
        }
        let mut paths = Vec::new();
        let mut shared_git = Vec::new();
        if let Some(listed) = self.device_worktrees.get(device_id) {
            for project in roots.iter().filter_map(|root| listed.projects.get(root)) {
                paths.extend(project.worktrees.iter().map(|w| PathBuf::from(&w.path)));
                if let Some(shared) = project.shared_git_path.as_ref().map(PathBuf::from) {
                    paths.push(shared.clone());
                    shared_git.push(shared);
                }
            }
        }
        paths.sort();
        paths.dedup();
        shared_git.sort();
        shared_git.dedup();
        crate::disk::DiskRequest {
            paths,
            shared_git,
            generation: self.disk_generation,
        }
    }

    /// A node that dials this core measured its own projects: their rows
    /// carry the sizes, as this machine's do.
    pub(crate) fn ingest_device_disk_usage(
        &mut self,
        device_id: &str,
        disk: Vec<crate::model::DiskUsageSnapshot>,
    ) -> bool {
        if !self
            .link_origin(device_id)
            .is_some_and(LinkOrigin::reports_own_facts)
            || !self.merge_device_disk(device_id, disk)
        {
            return false;
        }
        self.refresh_device_catalog(device_id)
    }

    /// The lane card's rows, laid over the Projects tab's by path.
    pub fn ingest_disk_usage(&mut self, disk: Vec<crate::model::DiskUsageSnapshot>) -> bool {
        if !self.merge_local_disk(disk) {
            return false;
        }
        self.refresh_worktree_projection();
        true
    }

    pub fn remeasure_disk(&mut self) {
        self.disk_generation = self.disk_generation.wrapping_add(1);
        self.refresh_card();
    }

    pub(super) fn focused_local_checkout(&self) -> Option<(&WorkspaceSnapshot, &CheckoutSnapshot)> {
        let focused_checkout_id = self.snapshot.navigator.focused_checkout_id.as_deref()?;
        self.snapshot
            .navigator
            .workspaces
            .iter()
            .filter(|workspace| workspace.remote_target_id.is_none())
            .find_map(|workspace| {
                workspace
                    .checkouts
                    .iter()
                    .find(|checkout| checkout.id == focused_checkout_id)
                    .map(|checkout| (workspace, checkout))
            })
    }

    /// The checkout in front of the Overview when it is one of a node that
    /// dials this core, with that node's id: its Overview reads and cleans
    /// up on the node as one of this machine's does here (D-29).
    pub(super) fn focused_node_checkout(
        &self,
    ) -> Option<(&str, &WorkspaceSnapshot, &CheckoutSnapshot)> {
        let focused_checkout_id = self.snapshot.navigator.focused_checkout_id.as_deref()?;
        self.node_sessions().find_map(|(node, session)| {
            session.workspaces.iter().find_map(|workspace| {
                workspace
                    .checkouts
                    .iter()
                    .find(|checkout| checkout.id == focused_checkout_id)
                    .map(|checkout| (node, workspace, checkout))
            })
        })
    }

    /// Reads the project in front of the Overview again, on whichever
    /// machine it is: its GitHub answer, its worktrees and its sizes.
    pub(super) fn reread_focused_project(&mut self) {
        // A node's project is read on the node; this machine's worktrees and
        // disk have nothing to say about it.
        if let Some((node, workspace, _)) = self.focused_node_checkout() {
            let (node, root) = (node.to_owned(), workspace.path.clone());
            self.reread_node_github(&node, &root);
            self.request_device_worktrees(&node, true);
            return;
        }
        if let Some((workspace, _)) = self.focused_local_checkout() {
            let project = workspace.path.clone();
            self.refresh_pull_requests(&project);
        }
        self.refresh_worktrees();
        self.remeasure_disk();
    }

    pub(super) fn apply_pull_requests(&mut self) -> bool {
        self.mission_pulls_moved();
        let mut changed = false;
        let github = self.github.clone();
        let awaiting: HashSet<String> = self
            .github_projects()
            .0
            .into_iter()
            .map(|workspace| workspace.path.clone())
            .filter(|path| !self.github_settled.contains(path))
            .collect();
        let link_summaries = self
            .snapshot
            .link_summaries
            .as_ref()
            .map(|summaries| &summaries.projects);
        for workspace in self.snapshot.navigator.workspaces.iter_mut() {
            let project = github.project(&workspace.path);
            let status = project
                .map(|project| project.status.clone())
                .unwrap_or_else(|| crate::model::GithubStatusSnapshot {
                    loading: awaiting.contains(&workspace.path),
                    ..Default::default()
                });
            let mut home_issues = project
                .map(|project| project.issues.clone())
                .unwrap_or_default();
            if let Some(crate::project_settings::RepositoryConnection::Repository { repository }) =
                crate::project_settings::get(
                    &self.snapshot.ui_state.project_settings,
                    self.node.as_str(),
                    &workspace.path,
                )
                .and_then(|record| record.repository.as_ref())
            {
                home_issues.repository = Some(repository.clone());
            }
            if workspace.home_issues != home_issues {
                workspace.home_issues = home_issues;
                changed = true;
            }
            let mut pull_requests = match project {
                Some(project) if workspace.is_git && workspace.remote_target_id.is_none() => {
                    super::pull_requests::shown_pull_requests(
                        &project.pull_requests,
                        &workspace.checkouts,
                        unix_milliseconds(),
                    )
                }
                _ => Vec::new(),
            };
            // The sessions are attached before the comparison, so a pass that
            // read nothing new compares equal and publishes nothing.
            super::links::associate_pr_sessions(
                &mut pull_requests,
                link_summaries.and_then(|summaries| summaries.get(&workspace.id)),
            );
            if workspace.pull_requests != pull_requests {
                workspace.pull_requests = pull_requests;
                changed = true;
            }
            for checkout in workspace.checkouts.iter_mut() {
                if checkout.github != status {
                    checkout.github = status.clone();
                    changed = true;
                }
            }
            changed |= associate_pull_requests(workspace, project);
            changed |= super::links::associate_landed(
                workspace,
                link_summaries.and_then(|summaries| summaries.get(&workspace.id)),
            );
        }
        let times = pull_request_times(&github);
        if *self.pull_request_times != times {
            // A sighted address the answer now holds needs no place kept.
            self.github_sighted
                .retain(|address, _| !times.contains_key(address));
            self.pull_request_times = std::sync::Arc::new(times);
            if let Some(services) = self.label_services.as_ref() {
                services.wake_local();
            }
        }
        changed |= crate::sidebar::sync_checkout_purposes(
            &mut self.snapshot.navigator.workspaces,
            &self.snapshot.navigator.agents,
        );
        changed |= self.sync_issues();
        changed |= self.sync_tasks();
        changed |= self.sync_request_rows();
        self.feed_links();
        changed
    }

    pub fn refresh_card(&mut self) -> bool {
        let card = self.projected_card();
        if self.snapshot.card == card {
            return false;
        }
        let retired = self
            .snapshot
            .card
            .panes
            .iter()
            .filter(|old| {
                !card.panes.iter().any(|new| {
                    new.pane_id == old.pane_id && new.parent_pane_id == old.parent_pane_id
                })
            })
            .count();
        if retired > 0 {
            crate::diagnostic!(serde_json::json!({
                "component": "checkout_context", "kind": "context.retired", "count": retired,
            }));
        }
        self.snapshot.card = card;
        true
    }

    pub(super) fn projected_card(&self) -> crate::model::CheckoutCardSnapshot {
        if let Some((workspace, checkout)) = self.focused_local_checkout() {
            let github = if workspace.is_git {
                self.github
                    .project(&workspace.path)
                    .map(|project| project.status.clone())
                    // An absent answer is loading until the first read of this
                    // project has answered; a project the core does not read
                    // (past the limit) has no work in flight.
                    .unwrap_or(crate::model::GithubStatusSnapshot {
                        loading: self.github_awaiting(&workspace.path),
                        ..crate::model::GithubStatusSnapshot::default()
                    })
            } else {
                crate::model::GithubStatusSnapshot::default()
            };
            let panes = crate::project_context::checkout_panes(
                checkout,
                &self.snapshot.navigator.workspaces,
                &self.snapshot.navigator.agents,
            );
            return checkout_card(checkout, github, &self.disk_usage, panes);
        }
        // A node's checkout: its GitHub state is the one its rows carry, its
        // sizes the node measured, its panes those of the node's session.
        if let Some((node, _, checkout)) = self.focused_node_checkout()
            && let Some((_, session)) = self.node_sessions().find(|(id, _)| *id == node)
        {
            let panes = crate::project_context::checkout_panes(
                checkout,
                &session.workspaces,
                &session.agents,
            );
            let disk = self
                .device_disk
                .get(node)
                .map(Vec::as_slice)
                .unwrap_or_default();
            return checkout_card(checkout, checkout.github.clone(), disk, panes);
        }
        crate::model::CheckoutCardSnapshot::default()
    }

    pub(super) fn create_project_worktree(&mut self, payload: CreateWorktreePayload) -> bool {
        let request_id = payload.request_id.clone();
        let before_id = self.snapshot.task_operation.as_ref().map(|task| task.id);
        let before_error = self.snapshot.status.last_error.clone();
        let changed = self.start_worktree_task(payload, false);
        if let Some(request_id) = request_id {
            if let Some(operation) = self.snapshot.task_operation.as_mut()
                && Some(operation.id) != before_id
            {
                operation.request_id = Some(request_id);
            } else if self.snapshot.status.last_error != before_error
                && let Some(error) = self.snapshot.status.last_error.as_mut()
            {
                error.request_id = Some(request_id);
            }
        }
        changed
    }

    /// `create_worktree`, on a new branch or, `existing_branch`, on a branch
    /// that already exists in the repository or on its `origin` (a pull
    /// request's, PRD overview-lenses-prs D-46), on this machine or a node's
    /// through its link and its Herdr (D-29).
    pub(super) fn start_worktree_task(
        &mut self,
        mut payload: CreateWorktreePayload,
        existing_branch: bool,
    ) -> bool {
        // The kind reaches `agent.start`, which runs it in the new pane's
        // shell, so only the providers Hide can start are accepted.
        if let Some(kind) = payload.agent_kind.as_deref() {
            let Some(canonical) = hide_agent_adapter::start_kind(kind) else {
                self.set_error(
                    "worktree.create_unknown_agent",
                    format!("No agent provider named {kind}"),
                    false,
                );
                return true;
            };
            payload.agent_kind = Some(canonical.to_owned());
        }
        let branch = payload.branch.trim().to_owned();
        if branch.is_empty() {
            self.set_error(
                "worktree.create_invalid_branch",
                "Branch is required",
                false,
            );
            return true;
        }
        let device = payload
            .device_id
            .clone()
            .filter(|device| device != self.node.as_str());
        let listed = match device.as_deref() {
            Some(device) => self
                .device_worktrees
                .get(device)
                .and_then(|worktrees| worktrees.projects.get(&payload.repository_root)),
            None => self.worktree_catalog.project(&payload.repository_root),
        };
        // A repository whose worktrees have not been read yet is not one
        // without branches: a device's are read after its helper answers.
        let Some(listed) = listed else {
            self.set_error(
                "worktree.create_unread",
                format!(
                    "The worktrees of {} have not been read yet; try again in a moment",
                    payload.repository_root
                ),
                true,
            );
            if let Some(device) = device.as_deref() {
                self.request_device_worktrees(device, false);
            }
            return true;
        };
        let has_branch = listed
            .worktrees
            .iter()
            .any(|row| row.branch.is_some() && row.head_sha.is_some());
        if !has_branch {
            self.set_error(
                "worktree.create_without_branches",
                "Create the repository's first branch before creating a worktree",
                true,
            );
            return true;
        }
        // The link is written into the new worktree's own metadata, in the
        // form the linking chain reads back (`sync_issues`).
        let issue = match payload.task_key.as_deref() {
            None => None,
            Some(key) => match crate::tasks::issue_token(key) {
                Some(token) => Some(token),
                None => {
                    self.set_error(
                        "worktree.create_unknown_task",
                        format!("No task named {key}"),
                        false,
                    );
                    return true;
                }
            },
        };
        let model = match agent_choice::chosen_model(
            payload.agent_kind.as_deref(),
            payload.model.as_deref(),
        ) {
            Ok(model) => model,
            Err(message) => {
                self.set_error("worktree.create_unknown_model", message, false);
                return true;
            }
        };
        let prompt = payload.agent_kind.as_ref().and(payload.prompt.clone());
        if let Some(message) = prompt
            .as_deref()
            .and_then(|prompt| live::prompt_argument(prompt).err())
        {
            self.set_error("worktree.create_invalid_prompt", message, false);
            return true;
        }
        let id = match self.begin_task_operation(
            "worktree_create",
            Some(payload.repository_root.clone()),
            Some(branch.clone()),
            payload.base_branch.clone(),
            payload.agent_kind.clone(),
        ) {
            Ok(id) => id,
            Err(message) => {
                self.set_error("task_operation.busy", message, true);
                return true;
            }
        };
        self.remember_agent_choice(payload.agent_kind.as_deref(), model.as_deref());
        self.set_task_agent_launch(
            id,
            prompt,
            agent_choice::agent_arguments(payload.agent_kind.as_deref(), model.as_deref(), &[]),
            hide_agent_adapter::LaunchChoice {
                model,
                effort: None,
            },
        );
        if let Some(operation) = self.snapshot.task_operation.as_mut() {
            operation.device_id = device.clone();
        }
        let request = live::WorktreeTaskRequest {
            id,
            repository_root: payload.repository_root,
            branch,
            base_branch: payload.base_branch,
            agent_kind: payload.agent_kind,
            focus: true,
            purpose: payload.purpose,
            // The link is kept where the core keeps a checkout's links: on
            // this machine or a node that dials it (D-29), never a device it
            // dials.
            issue: if device.as_deref().is_none_or(|device| {
                self.link_origin(device)
                    .is_some_and(LinkOrigin::reports_own_facts)
            }) {
                issue
            } else {
                None
            },
        };
        let context = match device.as_deref() {
            Some(device) => self.device_worktree_target(device),
            None => self.local_worktree_target(),
        };
        let context = match context {
            Ok(context) => context,
            Err(message) => {
                return self.ingest_task_operation_result(
                    id,
                    Err(format!("create worktree: {message}").into()),
                );
            }
        };
        let spawned = if existing_branch {
            live::spawn_existing_branch_worktree(context, request)
        } else {
            live::spawn_worktree_create(context, request)
        };
        if let Err(message) = spawned {
            return self.ingest_task_operation_result(id, Err(message.into()));
        }
        true
    }

    /// Writes the live Herdr token first and the branch-description mirror
    /// second on the task-operation worker. The runtime lock is held only for
    /// validating the target and publishing the receipt.
    pub(super) fn set_checkout_purpose(&mut self, payload: SetCheckoutPurposePayload) -> bool {
        let text = payload.text.trim().to_owned();
        let target = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .find_map(|workspace| {
                workspace
                    .checkouts
                    .iter()
                    .find(|checkout| checkout.id == payload.checkout_id)
                    .map(|checkout| (None, workspace.clone(), checkout.clone()))
            })
            .or_else(|| {
                self.snapshot.status.remote.iter().find_map(|status| {
                    status
                        .session
                        .as_ref()?
                        .workspaces
                        .iter()
                        .find_map(|workspace| {
                            workspace
                                .checkouts
                                .iter()
                                .find(|checkout| checkout.id == payload.checkout_id)
                                .map(|checkout| {
                                    (
                                        Some(status.target_id.clone()),
                                        workspace.clone(),
                                        checkout.clone(),
                                    )
                                })
                        })
                })
            });
        let Some((remote_target_id, workspace, checkout)) = target else {
            let id = match self.begin_task_operation("checkout_purpose", None, None, None, None) {
                Ok(id) => id,
                Err(message) => {
                    self.set_error("task_operation.busy", message, true);
                    return true;
                }
            };
            self.purpose_operation_target = Some(PurposeOperationTarget {
                id,
                checkout_id: payload.checkout_id,
                remote_target_id: None,
            });
            return self.fail_purpose_operation(
                id,
                "The checkout is no longer available",
                "checkout_purpose.unknown_checkout",
            );
        };
        let repository_root = workspace.path.clone();
        let checkout_path = checkout.path.clone();
        let branch = checkout.branch.clone();
        let persisted_branch = remote_target_id.is_none().then(|| branch.clone()).flatten();
        let id = match self.begin_task_operation(
            "checkout_purpose",
            Some(repository_root.clone()),
            persisted_branch.clone(),
            None,
            None,
        ) {
            Ok(id) => id,
            Err(message) => {
                self.set_error("task_operation.busy", message, true);
                return true;
            }
        };
        if let Some(operation) = self.snapshot.task_operation.as_mut() {
            operation.path = Some(checkout_path.clone());
        }
        self.purpose_operation_target = Some(PurposeOperationTarget {
            id,
            checkout_id: payload.checkout_id.clone(),
            remote_target_id: remote_target_id.clone(),
        });
        if text.chars().count() > 80 || text.contains(['\n', '\r']) {
            return self.fail_purpose_operation(
                id,
                "Purpose must be one line of 80 characters or fewer",
                "checkout_purpose.invalid",
            );
        }
        if let Some(target_id) = remote_target_id.as_deref() {
            let version = self
                .snapshot
                .status
                .remote
                .iter()
                .find(|remote| remote.target_id == target_id)
                .and_then(|remote| remote.herdr_version.as_deref());
            if !herdr_version_supports_purpose(version) {
                return self.fail_purpose_operation(
                    id,
                    remote_purpose_unavailable_reason(version),
                    "checkout_purpose.remote_unsupported",
                );
            }
        }
        // A device checkout's purpose goes to its owner workspace; its other
        // workspaces only hold tabs there (D-10).
        let session_workspace_id = if remote_target_id.is_some() {
            checkout.owner_workspace_id.clone()
        } else {
            workspace::authoritative_session_space(
                &self.last_session_spaces,
                &workspace,
                &checkout_path,
            )
            .map(|space| space.id.clone())
        };
        if branch.is_none() && session_workspace_id.is_none() {
            return self.fail_purpose_operation(
                id,
                "This detached checkout has no live Herdr workspace to hold a purpose",
                "checkout_purpose.no_target",
            );
        }
        let request = live::PurposeTaskRequest {
            id,
            checkout_id: payload.checkout_id,
            repository_root,
            branch: persisted_branch,
            session_workspace_id,
            purpose: text,
        };
        let spawn_result = if let Some(target_id) = remote_target_id.as_deref() {
            self.remote_controls
                .get(target_id)
                .cloned()
                .ok_or_else(|| "The remote Herdr connection is unavailable".to_owned())
                .and_then(|context| live::spawn_remote_purpose_write(context, request))
        } else {
            self.live
                .as_ref()
                .cloned()
                .ok_or_else(|| "A live Herdr connection is required".to_owned())
                .and_then(|context| live::spawn_purpose_write(context, request))
        };
        if let Err(message) = spawn_result {
            return self.ingest_purpose_operation_result(id, Err(message));
        }
        true
    }

    /// The Overview's `Open in History` and its `N files` chip: focus the
    /// checkout and switch the panel section together. The checkout has to be
    /// a local one the navigator lists; the section has to be one the panel
    /// has. Either failing is an error the operator did not cause and cannot
    /// act on, so it goes to the log and nothing on screen moves.
    pub(super) fn overview_open_section(&mut self, payload: OverviewOpenSectionPayload) -> bool {
        let Some(section) = RightPanelSection::parse(&payload.section) else {
            self.set_error(
                "overview.unknown_section",
                format!("Right panel has no section {}", payload.section),
                false,
            );
            return true;
        };
        let Some((workspace_id, checkout_id)) = self.local_checkout_ids(&payload.checkout_path)
        else {
            self.set_error(
                "overview.unknown_checkout",
                format!("Checkout is not listed: {}", payload.checkout_path),
                false,
            );
            return true;
        };
        if self.snapshot.navigator.focused_checkout_id.as_deref() != Some(checkout_id.as_str()) {
            self.focus_checkout(&workspace_id, &checkout_id);
        }
        self.snapshot.ui_state.right_panel_visible = true;
        self.snapshot.ui_state.right_panel_section = section;
        self.persist_ui_state();
        true
    }

    /// `New agent here ▸ Terminal only / Claude / Codex` and the empty
    /// group's `Start agent…`: one tab with the checkout as its cwd, in the
    /// checkout's own Herdr workspace, through the task operation slot the
    /// worktree sheet already reports through. The shell starts the provider
    /// in the pane the slot names, so `terminal` needs nothing more from it.
    pub(super) fn agent_start_in_checkout(&mut self, payload: AgentStartInCheckoutPayload) -> bool {
        self.start_in_checkout(payload, true, None)
    }

    /// A start in a checkout's new tab; `remember` false keeps the start out
    /// of the remembered agent choice (an archive resume). `cwd` is a folder
    /// inside the checkout the resumed conversation ran in; one outside it is
    /// ignored and the tab starts at the checkout.
    pub(super) fn start_in_checkout(
        &mut self,
        mut payload: AgentStartInCheckoutPayload,
        remember: bool,
        cwd: Option<String>,
    ) -> bool {
        let request_id = payload.request_id.clone();
        if payload.confirm_only {
            let device = payload.device_id.as_deref().unwrap_or(self.node.as_str());
            match self.group_start_confirmation_kind(
                device,
                payload.checkout_path.as_deref(),
                request_id.as_deref(),
            ) {
                Ok(provider) => {
                    payload.provider = provider;
                    payload.model = None;
                    payload.prompt = None;
                    payload.resume_session_id = None;
                    payload.resume_session_path = None;
                }
                Err(message) => {
                    self.set_request_error(
                        "agent_start.confirmation_unavailable",
                        message,
                        true,
                        request_id.as_deref(),
                    );
                    return true;
                }
            }
        }
        if self.repeat_checkout_start(request_id.as_deref())
            && (!payload.confirm_only
                || self.snapshot.task_operation.as_ref().is_some_and(|op| {
                    op.phase == "working" || op.agent_phase.as_deref() == Some("starting")
                }))
        {
            return true;
        }
        let agent_kind = match payload.provider.as_str() {
            "terminal" => None,
            other => {
                if let Some(kind) = hide_agent_adapter::start_kind(other) {
                    Some(kind.to_owned())
                } else {
                    self.set_request_error(
                        "agent_start.unknown_provider",
                        format!("No agent provider named {other}"),
                        false,
                        request_id.as_deref(),
                    );
                    return true;
                }
            }
        };
        let model = match agent_choice::chosen_model(
            agent_kind.as_deref(),
            agent_kind.as_ref().and(payload.model.as_deref()),
        ) {
            Ok(model) => model,
            Err(message) => {
                self.set_request_error(
                    "agent_start.unknown_model",
                    message,
                    false,
                    request_id.as_deref(),
                );
                return true;
            }
        };
        let resume = match payload.resume_session_id.as_deref() {
            None => None,
            Some(session_id) => match agent_kind.as_deref().and_then(|kind| {
                resume_session_arguments(kind, session_id).filter(|_| payload.prompt.is_none())
            }) {
                Some(arguments) => Some(arguments),
                None => {
                    self.set_request_error(
                        "agent_start.invalid_resume",
                        "This session cannot be resumed".to_owned(),
                        false,
                        request_id.as_deref(),
                    );
                    return true;
                }
            },
        };
        let prompt = agent_kind.as_ref().and(payload.prompt.clone());
        // A prompt the agent's command line cannot carry is refused before a
        // tab or Home sync, so nothing is left behind.
        if let Some(message) = prompt
            .as_deref()
            .and_then(|prompt| live::prompt_argument(prompt).err())
        {
            self.set_request_error(
                "agent_start.invalid_prompt",
                message,
                false,
                request_id.as_deref(),
            );
            return true;
        }
        let device = payload
            .device_id
            .clone()
            .filter(|device| !device.is_empty())
            .unwrap_or_else(|| self.node.as_str().to_owned());
        // A start names one place: a device's Home, or one of its checkouts.
        let checkout_path = match (payload.home, payload.checkout_path.clone()) {
            (true, None) => {
                return self.start_in_home(
                    &device,
                    agent_kind,
                    model,
                    prompt,
                    request_id,
                    payload.stay_behind,
                );
            }
            (false, Some(path)) if !path.is_empty() => path,
            _ => {
                self.set_request_error(
                    "agent_start.invalid_target",
                    "A start names either Home or one checkout",
                    false,
                    request_id.as_deref(),
                );
                return true;
            }
        };
        let local = device == self.node.as_str();
        let found = if local {
            self.local_checkout_tab(&checkout_path)
        } else {
            self.device_checkout_tab(&device, &checkout_path)
        };
        let (workspace_path, label, host) = match found {
            Ok(found) => found,
            Err((kind, message)) => {
                self.set_request_error(kind, message, false, request_id.as_deref());
                return true;
            }
        };
        // A natively proven resume names the reference its proof reads:
        // OpenCode's session id itself, a file reader's source file.
        let proof = agent_kind
            .as_deref()
            .and_then(hide_session::Agent::from_kind)
            .filter(|agent| agent.requires_native_proof());
        let resume_reference = match proof.filter(|_| resume.is_some()) {
            Some(agent) if agent.proof_reference_kind() == "id" => payload
                .resume_session_id
                .clone()
                .map(|id| crate::sidebar::SessionAgentSessionPayload {
                    kind: "id".into(),
                    value: id,
                }),
            Some(_) => {
                let Some(path) = payload.resume_session_path.as_ref().filter(|path| {
                    !path.is_empty() && path.len() <= 4096 && !path.chars().any(char::is_control)
                }) else {
                    self.set_request_error("agent_start.invalid_resume", "The selected session has no confirmed source file. Refresh its record before retrying.", false, request_id.as_deref());
                    return true;
                };
                Some(crate::sidebar::SessionAgentSessionPayload {
                    kind: "path".into(),
                    value: path.clone(),
                })
            }
            None => None,
        };
        let retry = (!payload.confirm_only)
            .then(|| {
                self.snapshot
                    .task_operation
                    .as_ref()
                    .filter(|op| {
                        op.request_id.as_deref() == request_id.as_deref()
                            && request_id.is_some()
                            && op.agent_phase.as_deref() == Some("failed")
                            && op.path.as_deref() == Some(checkout_path.as_str())
                            && op.agent_kind == agent_kind
                            && op.device_id.as_deref().unwrap_or(self.node.as_str()) == device
                    })
                    .map(|op| op.id)
            })
            .flatten();
        let id = match retry.map(Ok).unwrap_or_else(|| {
            self.begin_task_operation(
                "agent_start",
                Some(workspace_path),
                None,
                None,
                agent_kind.clone(),
            )
        }) {
            Ok(id) => id,
            Err(message) => {
                self.set_request_error("task_operation.busy", message, true, request_id.as_deref());
                return true;
            }
        };
        if let Some(operation) = self.snapshot.task_operation.as_mut() {
            operation.request_id = request_id.clone();
            operation.device_id = (!local).then(|| device.clone());
            operation.stays_behind = payload.stay_behind;
        }
        // A resume Hide makes from the archive is not the operator's choice
        // of agent.
        if remember && !payload.confirm_only {
            self.remember_agent_choice(agent_kind.as_deref(), model.as_deref());
        }
        let mut arguments = resume.unwrap_or_default();
        arguments.extend(agent_choice::agent_arguments(
            agent_kind.as_deref(),
            model.as_deref(),
            &[],
        ));
        self.set_task_agent_launch(
            id,
            prompt,
            arguments,
            hide_agent_adapter::LaunchChoice {
                model,
                effort: None,
            },
        );
        if let Some(launch) = self.task_agent_launch.as_mut() {
            launch.resume_reference = resume_reference.clone();
        }
        if retry.is_some() {
            return self.retry_task_agent(id);
        }
        if (local && self.live.is_none()) || (!local && !self.remote_controls.contains_key(&device))
        {
            return self.ingest_task_operation_result(
                id,
                Err(if local {
                    "workspace.control_unavailable".into()
                } else {
                    "agent_start.device_unavailable".into()
                }),
            );
        }
        let prepared = if self.workspace_views.is_some() || payload.target.is_some() {
            match self.prepare_group_start(
                id,
                (&device, &checkout_path),
                payload.target,
                request_id.as_deref(),
                agent_kind.as_deref(),
                payload.confirm_only,
            ) {
                Ok(prepared) => Some(prepared),
                Err(message) => return self.ingest_task_operation_result(id, Err(message.into())),
            }
        } else {
            self.group_start = None;
            None
        };
        let cwd = cwd
            .filter(|cwd| hide_platform::path::wire_relative(&checkout_path, cwd).is_ok())
            .unwrap_or_else(|| checkout_path.clone());
        let request = live::CheckoutTabRequest {
            start: prepared.as_ref().map(|prepared| prepared.request.clone()),
            id,
            agent_kind,
            resume_reference,
            resume_scope: payload
                .resume_session_id
                .as_ref()
                .filter(|_| proof.is_some())
                .map(|id| hide_session::SessionReadScope {
                    id: id.clone(),
                    cwd: cwd.clone(),
                }),
            cwd,
            checkout_path,
            label,
            host,
            // The result selects its target only if it is still visible.
            // Herdr must not move focus behind a group the user selected.
            focus: !payload.stay_behind && prepared.is_none(),
        };
        let target = if local {
            self.live
                .as_ref()
                .map(|context| live::TabTarget::local(context, self.live_generation))
                .ok_or("start an agent: a live Herdr connection is required")
        } else {
            self.remote_controls
                .get(&device)
                .map(live::TabTarget::device)
                .ok_or("agent_start.device_unavailable")
        };
        let target = match target {
            Ok(target) => target,
            Err(message) => return self.ingest_task_operation_result(id, Err(message.into())),
        };
        let recovery_only = prepared
            .as_ref()
            .is_some_and(|prepared| prepared.request.recovery_only);
        let spawned = match prepared {
            Some(prepared) => live::spawn_targeted_checkout_start(target, request, prepared.saved),
            None => live::spawn_checkout_tab_create(target, request),
        };
        if let Err(message) = spawned {
            self.group_start_failed(id, recovery_only);
            return self.ingest_task_operation_result(id, Err(message.into()));
        }
        true
    }

    /// The project path, next tab label and tab host of a local checkout, or
    /// the refusal a start reports for a path the navigator does not list.
    fn local_checkout_tab(
        &self,
        checkout_path: &str,
    ) -> Result<(String, String, TabHost), (&'static str, String)> {
        let unknown = || {
            (
                "overview.unknown_checkout",
                format!("Checkout is not listed: {checkout_path}"),
            )
        };
        let (workspace_id, checkout_id) =
            self.local_checkout_ids(checkout_path).ok_or_else(unknown)?;
        let workspace = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .find(|workspace| workspace.id == workspace_id)
            .ok_or_else(unknown)?;
        let checkout = workspace
            .checkouts
            .iter()
            .find(|checkout| checkout.id == checkout_id)
            .ok_or_else(unknown)?;
        let host = self
            .local_tab_host(&workspace_id, &checkout_id)
            .ok_or_else(unknown)?;
        Ok((
            workspace.path.clone(),
            checkout.next_tab_label.clone(),
            host,
        ))
    }

    /// The same for a checkout on `device`, from that device's session: the
    /// tab goes to the checkout's owner there, opened first when none is
    /// (PRD home-device-rail D-22, checkout-workspace-binding D-07).
    fn device_checkout_tab(
        &self,
        device: &str,
        checkout_path: &str,
    ) -> Result<(String, String, TabHost), (&'static str, String)> {
        let session = self
            .snapshot
            .status
            .remote
            .iter()
            .find(|remote| remote.target_id == device)
            .and_then(|remote| remote.session.as_ref())
            .ok_or_else(|| {
                (
                    "agent_start.device_unavailable",
                    format!("{device} is not connected"),
                )
            })?;
        let (project, checkout) = session
            .workspaces
            .iter()
            .find_map(|project| {
                project
                    .checkouts
                    .iter()
                    .find(|checkout| checkout.path == checkout_path)
                    .map(|checkout| (project, checkout))
            })
            .ok_or_else(|| {
                (
                    "overview.unknown_checkout",
                    format!("Checkout is not listed on {device}: {checkout_path}"),
                )
            })?;
        if checkout.owner_workspace_id.is_none() && checkout.unconfirmed {
            return Err((
                "remote.control.checkout_unconfirmed",
                format!("{checkout_path} on {device} is not confirmed yet; no tab was opened"),
            ));
        }
        Ok((
            project.path.clone(),
            checkout.next_tab_label.clone(),
            tab_host(project, checkout, device, &self.node),
        ))
    }

    /// The workspace and checkout ids a local checkout path names, or `None`
    /// for a path the navigator does not list locally.
    fn local_checkout_ids(&self, checkout_path: &str) -> Option<(String, String)> {
        self.snapshot
            .navigator
            .workspaces
            .iter()
            .filter(|workspace| workspace.remote_target_id.is_none())
            .find_map(|workspace| {
                workspace
                    .checkouts
                    .iter()
                    .find(|checkout| checkout.path == checkout_path)
                    .map(|checkout| (workspace.id.clone(), checkout.id.clone()))
            })
    }

    /// Where a new tab in one local checkout goes: its owner Herdr workspace,
    /// or that owner opened first (PRD checkout-workspace-binding D-07). A
    /// workspace that merely holds the checkout's tabs is never the answer
    /// (D-08).
    pub(super) fn local_tab_host(&self, project_id: &str, checkout_id: &str) -> Option<TabHost> {
        let project = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .find(|workspace| workspace.id == project_id)?;
        let checkout = project
            .checkouts
            .iter()
            .find(|checkout| checkout.id == checkout_id)?;
        Some(tab_host(project, checkout, self.node.as_str(), &self.node))
    }

    pub(super) fn migrate_main_branch(&mut self, payload: MigrateMainBranchPayload) -> bool {
        let Some(project) = self.worktree_catalog.project(&payload.repository_root) else {
            self.set_error(
                "branch_migrate.unknown_project",
                "Repository status is unavailable",
                true,
            );
            return true;
        };
        let Some(main) = project.worktrees.iter().find(|row| row.is_main) else {
            self.set_error(
                "branch_migrate.missing_main",
                "The main worktree is unavailable",
                true,
            );
            return true;
        };
        if main.dirty {
            self.set_error(
                "branch_migrate.dirty",
                "Commit or discard uncommitted changes before moving the branch",
                true,
            );
            return true;
        }
        let branch = main.branch.clone();
        let id = match self.begin_task_operation(
            "branch_migrate",
            Some(payload.repository_root.clone()),
            branch.clone(),
            Some(payload.base_branch.clone()),
            None,
        ) {
            Ok(id) => id,
            Err(message) => {
                self.set_error("task_operation.busy", message, true);
                return true;
            }
        };
        let request = live::WorktreeTaskRequest {
            id,
            repository_root: payload.repository_root,
            branch: branch.unwrap_or_default(),
            base_branch: Some(payload.base_branch),
            agent_kind: None,
            focus: false,
            purpose: None,
            issue: None,
        };
        let Some(context) = self.live.as_ref().cloned() else {
            return self
                .ingest_task_operation_result(id, Err("workspace.control_unavailable".into()));
        };
        if let Err(message) =
            live::spawn_branch_migration(context, Arc::clone(&self.own_node), request)
        {
            return self.ingest_task_operation_result(id, Err(message.into()));
        }
        true
    }

    /// Accepts a projection only while it still describes the checkout the
    /// runtime is asking about, so a slow read against a checkout the operator
    /// has already left cannot overwrite the current one.
    pub fn ingest_changes(&mut self, answer: crate::changes::ChangesAnswer) -> bool {
        if answer.key != self.changes_key() {
            return false;
        }
        let mut changes = answer.changes;
        let detail = answer.detail;
        // A failed read of the checkout already on screen keeps the list its
        // last successful read confirmed, marked stale with the reason,
        // rather than replacing it with nothing (S5.5 B22).
        if let Some(reason) = changes.unavailable_reason.clone()
            && self.changes_published_key == answer.key
            && self.snapshot.changes.unavailable_reason.is_none()
            && self.snapshot.changes.root_path == changes.root_path
        {
            let mut kept = crate::model::ChangesSnapshot::clone(&self.snapshot.changes);
            kept.stale_reason = Some(reason);
            changes = kept;
        }
        // An answer lands one wake after its request, and with View areas a
        // file in front leaves the next request asking for the row History
        // shows. An answer for a row the operator has left since would put
        // that row back, the next request would ask for it again, and the
        // two rows would chase each other with a read and a frame on every
        // tick while nothing is driven. Its list is still the newest, so only
        // the row and its diff stay as they are. The closed view's empty
        // projection is taken whole, so its diff text leaves the wire, and
        // an older client's reading is left as it was.
        if self.separate_view_areas()
            && answer.key.is_some()
            && answer.selection != self.changes_selection()
        {
            changes.selected_path = self.snapshot.changes.selected_path.clone();
            changes.selected_committed = self.snapshot.changes.selected_committed;
            changes.diff = self.snapshot.changes.diff.clone();
        }
        self.changes_published_key = answer.key;
        // The one comparison of the section: it takes a new edit number, and
        // is sent again, only when this read changed it.
        if self.snapshot.changes == changes {
            return false;
        }
        // A failure is logged when it first shows or changes. One the
        // operator cannot act on, a folder that is not a repository, is shown
        // nowhere else (issue 570).
        let failure = |changes: &crate::model::ChangesSnapshot| {
            changes
                .unavailable_reason
                .clone()
                .or_else(|| changes.stale_reason.clone())
        };
        if let Some(error) = failure(&changes)
            && failure(&self.snapshot.changes).as_ref() != Some(&error)
        {
            crate::diagnostic!(serde_json::json!({
                "component": "changes",
                "kind": "changes.unavailable",
                "device": self.changes_published_key.as_ref().map(|key| key.device_id.as_str()),
                "not_a_repository": changes.not_a_repository,
                "stale": changes.stale_reason.is_some(),
                "error": error,
                "reason": detail,
            }));
        }
        self.snapshot.changes.set(changes);
        true
    }

    /// Herdr closes a workspace with its last pane, and a project that exists
    /// only as that Herdr workspace would vanish from the sidebar with it.
    /// The user asked to close a pane, not to forget the project, so the
    /// project is registered at its repository path first. The path-keyed
    /// project id is unchanged by this, and the row stays selectable with
    /// its "start new terminal" control once Herdr's workspace is gone.
    pub(super) fn retain_project_before_last_pane_closes(&mut self, pane_id: &str) {
        let Some(project) = self.snapshot.navigator.workspaces.iter().find(|workspace| {
            workspace.remote_target_id.is_none()
                && workspace
                    .checkouts
                    .iter()
                    .flat_map(|checkout| checkout.tabs.iter())
                    .flat_map(|tab| tab.panes.iter())
                    .any(|pane| pane.id == pane_id)
        }) else {
            return;
        };
        let pane_count = project
            .checkouts
            .iter()
            .flat_map(|checkout| checkout.tabs.iter())
            .map(|tab| tab.panes.len())
            .sum::<usize>();
        if project.registered || pane_count != 1 {
            return;
        }
        let registration =
            match workspace::registration(&project.path, &project.repo_name, &project.device_id) {
                Ok(registration) => registration,
                Err(message) => {
                    self.set_error("workspace.retain_failed", message, false);
                    return;
                }
            };
        if self
            .snapshot
            .ui_state
            .workspace_registrations
            .iter()
            .any(|existing| existing.id == registration.id)
        {
            return;
        }
        self.push_diagnostic(
            "workspace.retained",
            format!(
                "Registered {} at {} so closing its last pane keeps the project listed",
                registration.label, registration.path
            ),
        );
        self.snapshot
            .ui_state
            .workspace_registrations
            .push(registration);
        // The project id is path-keyed, so registering changes nothing the
        // sidebar shows right now; the sync that follows Herdr's
        // workspace_closed rebuilds the catalog off the runtime lock.
        self.persist_current_ui_state();
    }
}

/// Where a new tab in `checkout` of `project` on `device_id` goes: the owner
/// the session names, else the owner to open.
pub(super) fn tab_host(
    project: &WorkspaceSnapshot,
    checkout: &crate::model::CheckoutSnapshot,
    device_id: &str,
    node: &crate::node::NodeId,
) -> TabHost {
    match &checkout.owner_workspace_id {
        Some(owner) => TabHost::Workspace(owner.clone()),
        None => TabHost::Open(owner_open(project, checkout, device_id, node)),
    }
}

/// How `checkout` of `project` gets its owner opened.
pub(super) fn owner_open(
    project: &WorkspaceSnapshot,
    checkout: &crate::model::CheckoutSnapshot,
    device_id: &str,
    node: &crate::node::NodeId,
) -> OwnerOpen {
    let owner = OwnerOpen::for_checkout(
        device_id,
        &checkout.path,
        &project.path,
        project.is_git,
        owner_label(project, checkout),
    );
    if *node == device_id {
        owner.on_node()
    } else {
        owner
    }
}

/// The name a workspace Hide opens for `checkout` takes, as the sidebar names
/// the checkout: a linked worktree by its branch, the primary checkout and a
/// plain folder by the project (D-14).
pub(super) fn owner_label<'a>(
    project: &'a WorkspaceSnapshot,
    checkout: &'a crate::model::CheckoutSnapshot,
) -> &'a str {
    if checkout.is_worktree {
        &checkout.label
    } else {
        &project.label
    }
}

/// Gives each checkout the one pull request that is its own work
/// (`github::pull_request_for_checkout`), and reports whether any changed.
/// Both places that learn something new about a checkout, GitHub's list and
/// the worktree reader's HEAD, come through here.
pub(super) fn associate_pull_requests(
    workspace: &mut crate::model::WorkspaceSnapshot,
    project: Option<&crate::model::GithubProjectSnapshot>,
) -> bool {
    let mut changed = false;
    for checkout in &mut workspace.checkouts {
        let found = project.and_then(|project| {
            crate::github::pull_request_for_checkout(
                &project.pull_requests,
                checkout.branch.as_deref(),
                checkout.head_sha(),
            )
        });
        // Cloned only when it moved: this runs on every session update.
        if checkout.pull_request.as_ref() != found {
            checkout.pull_request = found.cloned();
            changed = true;
        }
    }
    changed
}

/// A pull request's lowercase `owner/name` and number, from its address.
pub(super) fn pull_request_address(url: &str) -> Option<(String, u64)> {
    hide_session::pull_request_addresses(url)
        .into_iter()
        .next()
        .map(|(repository, number)| (repository.to_ascii_lowercase(), number))
}

/// When GitHub made each pull request read, by the address's lowercase
/// `owner/name` and number.
fn pull_request_times(
    github: &crate::model::GithubSnapshot,
) -> crate::labels::facts::PullRequestTimes {
    github
        .projects
        .iter()
        .flat_map(|project| project.pull_requests.iter())
        .filter_map(|pull_request| {
            let created = pull_request.created_at_unix_ms?;
            Some((pull_request_address(&pull_request.url)?, created))
        })
        .collect()
}

/// The arguments that resume `session_id` in `kind`'s CLI, or none for an id
/// that is not one plain token: it reaches the agent's command line, where
/// one that began with `-` would read as an option.
pub(super) fn resume_session_arguments(kind: &str, session_id: &str) -> Option<Vec<String>> {
    let proven = hide_session::Agent::from_kind(kind)
        .is_some_and(hide_session::Agent::requires_native_proof);
    let plain = if proven {
        hide_session::valid_native_id(session_id) && !session_id.ends_with(".jsonl")
    } else {
        session_id
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
            && session_id.len() <= 128
            && session_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    };
    if !plain {
        return None;
    }
    crate::recent_closed::resume_arguments(&crate::recent_closed::ClosedAgent {
        kind: kind.to_owned(),
        session_id: Some(session_id.to_owned()),
        ..Default::default()
    })
}

/// `answer` with what `previous` read kept wherever a project's read failed:
/// a failed lookup must not erase the answer it failed to replace, so the
/// card and the board show the previous pull requests and issues with `as of`
/// beside them and only the status changes; dependencies and sub-issues that
/// could not be read keep the ones read before. This machine's answers and a
/// node's go through it alike.
fn keep_previous_reads(
    answer: &mut crate::model::GithubSnapshot,
    previous: &crate::model::GithubSnapshot,
) {
    for project in &mut answer.projects {
        if (project.status.stale || project.status.unavailable_reason.is_some())
            && let Some(previous) = previous.project(&project.root_path)
        {
            let incomplete = !project.pull_requests_read || !project.issues_read;
            if !project.pull_requests_read
                && (previous.pull_requests_read
                    || previous.status.last_success_at_unix_ms.is_some())
            {
                project.pull_requests = previous.pull_requests.clone();
                project.pull_requests_read = true;
                // The kept list carries the judgments it was kept with, so
                // the questions that left them unknown stay named beside it.
                project.status.unread_queries = previous.status.unread_queries.clone();
            }
            if !project.issues_read
                && (previous.issues_read || previous.status.last_success_at_unix_ms.is_some())
            {
                project.issues = previous.issues.clone();
                project.issues_read = true;
            }
            if incomplete {
                project.status.stale = true;
                project.status.last_success_at_unix_ms = previous.status.last_success_at_unix_ms;
            }
        }
        // Dependencies and sub-issues that could not be read this pass keep the ones
        // read before, the same way a failed lookup keeps its answer.
        if project.issues.dependencies_failure.is_some()
            && let Some(previous) = previous.project(&project.root_path)
        {
            for issue in &mut project.issues.issues {
                if let Some(known) = previous
                    .issues
                    .issues
                    .iter()
                    .find(|known| known.reference == issue.reference)
                {
                    issue.blocked_by = known.blocked_by.clone();
                    issue.sub_issues = known.sub_issues.clone();
                }
            }
        }
    }
}

/// The Overview card of `checkout`, whichever machine it is on.
fn checkout_card(
    checkout: &CheckoutSnapshot,
    github: crate::model::GithubStatusSnapshot,
    sizes: &[crate::model::DiskUsageSnapshot],
    panes: Vec<crate::model::CheckoutPaneContext>,
) -> crate::model::CheckoutCardSnapshot {
    let disk = sizes
        .iter()
        .find(|disk| disk.path.as_deref() == Some(checkout.path.as_str()))
        .cloned()
        .unwrap_or_default();
    let disk_measuring = checkout.is_worktree && disk.path.is_none();
    crate::model::CheckoutCardSnapshot {
        checkout_id: Some(checkout.id.clone()),
        panes,
        github,
        disk,
        disk_measuring,
        deletion_gate: checkout
            .worktree
            .as_ref()
            .map(|worktree| worktree.deletion_gate.clone()),
    }
}

/// The most heads a project keeps from cleanup reviews' branch lookups; one
/// past it is not kept and is logged (`review.merge_proofs_full`).
pub(super) const REVIEW_PROOF_LIMIT: usize = 1024;

/// GitHub's merged heads for a project's sidebar mark: the pull request list
/// joined with what a cleanup review proved by branch for the same default
/// branch, so the mark and the review give one answer (PRD git-freshness
/// B17). With neither, GitHub has not answered.
pub(super) fn with_review_proofs(
    listed: Option<BTreeSet<String>>,
    proofs: &BTreeMap<(Option<String>, String, String), BTreeSet<String>>,
    node: Option<&str>,
    root: &str,
    base: Option<&str>,
) -> Option<BTreeSet<String>> {
    let proved = base
        .and_then(|base| proofs.get(&(node.map(str::to_owned), root.to_owned(), base.to_owned())));
    match (listed, proved) {
        (listed, None) => listed,
        (None, Some(proved)) => Some(proved.clone()),
        (Some(listed), Some(proved)) => {
            Some(listed.into_iter().chain(proved.iter().cloned()).collect())
        }
    }
}

#[cfg(test)]
mod purpose_version_tests {
    use super::*;

    #[test]
    fn remote_purpose_requires_the_pinned_minimum_version() {
        assert!(!herdr_version_supports_purpose(None));
        assert!(!herdr_version_supports_purpose(Some("0.9.0")));
        assert!(herdr_version_supports_purpose(Some("0.9.1")));
        assert!(herdr_version_supports_purpose(Some("v0.10.0")));
        assert!(herdr_version_supports_purpose(Some("1.0.0-beta.1")));
        assert!(!herdr_version_supports_purpose(Some("unknown")));
    }
}
