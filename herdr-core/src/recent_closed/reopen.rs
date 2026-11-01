//! Reopening a closed item through the engine: its tab, its panes and their
//! geometry, its agents resumed with their conversations, and a sleeping
//! agent's tab, each found again by the intent marker it was made with.

use std::sync::{Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use crate::engine::Engine;

use crate::checkout_owner::OwnerOpen;
use crate::checkout_owner::open::ensure_owner;
use crate::handle::ChangeNotifier;
use crate::live::{CONTROL_WAIT, LiveContext, failed, fetch_session, start_agent};
use crate::model::EditorDocumentSnapshot;
use crate::recent_closed::{
    ClosedAgent, ClosedContext, ClosedItem, ClosedLayoutBranch, ClosedLayoutNode, ClosedPane,
    PanePlacement, resume_arguments,
};
use crate::runtime::Runtime;
use crate::sidebar::SessionSnapshotPayload;

#[derive(Clone, Debug)]
pub struct ReopenRequest {
    pub item: ClosedItem,
    pub workspace_exists: bool,
    pub tab_exists: bool,
    pub fallback_pane_id: Option<String>,
    /// The owner of the checkout the closed item belongs to: a tab that has to
    /// be made again lands there, never in the workspace it was closed from
    /// (PRD checkout-workspace-binding D-16). `None` for a file.
    pub owner: Option<OwnerOpen>,
    pub(crate) codex_daemon: crate::codex_launch::CodexDaemon,
    /// The 권한 of each agent on the machine the item reopens on, read when
    /// the reopen was asked for (`agent_permission`).
    pub(crate) permissions: crate::agent_permission::AgentPermissions,
}

pub(crate) const REOPEN_INTENT_ENV: &str = "HIDE_REOPEN_INTENT";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReopenIntentStage {
    LocalCreate,
    Workspace,
    Layout,
    Pane,
    CloseReplacement,
}

pub(crate) fn reopen_intent_marker(key: &str, stage: ReopenIntentStage) -> String {
    let stage = match stage {
        ReopenIntentStage::LocalCreate => "local-create",
        ReopenIntentStage::Workspace => "workspace",
        ReopenIntentStage::Layout => "layout",
        ReopenIntentStage::Pane => "pane",
        ReopenIntentStage::CloseReplacement => "close-replacement",
    };
    format!("{key}:{stage}")
}

pub(crate) fn reopen_intent_env(
    key: &str,
    stage: ReopenIntentStage,
) -> std::collections::BTreeMap<String, String> {
    [(
        REOPEN_INTENT_ENV.to_owned(),
        reopen_intent_marker(key, stage),
    )]
    .into_iter()
    .collect()
}

fn tag_reopen_layout(
    node: &ClosedLayoutNode,
    key: &str,
    stage: ReopenIntentStage,
) -> ClosedLayoutNode {
    match node {
        ClosedLayoutNode::Pane {
            pane_id,
            label,
            cwd,
            command,
            env,
        } => {
            let mut env = env.clone();
            env.insert(
                REOPEN_INTENT_ENV.to_owned(),
                reopen_intent_marker(key, stage),
            );
            ClosedLayoutNode::Pane {
                pane_id: pane_id.clone(),
                label: crate::start_claim::public_label(label.as_deref()),
                cwd: cwd.clone(),
                command: command.clone(),
                env,
            }
        }
        ClosedLayoutNode::Split {
            direction,
            ratio,
            first,
            second,
        } => ClosedLayoutNode::Split {
            direction: *direction,
            ratio: *ratio,
            first: Box::new(tag_reopen_layout(first, key, stage)),
            second: Box::new(tag_reopen_layout(second, key, stage)),
        },
    }
}

pub(crate) fn pane_ids_with_reopen_marker(
    node: &ClosedLayoutNode,
    marker: &str,
    output: &mut Vec<String>,
) {
    match node {
        ClosedLayoutNode::Pane { pane_id, env, .. } => {
            if env
                .get(REOPEN_INTENT_ENV)
                .is_some_and(|value| value == marker)
            {
                output.extend(pane_id.iter().cloned());
            }
        }
        ClosedLayoutNode::Split { first, second, .. } => {
            pane_ids_with_reopen_marker(first, marker, output);
            pane_ids_with_reopen_marker(second, marker, output);
        }
    }
}

pub(crate) fn layout_has_reopen_marker(
    layout: &crate::recent_closed::ClosedLayout,
    marker: &str,
) -> bool {
    let mut pane_ids = Vec::new();
    pane_ids_with_reopen_marker(&layout.root, marker, &mut pane_ids);
    !pane_ids.is_empty()
}

pub(crate) fn export_reopen_layout(
    engine: &dyn Engine,
    tab_id: &str,
) -> Result<crate::recent_closed::ClosedLayout, String> {
    engine
        .export_layout(CONTROL_WAIT, tab_id)
        .map_err(failed("layout.export"))
}

fn restore_tab_position(
    engine: &dyn Engine,
    context: &ClosedContext,
    layout: crate::recent_closed::ClosedLayout,
    notices: &mut Vec<String>,
) -> crate::recent_closed::ClosedLayout {
    // The position is an index among the tabs of the workspace it was closed
    // from; a tab that reopened in the checkout's owner elsewhere keeps the
    // place Herdr gave it. That is the usual case for a tab closed outside
    // the owner, and for a plain folder whose owner closed with its last tab.
    if layout.workspace_id != context.workspace_id {
        return layout;
    }
    let move_result = engine
        .move_tab(CONTROL_WAIT, &layout.tab_id, context.tab_index)
        .map_err(failed("tab.move"));
    if let Err(message) = move_result {
        notices.push(format!(
            "The tab reopened but its original position could not be restored: {message}"
        ));
    }
    layout
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReopenNotice {
    pub pane_id: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReopenOutcome {
    pub tab_id: Option<String>,
    pub consumed: bool,
    pub focused_pane_id: Option<String>,
    pub notices: Vec<ReopenNotice>,
    /// The agents the reopen started, so the core keeps each one's launch
    /// choice with its new pane.
    pub agents: Vec<ReopenedAgent>,
}

/// An agent a reopen started in a restored pane: the pane, its kind, the
/// choice it was started with, and the conversation it resumed (none for a
/// new one, when the resume did not take).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReopenedAgent {
    pub pane_id: String,
    pub kind: String,
    /// The name the reopen started it under.
    pub name: String,
    pub launch: hide_agent_adapter::LaunchChoice,
    pub session_id: Option<String>,
}

#[derive(Debug)]
pub enum FileReopenResult {
    Opened(Box<(EditorDocumentSnapshot, crate::files::DocumentPlace)>),
    Missing,
    Failed(String),
}

pub fn spawn_reopen(context: LiveContext, request: ReopenRequest) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-reopen-closed".to_owned())
        .spawn(move || {
            let node = context.node.as_ref();
            let folders = |path: &str| crate::node_access::is_directory(node, path);
            let result = run_herdr_reopen(context.engine.as_ref(), &folders, &request)
                .map(FileReopenResultOrHerdr::Herdr);
            let Some(runtime) = context.runtime.upgrade() else {
                return;
            };
            let changed = match runtime.lock() {
                Ok(mut guard) => guard.ingest_reopen_result(&request, result),
                Err(_) => return,
            };
            drop(runtime);
            if changed {
                context.notifier.notify();
            }
        })
        .map(|_| ())
        .map_err(|error| format!("reopen worker could not be started: {error}"))
}

pub fn spawn_file_reopen(
    runtime: Weak<Mutex<Runtime>>,
    notifier: ChangeNotifier,
    request: ReopenRequest,
    root: crate::files::DocumentRoot,
    channel: std::sync::Arc<dyn crate::node_access::NodeLink>,
) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-file-reopen".to_owned())
        .spawn(move || {
            let ClosedItem::File { path, .. } = &request.item else {
                return;
            };
            let result = Ok(run_file_reopen(channel.as_ref(), &root, path));
            let Some(runtime) = runtime.upgrade() else {
                return;
            };
            let changed = match runtime.lock() {
                Ok(mut guard) => guard.ingest_reopen_result(&request, result),
                Err(_) => return,
            };
            drop(runtime);
            if changed {
                notifier.notify();
            }
        })
        .map(|_| ())
        .map_err(|error| format!("file reopen worker could not be started: {error}"))
}

#[derive(Debug)]
pub enum FileReopenResultOrHerdr {
    File(FileReopenResult),
    Herdr(ReopenOutcome),
}

fn run_file_reopen(
    channel: &dyn crate::node_access::NodeLink,
    root: &crate::files::DocumentRoot,
    path: &str,
) -> FileReopenResultOrHerdr {
    FileReopenResultOrHerdr::File(match crate::files::open_document(channel, root, path) {
        Ok(opened) => FileReopenResult::Opened(Box::new(opened)),
        Err(crate::files::OpenFailure::Missing) => FileReopenResult::Missing,
        Err(failure) => FileReopenResult::Failed(failure.message()),
    })
}

/// `folders` says whether a closed pane's folder is still a directory on the
/// node that held it.
fn run_herdr_reopen(
    engine: &dyn Engine,
    folders: &dyn Fn(&str) -> bool,
    request: &ReopenRequest,
) -> Result<ReopenOutcome, String> {
    match &request.item {
        ClosedItem::Pane {
            key,
            context,
            pane,
            placement,
        } => reopen_pane(engine, folders, key, context, pane, placement, request),
        ClosedItem::Tab {
            key,
            context,
            layout,
            panes,
        } => reopen_tab(
            engine,
            folders,
            key,
            context,
            reopen_owner(request)?,
            &layout.root,
            panes,
            request.codex_daemon,
            &request.permissions,
        ),
        ClosedItem::File { .. } => unreachable!("file reopen uses the filesystem worker"),
        ClosedItem::Draft { .. } => unreachable!("draft reopen is local"),
        ClosedItem::Tree { .. } => unreachable!("a tree reopens one member at a time"),
    }
}

/// Creates or adopts exactly one intent-marked wake tab in the current
/// checkout owner. No folder substitution or fresh-agent fallback is allowed.
pub(crate) fn create_sleep_tab(
    engine: &dyn Engine,
    node: &dyn crate::node_access::NodeLink,
    key: &str,
    context: &ClosedContext,
    owner: &OwnerOpen,
    cwd: &str,
) -> Result<(String, String, SessionSnapshotPayload), String> {
    if !crate::node_access::is_directory(node, cwd) {
        return Err("The sleeping session's working folder is unavailable".into());
    }
    let root = ClosedLayoutNode::Pane {
        pane_id: None,
        label: None,
        cwd: Some(cwd.into()),
        command: None,
        env: Default::default(),
    };
    let mut notices = Vec::new();
    let layout = ensure_workspace_and_tab(engine, key, context, owner, false, &root, &mut notices)?;
    // Position notices do not change the owned tab/pane identity or cwd.
    for _ in notices {
        crate::diagnostic!(
            serde_json::json!({"component":"agent_sleep", "kind":"agent_sleep.wake_tab_position_unconfirmed", "sleep_id":key})
        );
    }
    let snapshot = fetch_session(engine)
        .map_err(|_| "The wake tab exists but its current topology could not be read".to_owned())?;
    Ok((layout.tab_id, layout.focused_pane_id, snapshot))
}

struct ReopenIntentMatch {
    open_owner: Option<String>,
    layout: Option<crate::recent_closed::ClosedLayout>,
    workspace_seed: Option<(String, String)>,
}

/// Searches only current-owner tabs carrying the exact intent marker.
/// This part of reopen is read-only and is also used to inspect uncertain wakes.
fn find_reopen_intent(
    engine: &dyn Engine,
    key: &str,
    context: &ClosedContext,
    owner: &OwnerOpen,
    snapshot: &SessionSnapshotPayload,
) -> Result<ReopenIntentMatch, String> {
    let started = Instant::now();
    // The tab goes to the checkout's owner. A retry adopts only a layout or a
    // workspace seed carrying this intent's marker: in the owner when it is
    // open, else in a workspace that appeared since the close.
    let open_owner = owner
        .find_in(
            snapshot
                .workspaces
                .iter()
                .map(crate::checkout_owner::WorkspaceFacts::from),
        )
        .map(str::to_owned);
    let workspace_marker = reopen_intent_marker(key, ReopenIntentStage::Workspace);
    let layout_marker = reopen_intent_marker(key, ReopenIntentStage::Layout);
    let candidate_workspace_ids = match &open_owner {
        Some(owner) => vec![owner.clone()],
        None => snapshot
            .workspaces
            .iter()
            .filter(|workspace| {
                !context
                    .workspace_ids_before_close
                    .contains(&workspace.workspace_id)
            })
            .map(|workspace| workspace.workspace_id.clone())
            .collect(),
    };
    let mut recovered_layouts = Vec::new();
    let mut recovered_workspace_seeds = Vec::new();
    let candidates = snapshot
        .tabs
        .iter()
        .filter(|tab| {
            candidate_workspace_ids.contains(&tab.workspace_id)
                && !context.tab_ids_before_close.contains(&tab.tab_id)
        })
        .collect::<Vec<_>>();
    if candidates.len() > 64 {
        return Err("Too many tabs to inspect for the saved intent".into());
    }
    for tab in candidates {
        if started.elapsed() > Duration::from_secs(5) {
            return Err("Saved-intent inspection timed out".into());
        }
        let layout = export_reopen_layout(engine, &tab.tab_id)?;
        if layout_has_reopen_marker(&layout, &layout_marker) {
            recovered_layouts.push(layout);
        } else if layout_has_reopen_marker(&layout, &workspace_marker) {
            recovered_workspace_seeds.push((tab.workspace_id.clone(), tab.tab_id.clone()));
        }
    }
    if recovered_layouts.len() > 1 {
        return Err(format!(
            "reopen retry found {} layouts owned by {key}; refusing to choose one",
            recovered_layouts.len()
        ));
    }
    if recovered_workspace_seeds.len() > 1 {
        return Err(format!(
            "reopen retry found {} workspace seeds owned by {key}; refusing to choose one",
            recovered_workspace_seeds.len()
        ));
    }
    Ok(ReopenIntentMatch {
        open_owner,
        layout: recovered_layouts.pop(),
        workspace_seed: recovered_workspace_seeds.pop(),
    })
}

pub(crate) fn inspect_sleep_tab(
    engine: &dyn Engine,
    key: &str,
    context: &ClosedContext,
    owner: &OwnerOpen,
    snapshot: &SessionSnapshotPayload,
) -> Result<Option<(String, String)>, String> {
    let found = find_reopen_intent(engine, key, context, owner, snapshot)?;
    match found.layout {
        Some(layout) if layout.root.pane_count() == 1 => {
            Ok(Some((layout.tab_id, layout.focused_pane_id)))
        }
        Some(_) => Err("The wake tab's layout changed; it was not adopted".into()),
        None => Ok(None),
    }
}

fn ensure_workspace_and_tab(
    engine: &dyn Engine,
    key: &str,
    context: &ClosedContext,
    owner: &OwnerOpen,
    tab_exists: bool,
    root: &ClosedLayoutNode,
    notices: &mut Vec<String>,
) -> Result<crate::recent_closed::ClosedLayout, String> {
    if tab_exists {
        return Err("the requested tab already exists".into());
    }
    let snapshot = fetch_session(engine)
        .map_err(|error| format!("session.snapshot before reopen failed: {}", error.message()))?;
    let found = find_reopen_intent(engine, key, context, owner, &snapshot)?;
    if let Some(layout) = found.layout {
        let layout = repair_incomplete_tab_layout(engine, key, context, root, layout, notices)?;
        return Ok(restore_tab_position(engine, context, layout, notices));
    }
    let (workspace_id, seed_tab_id) = if let Some(seed) = found.workspace_seed {
        (seed.0, Some(seed.1))
    } else if let Some(owner) = found.open_owner {
        (owner, None)
    } else {
        // The owner's first tab is the seed the layout replaces.
        let (workspace_id, first_tab) = ensure_owner(
            engine,
            owner,
            true,
            reopen_intent_env(key, ReopenIntentStage::Workspace),
        )
        .map_err(|failure| failure.message().to_owned())?;
        (workspace_id, first_tab.map(|(tab_id, _, _)| tab_id))
    };
    let tagged_root = tag_reopen_layout(root, key, ReopenIntentStage::Layout);
    let layout = engine
        .apply_layout(
            CONTROL_WAIT,
            &workspace_id,
            seed_tab_id.as_deref(),
            context.herdr_tab_label(),
            &tagged_root,
        )
        .map_err(failed("layout.apply"))?;
    let layout = repair_incomplete_tab_layout(engine, key, context, root, layout, notices)?;
    Ok(restore_tab_position(engine, context, layout, notices))
}

fn repair_incomplete_tab_layout(
    engine: &dyn Engine,
    key: &str,
    context: &ClosedContext,
    root: &ClosedLayoutNode,
    mut layout: crate::recent_closed::ClosedLayout,
    notices: &mut Vec<String>,
) -> Result<crate::recent_closed::ClosedLayout, String> {
    let expected = root.pane_count();
    let first_actual = layout.root.pane_count();
    if first_actual == expected {
        return Ok(layout);
    }
    if first_actual > expected {
        return Err(format!(
            "the reopened tab returned {first_actual} panes, expected {expected}; extra panes were not adopted or overwritten and retry is available"
        ));
    }
    layout = engine
        .apply_layout(
            CONTROL_WAIT,
            &layout.workspace_id,
            Some(&layout.tab_id),
            context.herdr_tab_label(),
            &tag_reopen_layout(root, key, ReopenIntentStage::Layout),
        )
        .map_err(failed("layout.apply"))?;
    let repaired_actual = layout.root.pane_count();
    if repaired_actual > expected {
        return Err(format!(
            "the reopened tab restored {first_actual} of {expected} panes, then returned {repaired_actual} after repair; extra panes were not adopted or overwritten and retry is available"
        ));
    }
    if repaired_actual < expected {
        return Err(format!(
            "the reopened tab restored {first_actual} of {expected} panes, then {repaired_actual} after repair; retry is available"
        ));
    }
    notices.push("The incomplete tab restore was repaired before reopening sessions".into());
    Ok(layout)
}

fn reopen_pane(
    engine: &dyn Engine,
    folders: &dyn Fn(&str) -> bool,
    key: &str,
    context: &ClosedContext,
    pane: &ClosedPane,
    placement: &PanePlacement,
    request: &ReopenRequest,
) -> Result<ReopenOutcome, String> {
    let mut notices = Vec::new();
    let cwd = restored_cwd(folders, &pane.cwd, &context.checkout_path, &mut notices);
    let (restored_pane_id, restored_tab_id) = if request.tab_exists {
        let current_layout = export_reopen_layout(engine, &context.tab_id)?;
        let marker = reopen_intent_marker(key, ReopenIntentStage::Pane);
        let mut recovered_panes = Vec::new();
        pane_ids_with_reopen_marker(&current_layout.root, &marker, &mut recovered_panes);
        if recovered_panes.len() > 1 {
            return Err(format!(
                "reopen retry found {} panes owned by {key}; refusing to choose one",
                recovered_panes.len(),
            ));
        }
        let (new_pane_id, current_target_was_first) = if let Some(pane_id) = recovered_panes.pop() {
            let current = current_layout.root.placement_for(&pane_id).ok_or_else(|| {
                format!("reopen retry could not locate owned pane {pane_id} in its layout")
            })?;
            (pane_id, current.target_was_first)
        } else if let Some(target) = placement.neighbor_pane_id.as_ref() {
            if request.fallback_pane_id.as_deref() != Some(target.as_str()) {
                return Err(format!(
                    "the original sibling pane {target} is no longer available"
                ));
            }
            let pane_id = engine
                .split_pane_at(
                    CONTROL_WAIT,
                    target,
                    placement.direction,
                    &cwd,
                    placement.ratio,
                    reopen_intent_env(key, ReopenIntentStage::Pane),
                )
                .map_err(failed("pane.split"))?;
            (pane_id, false)
        } else {
            let pane_root = tag_reopen_layout(
                &ClosedLayoutNode::Pane {
                    pane_id: None,
                    label: crate::start_claim::public_label(pane.label.as_deref()),
                    cwd: Some(cwd.clone()),
                    command: None,
                    env: Default::default(),
                },
                key,
                ReopenIntentStage::Pane,
            );
            let root = wrap_surviving_subtree(
                current_layout.root.clone(),
                &placement.parent_path,
                pane_root,
                placement.direction,
                placement.ratio,
                placement.target_was_first,
            )?;
            // The tab is still there under the name it has now, which may
            // be a rename made after the close.
            let applied = engine
                .apply_layout(
                    CONTROL_WAIT,
                    &current_layout.workspace_id,
                    Some(&context.tab_id),
                    None,
                    &root,
                )
                .map_err(failed("layout.apply"))?;
            let mut owned = Vec::new();
            pane_ids_with_reopen_marker(&applied.root, &marker, &mut owned);
            if owned.len() != 1 {
                return Err(format!(
                    "reopen layout created {} owned panes instead of one",
                    owned.len()
                ));
            }
            (owned.remove(0), placement.target_was_first)
        };
        if current_target_was_first != placement.target_was_first
            && let Some(target) = placement.neighbor_pane_id.as_ref()
            && let Err(message) = engine
                .swap_panes(CONTROL_WAIT, &new_pane_id, target)
                .map_err(failed("pane.swap"))
        {
            notices.push(format!(
                "The pane reopened but its original side could not be restored: {message}"
            ));
        }
        (new_pane_id, context.tab_id.clone())
    } else {
        let root = ClosedLayoutNode::Pane {
            pane_id: None,
            label: crate::start_claim::public_label(pane.label.as_deref()),
            cwd: Some(cwd.clone()),
            command: None,
            env: Default::default(),
        };
        let layout = ensure_workspace_and_tab(
            engine,
            key,
            context,
            reopen_owner(request)?,
            false,
            &root,
            &mut notices,
        )?;
        (layout.focused_pane_id, layout.tab_id)
    };
    let agents = pane
        .agent
        .as_ref()
        .and_then(|agent| {
            start_or_degrade_agent(
                engine,
                key,
                0,
                &restored_pane_id,
                agent,
                request.codex_daemon,
                &request.permissions,
                &mut notices,
            )
        })
        .into_iter()
        .collect();
    Ok(ReopenOutcome {
        tab_id: Some(restored_tab_id),
        consumed: true,
        focused_pane_id: Some(restored_pane_id.clone()),
        notices: notices
            .into_iter()
            .map(|message| ReopenNotice {
                pane_id: Some(restored_pane_id.clone()),
                message,
            })
            .collect(),
        agents,
    })
}

fn wrap_surviving_subtree(
    surviving: ClosedLayoutNode,
    parent_path: &[ClosedLayoutBranch],
    reopened: ClosedLayoutNode,
    direction: crate::recent_closed::ClosedSplitDirection,
    ratio: f32,
    target_was_first: bool,
) -> Result<ClosedLayoutNode, String> {
    if parent_path.is_empty() {
        let (first, second) = if target_was_first {
            (reopened, surviving)
        } else {
            (surviving, reopened)
        };
        return Ok(ClosedLayoutNode::Split {
            direction,
            ratio,
            first: Box::new(first),
            second: Box::new(second),
        });
    }
    let ClosedLayoutNode::Split {
        direction: surviving_direction,
        ratio: surviving_ratio,
        first,
        second,
    } = surviving
    else {
        return Err("the surviving layout no longer matches the closed pane's parent path".into());
    };
    let (first, second) = match parent_path[0] {
        ClosedLayoutBranch::First => (
            Box::new(wrap_surviving_subtree(
                *first,
                &parent_path[1..],
                reopened,
                direction,
                ratio,
                target_was_first,
            )?),
            second,
        ),
        ClosedLayoutBranch::Second => (
            first,
            Box::new(wrap_surviving_subtree(
                *second,
                &parent_path[1..],
                reopened,
                direction,
                ratio,
                target_was_first,
            )?),
        ),
    };
    Ok(ClosedLayoutNode::Split {
        direction: surviving_direction,
        ratio: surviving_ratio,
        first,
        second,
    })
}

fn reopen_owner(request: &ReopenRequest) -> Result<&OwnerOpen, String> {
    request
        .owner
        .as_ref()
        .ok_or_else(|| "the closed item names no checkout to reopen in".to_owned())
}

#[allow(clippy::too_many_arguments)] // one closed tab with its owner, layout and folders
fn reopen_tab(
    engine: &dyn Engine,
    folders: &dyn Fn(&str) -> bool,
    key: &str,
    context: &ClosedContext,
    owner: &OwnerOpen,
    root: &ClosedLayoutNode,
    panes: &[ClosedPane],
    codex_daemon: crate::codex_launch::CodexDaemon,
    permissions: &crate::agent_permission::AgentPermissions,
) -> Result<ReopenOutcome, String> {
    let pane_map = panes
        .iter()
        .map(|pane| (pane.pane_id.clone(), pane.clone()))
        .collect();
    let mut terminal_ids = Vec::new();
    root.known_pane_ids(&pane_map, &mut terminal_ids);
    let mut common_notices = Vec::new();
    let Some(root) = root.resolve_panes(
        &pane_map,
        &context.checkout_path,
        folders,
        &mut common_notices,
    ) else {
        return Err("the closed tab contained no panes".into());
    };
    let layout = ensure_workspace_and_tab(
        engine,
        key,
        context,
        owner,
        false,
        &root,
        &mut common_notices,
    )?;
    let mut new_ids = Vec::new();
    layout.root.pane_ids(&mut new_ids);
    let terminal_panes = terminal_ids
        .iter()
        .filter_map(|id| pane_map.get(id))
        .collect::<Vec<_>>();
    ensure_complete_tab_restore(terminal_panes.len(), new_ids.len())?;
    let mut notices = Vec::new();
    let mut agents = Vec::new();
    for (index, (pane, new_id)) in terminal_panes.iter().zip(new_ids.iter()).enumerate() {
        let mut pane_notices = Vec::new();
        if !folders(&pane.cwd) {
            pane_notices.push(format!(
                "{} no longer exists; reopened in the checkout root",
                pane.cwd
            ));
        }
        if let Some(agent) = &pane.agent {
            agents.extend(start_or_degrade_agent(
                engine,
                key,
                index,
                new_id,
                agent,
                codex_daemon,
                permissions,
                &mut pane_notices,
            ));
        }
        notices.extend(pane_notices.into_iter().map(|message| ReopenNotice {
            pane_id: Some(new_id.clone()),
            message,
        }));
    }
    let first = new_ids.first().cloned();
    notices.extend(common_notices.into_iter().map(|message| ReopenNotice {
        pane_id: first.clone(),
        message,
    }));
    Ok(ReopenOutcome {
        tab_id: Some(layout.tab_id),
        consumed: true,
        focused_pane_id: first,
        notices,
        agents,
    })
}

fn ensure_complete_tab_restore(expected: usize, actual: usize) -> Result<(), String> {
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "the reopened tab restored {actual} of {expected} terminal panes; retry is available"
        ))
    }
}

fn restored_cwd(
    folders: &dyn Fn(&str) -> bool,
    cwd: &str,
    checkout_root: &str,
    notices: &mut Vec<String>,
) -> String {
    if folders(cwd) {
        cwd.to_owned()
    } else {
        notices.push(format!(
            "{cwd} no longer exists; reopened in the checkout root"
        ));
        checkout_root.to_owned()
    }
}

/// Starts `agent` in a restored pane with its conversation, or a new one when
/// the resume does not take, each with the launch choice it was started
/// with and the machine's permission choice; answers what started, or none
/// when only the shell was kept.
#[allow(clippy::too_many_arguments)] // the pane's identity plus its machine's two start facts
fn start_or_degrade_agent(
    engine: &dyn Engine,
    key: &str,
    index: usize,
    pane_id: &str,
    agent: &ClosedAgent,
    codex_daemon: crate::codex_launch::CodexDaemon,
    permissions: &crate::agent_permission::AgentPermissions,
    notices: &mut Vec<String>,
) -> Option<ReopenedAgent> {
    let name = format!("reopen-{key}-{index}");
    let launch = match crate::runtime::agent_choice::launch_arguments(
        &agent.kind,
        &agent.launch,
        &crate::agent_permission::of(permissions, &agent.kind),
    ) {
        Ok(launch) => launch,
        Err(message) => {
            notices.push(format!(
                "Agent could not be started; the shell was kept: {message}"
            ));
            return None;
        }
    };
    let started = |session_id: Option<&String>| ReopenedAgent {
        pane_id: pane_id.to_owned(),
        kind: agent.kind.clone(),
        name: name.clone(),
        launch: agent.launch.clone(),
        session_id: session_id.cloned(),
    };
    let interrupted_reused_agent = interrupt_reused_agent(engine, pane_id);
    let pane_was_clear = matches!(&interrupted_reused_agent, Ok(false));
    let resume = resume_arguments(agent);
    if let Some(mut args) = resume {
        args.extend(launch.iter().cloned());
        // A resume line is a session id, far under every terminal's line
        // limit, so the restore asks no node.
        let result = start_agent(
            engine,
            None,
            &format!("herdr-core:{key}:agent:{index}:resume"),
            pane_id,
            crate::agent_start::ShellStart::Plain,
            &name,
            &agent.kind,
            args,
            codex_daemon,
        );
        if result.is_ok() || (pane_was_clear && agent_is_running(engine, pane_id, &agent.kind)) {
            return Some(started(agent.session_id.as_ref()));
        }
        let preparation = interrupted_reused_agent
            .as_ref()
            .err()
            .map(|message| format!("; restored pane preparation failed: {message}"))
            .unwrap_or_default();
        notices.push(format!(
            "Session {} could not be resumed{preparation}; started a new conversation",
            agent.session_id.as_deref().unwrap_or("unknown"),
        ));
    } else {
        notices
            .push("Previous conversation could not be resumed; started a new conversation".into());
    }
    if let Err(message) = start_agent(
        engine,
        None,
        &format!("herdr-core:{key}:agent:{index}:fresh"),
        pane_id,
        crate::agent_start::ShellStart::Plain,
        &name,
        &agent.kind,
        launch,
        codex_daemon,
    ) && !(pane_was_clear && agent_is_running(engine, pane_id, &agent.kind))
    {
        notices.push(format!(
            "Agent could not be started; the shell was kept: {message}"
        ));
        return None;
    }
    Some(started(None))
}

/// Herdr can reuse the just-closed pane id while its old PTY is still alive.
/// A layout-only restore then surfaces that old process in the new tab before
/// `agent.start` can apply the explicit resume arguments. Closing the process
/// here returns the restored pane to its shell prompt; the start waits for
/// that prompt (`agent_start`) and `agent.start` remains the single owner of
/// session resumption.
fn interrupt_reused_agent(engine: &dyn Engine, pane_id: &str) -> Result<bool, String> {
    let snapshot = fetch_session(engine).map_err(|error| {
        format!(
            "session.snapshot before agent resume failed: {}",
            error.message()
        )
    })?;
    if !snapshot
        .agents
        .iter()
        .any(|candidate| candidate.pane_id.as_deref() == Some(pane_id))
    {
        return Ok(false);
    }
    engine
        .send_text(CONTROL_WAIT, pane_id, "\u{3}")
        .map_err(failed("pane.send_text"))?;
    Ok(true)
}

fn agent_is_running(engine: &dyn Engine, pane_id: &str, kind: &str) -> bool {
    fetch_session(engine).is_ok_and(|snapshot| {
        snapshot.agents.iter().any(|agent| {
            agent.pane_id.as_deref() == Some(pane_id)
                && agent.agent.as_deref().is_some_and(|agent_kind| {
                    hide_agent_adapter::canonical_kind(agent_kind)
                        .eq_ignore_ascii_case(hide_agent_adapter::canonical_kind(kind))
                })
        })
    })
}

#[cfg(test)]
mod tests {
    use crate::agent_permission::AgentPermission;
    use serde_json::json;
    use std::path::Path;
    use std::sync::Arc;

    use hide_herdr_client::HERDR_PROTOCOL_REVISION;

    use super::*;
    use crate::fake_herdr::FakeHerdr;

    /// A restored agent resumes with the model and effort it was started
    /// with, and the new conversation that replaces a refused resume keeps
    /// them too.
    #[test]
    fn a_restored_agent_starts_with_its_launch_choice_resumed_or_new() {
        let starts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = Arc::clone(&starts);
        let herdr =
            FakeHerdr::start_with_errors("reopen-choice", move |method, params| match method {
                "session.snapshot" => Ok(json!({"type": "session_snapshot", "snapshot": {
                    "version": "fixture", "protocol": HERDR_PROTOCOL_REVISION,
                    "workspaces": [], "tabs": [], "panes": [], "layouts": [], "agents": []
                }})),
                "pane.process_info" => Ok(json!({"type": "pane_process_info", "process_info": {
                    "pane_id": "pane", "shell_pid": 42, "foreground_process_group_id": 42,
                    "foreground_processes": []
                }})),
                "agent.start" => {
                    let mut seen = seen.lock().unwrap();
                    seen.push(params["args"].clone());
                    if seen.len() == 1 {
                        return Err(("agent_start_failed".into(), "resume refused".into()));
                    }
                    Ok(json!({"type": "agent_started", "argv": [], "agent": {
                        "pane_id": "pane", "terminal_id": "term", "workspace_id": "w1",
                        "tab_id": "w1:t1", "focused": false, "agent_status": "idle", "revision": 1
                    }}))
                }
                other => panic!("unexpected {other}"),
            });
        let launch = hide_agent_adapter::LaunchChoice {
            model: Some("gpt-6-luna".into()),
            effort: Some("low".into()),
        };
        let mut notices = Vec::new();
        let started = start_or_degrade_agent(
            herdr.engine().as_ref(),
            "key",
            0,
            "pane",
            &ClosedAgent {
                kind: "codex".into(),
                session_id: Some("session".into()),
                launch: launch.clone(),
            },
            crate::codex_launch::CodexDaemon::Unsupported,
            // No saved choice: full access, on the resume and on the new
            // start that follows its refusal alike.
            &Default::default(),
            &mut notices,
        );
        let choice = [
            "--dangerously-bypass-approvals-and-sandbox",
            "--model",
            "gpt-6-luna",
            "-c",
            "model_reasoning_effort=low",
        ];
        assert_eq!(
            *starts.lock().unwrap(),
            [
                json!([["resume", "session"].as_slice(), &choice].concat()),
                json!(choice)
            ]
        );
        assert_eq!(
            started,
            Some(ReopenedAgent {
                pane_id: "pane".into(),
                kind: "codex".into(),
                name: "reopen-key-0".into(),
                launch,
                session_id: None,
            })
        );
        assert_eq!(
            notices.len(),
            1,
            "the lost conversation is said: {notices:?}"
        );
    }

    #[test]
    fn reopen_canonicalizes_alias_starts_and_confirms_only_the_same_pane_agent() {
        for (kind, canonical, args) in [
            (" CLAUDE-CODE ", "claude", vec!["--resume", "session"]),
            (" CLAUDE ", "claude", vec!["--resume", "session"]),
            (" CLAUDE_CODE ", "claude", vec!["--resume", "session"]),
            (" CODEX ", "codex", vec!["resume", "session"]),
        ] {
            let herdr = FakeHerdr::start("reopen-alias", move |method, params| match method {
                "session.snapshot" => json!({"type": "session_snapshot", "snapshot": {
                    "version": "fixture", "protocol": HERDR_PROTOCOL_REVISION,
                    "workspaces": [], "tabs": [], "panes": [], "layouts": [], "agents": []
                }}),
                "pane.process_info" => json!({"type": "pane_process_info", "process_info": {
                    "pane_id": "pane", "shell_pid": 42, "foreground_process_group_id": 42,
                    "foreground_processes": []
                }}),
                "agent.start" => {
                    assert_eq!(params["kind"], canonical);
                    assert_eq!(params["args"], json!(args));
                    json!({"type": "agent_started", "argv": [], "agent": {
                        "pane_id": "pane", "terminal_id": "term", "workspace_id": "w1",
                        "tab_id": "w1:t1", "focused": false, "agent_status": "idle", "revision": 1
                    }})
                }
                other => panic!("unexpected {other}"),
            });
            let mut notices = Vec::new();
            start_or_degrade_agent(
                herdr.engine().as_ref(),
                "key",
                0,
                "pane",
                &ClosedAgent {
                    kind: kind.into(),
                    session_id: Some("session".into()),
                    ..Default::default()
                },
                crate::codex_launch::CodexDaemon::Unsupported,
                // 매번 물어보기 adds nothing after the resume arguments.
                &[
                    ("claude-code".to_owned(), AgentPermission::Ask),
                    ("codex".to_owned(), AgentPermission::Ask),
                ]
                .into(),
                &mut notices,
            );
            assert!(notices.is_empty(), "{kind}: {notices:?}");
            assert_eq!(
                herdr.methods(),
                ["session.snapshot", "pane.process_info", "agent.start"]
            );
        }
        let herdr = FakeHerdr::start("reopen-alias-confirm", |method, _| match method {
            "session.snapshot" => json!({"type": "session_snapshot", "snapshot": {
                "version": "fixture", "protocol": HERDR_PROTOCOL_REVISION,
                "workspaces": [], "tabs": [], "panes": [], "layouts": [], "agents": [{
                    "pane_id": "pane", "terminal_id": "term", "workspace_id": "w1", "tab_id": "w1:t1",
                    "name": "agent", "agent": "claude", "agent_status": "idle", "focused": false, "revision": 1
                }]
            }}),
            other => panic!("unexpected {other}"),
        });
        assert!(agent_is_running(
            herdr.engine().as_ref(),
            "pane",
            " CLAUDE_CODE "
        ));
        assert!(!agent_is_running(
            herdr.engine().as_ref(),
            "other-pane",
            "claude_code"
        ));
        assert!(!agent_is_running(herdr.engine().as_ref(), "pane", "codex"));
        assert!(!agent_is_running(
            herdr.engine().as_ref(),
            "pane",
            "future-agent"
        ));
    }

    #[test]
    fn nested_split_reopen_applies_the_original_outer_and_inner_geometry() {
        let herdr = FakeHerdr::start("reopen-nested-layout", |method, params| match method {
            "layout.export" => json!({"type":"layout_export","layout": {
                "workspace_id":"w1", "tab_id":"w1:t1", "zoomed":false,
                "focused_pane_id":"outer-left",
                "root":{"type":"split", "direction":"right", "ratio":0.4,
                    "first":{"type":"pane", "pane_id":"outer-left", "cwd":"/tmp", "env":{}},
                    "second":{"type":"split", "direction":"right", "ratio":0.65,
                        "first":{"type":"pane", "pane_id":"inner-left", "cwd":"/tmp", "env":{}},
                        "second":{"type":"pane", "pane_id":"inner-right", "cwd":"/tmp", "env":{}}}}
            }}),
            "layout.apply" => {
                // The tab keeps its own label: Herdr would keep any label
                // sent as the tab's name.
                assert!(params["tab_label"].is_null());
                let root = &params["root"];
                assert_eq!(root["direction"], "right");
                assert!((root["ratio"].as_f64().unwrap() - 0.4).abs() < 0.000_001);
                assert_eq!(root["first"]["pane_id"], "outer-left");
                assert_eq!(root["second"]["direction"], "down");
                assert!((root["second"]["ratio"].as_f64().unwrap() - 0.3).abs() < 0.000_001);
                assert_eq!(
                    root["second"]["first"]["env"][REOPEN_INTENT_ENV],
                    reopen_intent_marker("nested-intent", ReopenIntentStage::Pane)
                );
                assert_eq!(root["second"]["second"]["direction"], "right");
                assert!(
                    (root["second"]["second"]["ratio"].as_f64().unwrap() - 0.65).abs() < 0.000_001
                );
                assert_eq!(root["second"]["second"]["first"]["pane_id"], "inner-left");
                assert_eq!(root["second"]["second"]["second"]["pane_id"], "inner-right");
                json!({"type":"layout_apply","layout": {
                    "workspace_id":"w1", "tab_id":"w1:t1", "zoomed":false,
                    "focused_pane_id":"reopened",
                    "root":{"type":"split", "direction":"right", "ratio":0.4,
                        "first":{"type":"pane", "pane_id":"outer-left", "cwd":"/tmp", "env":{}},
                        "second":{"type":"split", "direction":"down", "ratio":0.3,
                            "first":{"type":"pane", "pane_id":"reopened", "cwd":"/tmp", "env":{
                                (REOPEN_INTENT_ENV): reopen_intent_marker("nested-intent", ReopenIntentStage::Pane)
                            }},
                            "second":{"type":"split", "direction":"right", "ratio":0.65,
                                "first":{"type":"pane", "pane_id":"inner-left", "cwd":"/tmp", "env":{}},
                                "second":{"type":"pane", "pane_id":"inner-right", "cwd":"/tmp", "env":{}}}}}
                }})
            }
            other => panic!("unexpected {other}"),
        });
        let context = ClosedContext {
            workspace_id: "w1".into(),
            workspace_label: "Fixture".into(),
            workspace_ids_before_close: vec!["w1".into()],
            tab_ids_before_close: vec!["w1:t1".into()],
            pane_ids_before_close: vec![
                "outer-left".into(),
                "closed".into(),
                "inner-left".into(),
                "inner-right".into(),
            ],
            checkout_id: "checkout".into(),
            checkout_path: "/tmp".into(),
            tab_id: "w1:t1".into(),
            tab_label: "Agent one".into(),
            tab_index: 0,
            agent_area: None,
            agent_group: None,
            replacement_shell: false,
            replacement_in_tab: false,
            tab_name: crate::recent_closed::ClosedTabName::Unnamed,
        };
        let pane = ClosedPane {
            pane_id: "closed".into(),
            label: None,
            cwd: "/tmp".into(),
            agent: None,
        };
        let placement = PanePlacement {
            neighbor_pane_id: None,
            parent_path: vec![ClosedLayoutBranch::Second],
            direction: crate::recent_closed::ClosedSplitDirection::Down,
            ratio: 0.3,
            target_was_first: true,
        };
        let request = ReopenRequest {
            codex_daemon: Default::default(),
            permissions: Default::default(),
            item: ClosedItem::Pane {
                key: "nested-intent".into(),
                context: context.clone(),
                pane: pane.clone(),
                placement: placement.clone(),
            },
            workspace_exists: true,
            tab_exists: true,
            fallback_pane_id: Some("outer-left".into()),
            owner: None,
        };

        let outcome = reopen_pane(
            herdr.engine().as_ref(),
            &|path| Path::new(path).is_dir(),
            "nested-intent",
            &context,
            &pane,
            &placement,
            &request,
        )
        .expect("nested pane reopens through layout.apply");

        assert_eq!(outcome.focused_pane_id.as_deref(), Some("reopened"));
        assert_eq!(herdr.methods(), ["layout.export", "layout.apply"]);
    }

    #[test]
    fn over_count_tab_layout_is_not_adopted_or_overwritten() {
        let pane = |id: &str| ClosedLayoutNode::Pane {
            pane_id: Some(id.to_owned()),
            label: None,
            cwd: Some("/tmp".to_owned()),
            command: None,
            env: Default::default(),
        };
        let expected_root = ClosedLayoutNode::Split {
            direction: crate::recent_closed::ClosedSplitDirection::Right,
            ratio: 0.5,
            first: Box::new(pane("expected-a")),
            second: Box::new(pane("expected-b")),
        };
        let actual_root = ClosedLayoutNode::Split {
            direction: crate::recent_closed::ClosedSplitDirection::Right,
            ratio: 0.5,
            first: Box::new(expected_root.clone()),
            second: Box::new(pane("unknown-extra")),
        };
        let context = ClosedContext {
            workspace_id: "w1".into(),
            workspace_label: "Fixture".into(),
            workspace_ids_before_close: vec!["w1".into()],
            tab_ids_before_close: vec!["w1:t1".into()],
            pane_ids_before_close: vec!["w1:p1".into(), "w1:p2".into()],
            checkout_id: "checkout".into(),
            checkout_path: "/tmp".into(),
            tab_id: "w1:t1".into(),
            tab_label: "Tab".into(),
            tab_index: 0,
            agent_area: None,
            agent_group: None,
            replacement_shell: false,
            replacement_in_tab: false,
            tab_name: Default::default(),
        };
        let error = repair_incomplete_tab_layout(
            crate::engine::herdr::local("/tmp/hide-reopen-over-count-must-not-connect.sock")
                .as_ref(),
            "over-count",
            &context,
            &expected_root,
            crate::recent_closed::ClosedLayout {
                workspace_id: "w1".into(),
                tab_id: "w1:t2".into(),
                zoomed: false,
                focused_pane_id: "unknown-extra".into(),
                root: actual_root,
            },
            &mut Vec::new(),
        )
        .expect_err("an extra pane is not owned by the reopen intent");

        assert!(error.contains("returned 3 panes, expected 2"));
        assert!(error.contains("not adopted or overwritten"));
        assert!(error.contains("retry is available"));
    }

    #[test]
    fn partial_tab_restore_is_not_consumed_as_success() {
        let error = ensure_complete_tab_restore(3, 2).expect_err("one pane is still missing");
        assert!(error.contains("restored 2 of 3"));
        assert!(error.contains("retry is available"));
        assert!(ensure_complete_tab_restore(3, 3).is_ok());
    }

    // A folder is made untraversable through its mode bits.
    #[cfg(unix)]
    #[test]
    fn file_reopen_distinguishes_metadata_failure_from_a_missing_file() {
        use std::os::unix::fs::PermissionsExt;
        let folder = tempfile::tempdir().expect("fixture folder");
        let root = folder.path().to_path_buf();
        let denied = root.join("denied");
        let file = denied.join("document.txt");
        std::fs::create_dir_all(&denied).expect("create denied fixture");
        std::fs::write(&file, "saved").expect("write fixture");
        std::fs::set_permissions(&denied, std::fs::Permissions::from_mode(0o000))
            .expect("deny traversal");

        let result = run_file_reopen(
            &hide_node::Local::of_process(),
            &crate::files::DocumentRoot {
                device_id: crate::node::TEST_NODE.to_owned(),
                path: root.to_string_lossy().into_owned(),
                identity: None,
            },
            file.to_str().unwrap(),
        );

        // Traversal comes back before the folder is dropped, or it could not
        // be removed.
        std::fs::set_permissions(&denied, std::fs::Permissions::from_mode(0o700))
            .expect("restore traversal");
        match result {
            FileReopenResultOrHerdr::File(FileReopenResult::Failed(message)) => {
                assert!(message.contains("could not be opened"), "{message}");
            }
            other => panic!("permission failure must stay retryable, got {other:?}"),
        }
    }

    #[test]
    fn pane_reopen_ignores_an_unowned_same_cwd_pane() {
        let herdr = FakeHerdr::start("reopen-owned-pane", |method, _| match method {
            "session.snapshot" => json!({"type":"session_snapshot","snapshot": {
                "version":"fixture", "protocol":HERDR_PROTOCOL_REVISION,
                "workspaces":[], "tabs":[], "agents":[],
                "panes":[{"pane_id":"w1:p3", "terminal_id": "fixture-terminal","workspace_id":"w1","tab_id":"w1:t1","cwd":"/tmp","focused":false,"agent_status":"idle","revision":0}],
                "layouts":[{"workspace_id":"w1","tab_id":"w1:t1","zoomed":false,
                    "area":{"x":0,"y":0,"width":80,"height":24}, "focused_pane_id":"w1:p1",
                    "panes":[{"pane_id":"w1:p1","focused":true,"rect":{"x":0,"y":0,"width":40,"height":24}},
                             {"pane_id":"w1:p3","focused":false,"rect":{"x":40,"y":0,"width":40,"height":24}}],
                    "splits":[] }]
            }}),
            "layout.export" => json!({
                "type": "layout_export",
                "layout": {
                    "workspace_id": "w1", "tab_id": "w1:t1", "zoomed": false,
                    "focused_pane_id": "w1:p1",
                    "root": {"type":"split", "direction":"right", "ratio":0.5,
                        "first":{"type":"pane","pane_id":"w1:p1","cwd":"/tmp","env":{}},
                        "second":{"type":"pane","pane_id":"w1:p3","cwd":"/tmp","env":{}}}
                }
            }),
            "pane.split" => json!({"type": "pane_info", "pane": {
                "pane_id": "w1:p4", "terminal_id": "fixture-terminal", "workspace_id": "w1", "tab_id": "w1:t1",
                "focused": true, "agent_status": "idle", "revision": 1
            }}),
            method => panic!("unexpected method {method}"),
        });
        let context = ClosedContext {
            workspace_id: "w1".into(),
            workspace_label: "fixture".into(),
            workspace_ids_before_close: vec!["w1".into()],
            tab_ids_before_close: vec!["w1:t1".into()],
            pane_ids_before_close: vec!["w1:p1".into(), "w1:p2".into()],
            checkout_id: "checkout".into(),
            checkout_path: "/tmp".into(),
            tab_id: "w1:t1".into(),
            tab_label: "tab".into(),
            tab_index: 0,
            agent_area: None,
            agent_group: None,
            replacement_shell: false,
            replacement_in_tab: false,
            tab_name: Default::default(),
        };
        let pane = ClosedPane {
            pane_id: "w1:p2".into(),
            label: None,
            cwd: "/tmp".into(),
            agent: None,
        };
        let placement = PanePlacement {
            neighbor_pane_id: Some("w1:p1".into()),
            parent_path: Vec::new(),
            direction: crate::recent_closed::ClosedSplitDirection::Right,
            ratio: 0.5,
            target_was_first: false,
        };
        let request = ReopenRequest {
            codex_daemon: Default::default(),
            permissions: Default::default(),
            item: ClosedItem::Pane {
                key: "first-attempt".into(),
                context: context.clone(),
                pane: pane.clone(),
                placement: placement.clone(),
            },
            workspace_exists: true,
            tab_exists: true,
            fallback_pane_id: Some("w1:p1".into()),
            owner: None,
        };

        let outcome = reopen_pane(
            herdr.engine().as_ref(),
            &|path| Path::new(path).is_dir(),
            "first-attempt",
            &context,
            &pane,
            &placement,
            &request,
        )
        .expect("fixture reopen succeeds");

        assert_eq!(outcome.focused_pane_id.as_deref(), Some("w1:p4"));
        let requests = herdr.requests();
        let split = requests
            .iter()
            .find(|request| request["method"] == "pane.split")
            .expect("the reopen creates its own pane");
        assert_eq!(
            split["params"]["env"][REOPEN_INTENT_ENV],
            reopen_intent_marker("first-attempt", ReopenIntentStage::Pane)
        );
        assert!(
            requests
                .iter()
                .all(|request| request["method"] != "pane.send_text")
        );
    }

    #[test]
    fn tab_reopen_ignores_unowned_same_label_workspace_and_tab() {
        let mut layout_apply_count = 0;
        let herdr = FakeHerdr::start("reopen-owned-layout", move |method, params| match method {
            "session.snapshot" => json!({"type":"session_snapshot","snapshot": {
                "version":"fixture", "protocol":HERDR_PROTOCOL_REVISION,
                "workspaces":[{"workspace_id":"w2","label":"Fixture","active_tab_id":"w2:t1","number":2,"focused":true,"pane_count":1,"tab_count":1,"agent_status":"idle"}],
                "tabs":[{"workspace_id":"w2","tab_id":"w2:t1","label":"Tab","number":1,"focused":true,"pane_count":1,"agent_status":"idle"}],
                "panes":[], "layouts":[], "agents":[]
            }}),
            "layout.export" => json!({"type":"layout_export","layout": {
                "workspace_id":"w2", "tab_id":"w2:t1", "zoomed":false,
                "focused_pane_id":"w2:p9",
                "root":{"type":"pane","pane_id":"w2:p9","cwd":"/tmp","env":{}}
            }}),
            "workspace.create" => {
                assert_eq!(
                    params["env"][REOPEN_INTENT_ENV],
                    reopen_intent_marker("layout-intent", ReopenIntentStage::Workspace)
                );
                json!({"type":"workspace_created",
                    "workspace":{"workspace_id":"w3","number":3,"label":"Fixture","focused":true,"pane_count":1,"tab_count":1,"active_tab_id":"w3:t1","agent_status":"idle"},
                    "tab":{"tab_id":"w3:t1","workspace_id":"w3","number":1,"label":"1","focused":true,"pane_count":1,"agent_status":"idle"},
                    "root_pane":{"pane_id":"w3:p3", "terminal_id": "fixture-terminal","workspace_id":"w3","tab_id":"w3:t1","focused":true,"agent_status":"idle","revision":1}
                })
            }
            "layout.apply" => {
                assert_eq!(params["tab_id"], "w3:t1");
                assert!(params.get("workspace_id").is_none());
                assert_eq!(
                    params["root"]["first"]["env"][REOPEN_INTENT_ENV],
                    reopen_intent_marker("layout-intent", ReopenIntentStage::Layout)
                );
                layout_apply_count += 1;
                if layout_apply_count == 1 {
                    json!({"type":"layout_apply","layout": {
                        "workspace_id":"w3", "tab_id":"w3:t1", "zoomed":false,
                        "focused_pane_id":"w3:p3",
                        "root":{"type":"pane","pane_id":"w3:p3","cwd":"/tmp","env":{
                            (REOPEN_INTENT_ENV): reopen_intent_marker("layout-intent", ReopenIntentStage::Layout)
                        }}
                    }})
                } else {
                    json!({"type":"layout_apply","layout": {
                        "workspace_id":"w3", "tab_id":"w3:t1", "zoomed":false,
                        "focused_pane_id":"w3:p3",
                        "root":{"type":"split","direction":"right","ratio":0.6,
                            "first":{"type":"pane","pane_id":"w3:p3","cwd":"/tmp","env":{
                                (REOPEN_INTENT_ENV): reopen_intent_marker("layout-intent", ReopenIntentStage::Layout)
                            }},
                            "second":{"type":"pane","pane_id":"w3:p4","cwd":"/tmp","env":{
                                (REOPEN_INTENT_ENV): reopen_intent_marker("layout-intent", ReopenIntentStage::Layout)
                            }}}
                    }})
                }
            }
            "workspace.list" => json!({"type":"workspace_list","workspaces":[
                {"workspace_id":"w2","number":2,"label":"other","focused":true,"pane_count":1,"tab_count":1,"active_tab_id":"w2:t1","agent_status":"idle"}
            ]}),
            "workspace.report_metadata" => json!({"type":"ok"}),
            _ => unreachable!(),
        });
        let context = ClosedContext {
            workspace_id: "w1".into(),
            workspace_label: "Fixture".into(),
            workspace_ids_before_close: vec!["w1".into()],
            tab_ids_before_close: vec!["w1:t1".into()],
            pane_ids_before_close: vec!["w1:p1".into()],
            checkout_id: "checkout".into(),
            checkout_path: "/tmp".into(),
            tab_id: "w1:t1".into(),
            tab_label: "Tab".into(),
            tab_index: 0,
            agent_area: None,
            agent_group: None,
            replacement_shell: false,
            replacement_in_tab: false,
            tab_name: Default::default(),
        };
        let layout_root = ClosedLayoutNode::Split {
            direction: crate::recent_closed::ClosedSplitDirection::Right,
            ratio: 0.6,
            first: Box::new(ClosedLayoutNode::Pane {
                pane_id: None,
                label: None,
                cwd: Some("/tmp".into()),
                command: None,
                env: Default::default(),
            }),
            second: Box::new(ClosedLayoutNode::Pane {
                pane_id: None,
                label: None,
                cwd: Some("/tmp".into()),
                command: None,
                env: Default::default(),
            }),
        };
        let mut notices = Vec::new();
        let restored = ensure_workspace_and_tab(
            herdr.engine().as_ref(),
            "layout-intent",
            &context,
            &OwnerOpen::Folder {
                path: "/tmp".into(),
                label: "Fixture".into(),
                mark: crate::checkout_owner::owner_mark(crate::node::TEST_NODE, "/tmp"),
                legacy_mark: None,
            },
            false,
            &layout_root,
            &mut notices,
        )
        .expect("owned layout is created");

        assert_eq!(restored.workspace_id, "w3");
        assert_eq!(restored.tab_id, "w3:t1");
        assert_eq!(restored.focused_pane_id, "w3:p3");
        assert_eq!(restored.root.pane_count(), 2);
        assert_eq!(
            notices,
            ["The incomplete tab restore was repaired before reopening sessions"]
        );
        assert_eq!(
            herdr.methods(),
            // The closed tab's workspace is gone, so it reopens in the
            // folder's owner, made and marked here; the old index names a
            // place in another workspace and is not restored.
            [
                "session.snapshot",
                "layout.export",
                "workspace.list",
                "workspace.create",
                "workspace.report_metadata",
                "layout.apply",
                "layout.apply"
            ]
        );
    }

    #[test]
    fn recovered_partial_tab_layout_is_repaired_and_revalidated() {
        let herdr = FakeHerdr::start(
            "reopen-repair-recovered",
            move |method, params| match method {
                "session.snapshot" => json!({"type":"session_snapshot","snapshot": {
                    "version":"fixture", "protocol":HERDR_PROTOCOL_REVISION,
                    "workspaces":[{"workspace_id":"w1","label":"Fixture","active_tab_id":"w1:t2","number":1,"focused":true,"pane_count":1,"tab_count":1,"agent_status":"idle",
                        "worktree":{"repo_key":"/tmp","repo_name":"tmp","repo_root":"/tmp","checkout_path":"/tmp","is_linked_worktree":false}}],
                    "tabs":[{"workspace_id":"w1","tab_id":"w1:t2","label":"Tab","number":1,"focused":true,"pane_count":1,"agent_status":"idle"}],
                    "panes":[], "layouts":[], "agents":[]
                }}),
                "layout.export" => json!({"type":"layout_export","layout": {
                    "workspace_id":"w1", "tab_id":"w1:t2", "zoomed":false,
                    "focused_pane_id":"w1:p2",
                    "root":{"type":"pane","pane_id":"w1:p2","cwd":"/tmp","env":{
                        (REOPEN_INTENT_ENV): reopen_intent_marker("recovered-intent", ReopenIntentStage::Layout)
                    }}
                }}),
                "layout.apply" => {
                    assert_eq!(params["tab_id"], "w1:t2");
                    assert_eq!(
                        params["root"]["second"]["env"][REOPEN_INTENT_ENV],
                        reopen_intent_marker("recovered-intent", ReopenIntentStage::Layout)
                    );
                    json!({"type":"layout_apply","layout": {
                        "workspace_id":"w1", "tab_id":"w1:t2", "zoomed":false,
                        "focused_pane_id":"w1:p2",
                        "root":{"type":"split","direction":"right","ratio":0.6,
                            "first":{"type":"pane","pane_id":"w1:p2","cwd":"/tmp","env":{
                                (REOPEN_INTENT_ENV): reopen_intent_marker("recovered-intent", ReopenIntentStage::Layout)
                            }},
                            "second":{"type":"pane","pane_id":"w1:p3","cwd":"/tmp","env":{
                                (REOPEN_INTENT_ENV): reopen_intent_marker("recovered-intent", ReopenIntentStage::Layout)
                            }}}
                    }})
                }
                "tab.move" => json!({"type":"tab_list","tabs":[]}),
                _ => unreachable!(),
            },
        );
        let context = ClosedContext {
            workspace_id: "w1".into(),
            workspace_label: "Fixture".into(),
            workspace_ids_before_close: vec!["w1".into()],
            tab_ids_before_close: vec!["w1:t1".into()],
            pane_ids_before_close: vec!["w1:p1".into()],
            checkout_id: "checkout".into(),
            checkout_path: "/tmp".into(),
            tab_id: "w1:t1".into(),
            tab_label: "Tab".into(),
            tab_index: 0,
            agent_area: None,
            agent_group: None,
            replacement_shell: false,
            replacement_in_tab: false,
            tab_name: Default::default(),
        };
        let root = ClosedLayoutNode::Split {
            direction: crate::recent_closed::ClosedSplitDirection::Right,
            ratio: 0.6,
            first: Box::new(ClosedLayoutNode::Pane {
                pane_id: None,
                label: None,
                cwd: Some("/tmp".into()),
                command: None,
                env: Default::default(),
            }),
            second: Box::new(ClosedLayoutNode::Pane {
                pane_id: None,
                label: None,
                cwd: Some("/tmp".into()),
                command: None,
                env: Default::default(),
            }),
        };
        let mut notices = Vec::new();
        let restored = ensure_workspace_and_tab(
            herdr.engine().as_ref(),
            "recovered-intent",
            &context,
            &OwnerOpen::Worktree {
                path: "/tmp".into(),
                repository_root: "/tmp".into(),
                label: "Fixture".into(),
            },
            false,
            &root,
            &mut notices,
        )
        .expect("the recovered layout is repaired");

        assert_eq!(restored.tab_id, "w1:t2");
        assert_eq!(restored.root.pane_count(), 2);
        assert_eq!(
            notices,
            ["The incomplete tab restore was repaired before reopening sessions"]
        );
        assert_eq!(
            herdr.methods(),
            [
                "session.snapshot",
                "layout.export",
                "layout.apply",
                "tab.move"
            ]
        );
    }

    #[test]
    fn reused_agent_process_is_interrupted_before_resume() {
        let herdr = FakeHerdr::start("reopen-interrupt", |method, params| match method {
            "session.snapshot" => json!({
                "type": "session_snapshot",
                "snapshot": {
                    "version": "fixture",
                    "protocol": HERDR_PROTOCOL_REVISION,
                    "workspaces": [],
                    "tabs": [],
                    "panes": [],
                    "layouts": [],
                    "agents": [{
                        "terminal_id": "term-1",
                        "name": "old-agent",
                        "agent": "claude",
                        "agent_status": "idle",
                        "workspace_id": "w1",
                        "tab_id": "w1:t1",
                        "pane_id": "w1:p1",
                        "focused": false,
                        "interactive_ready": true,
                        "state_change_seq": 1,
                        "cwd": "/tmp",
                        "foreground_cwd": "/tmp",
                        "revision": 0
                    }]
                }
            }),
            "pane.send_text" => {
                assert_eq!(*params, json!({"pane_id": "w1:p1", "text": "\u{3}"}));
                json!({"type": "ok"})
            }
            other => panic!("unexpected {other}"),
        });

        assert!(
            interrupt_reused_agent(herdr.engine().as_ref(), "w1:p1")
                .expect("reused agent is interrupted")
        );

        assert_eq!(herdr.methods(), ["session.snapshot", "pane.send_text"]);
    }
}
