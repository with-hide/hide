//! What the own engine holds of its session (PRD engine-bundle-3-core-owns
//! D-3): its workspaces in order, each workspace's tabs in order and the
//! checkout it is bound to, each tab's pane tree, focus, zoom and panes, the
//! keyboard's workspace, Hide's tokens, and the counters ids are given from.
//! It is plain data behind the engine's own lock, never the runtime's, and
//! starts and ends no program: the engine reserves an id here, starts the
//! pane's holder outside the lock, and adds the pane only once it runs.
//!
//! Every change follows the tree rules of [`super::layout`], the ones the
//! drawing ahead of an engine predicts with, so a layout drawn before this
//! engine answered is the layout it then has. A pane keeps its id wherever
//! it moves, so a letter, a mission or a parent that names it still finds it.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use hide_agent_adapter::definition::Catalog;
use hide_node_link::holders::PaneFacts;
use serde_json::Value;

use super::agents::{self, PaneAgent, PaneLive, Status};
use super::ids;
use super::layout::{
    contains, first_leaf, lay_out, remove_leaf, resized, set_nearest_split_ratio, split_leaf_at,
    swap_leaves,
};
use super::store::{
    FORMAT, Saved, SavedAgent, SavedDirection, SavedNode, SavedPane, SavedTab, SavedWorkspace,
    SavedWorktree,
};
use crate::engine::{EngineError, PaneJoinResult, PaneSessionKind};
use crate::live::PaneResizeDirection;
use crate::model::{PaneLayoutDirection, PaneLayoutNodeSnapshot};
use crate::recent_closed::{ClosedLayout, ClosedLayoutNode, ClosedSplitDirection};
use crate::session_sync::{
    ProjectedAgent, ProjectedPane, ProjectedTab, ProjectedWorkspace, ProjectedWorktree,
    ProjectionState,
};
use crate::sidebar::{
    SessionAgentSessionPayload, SessionLayoutPayload, SessionLayoutRect, SessionTabFocus,
    SessionTabMoves,
};

/// The area a tab is laid out in for its session's rects. The core reads a
/// tab's tree back from them (`live::project_layout`) and draws it at the
/// screen's own size, so only the arithmetic has to agree; the largest area
/// keeps the most splits apart.
const NOMINAL_AREA: SessionLayoutRect = SessionLayoutRect {
    x: 0,
    y: 0,
    width: u16::MAX,
    height: u16::MAX,
};

/// Why a pane move changed nothing, as Herdr words it: the caller reads it
/// as a move declined, not failed.
const SAME_TAB: &str = "same_tab";
const ZOOMED_TAB: &str = "zoomed_tab";

#[derive(Default)]
pub(super) struct Topology {
    workspaces: Vec<Workspace>,
    focused_workspace: Option<String>,
    /// The last workspace number given.
    workspace_numbers: usize,
    /// The tab the keyboard's workspace shows, each time it changed, in the
    /// order it changed, as Herdr's event stream reports its own moves: the
    /// core answers the tab moves it asked for from these, never from a
    /// wait ([`Topology::note_shown`]).
    shown: Shown,
    /// The engine's count of agent state changes, kept in memory only: a
    /// sequence is compared only with one this engine gave, as Herdr's is
    /// with its own server's.
    agent_seq: u64,
}

/// The shown tab's moves and the last of them, kept in memory only: a new
/// engine starts its count again, as a new Herdr replica does.
#[derive(Default)]
struct Shown {
    moves: SessionTabMoves,
    last: Option<SessionTabFocus>,
    /// The tab a creation made and focused since the last move recorded.
    created: Option<String>,
}

struct Workspace {
    id: String,
    label: String,
    tokens: BTreeMap<String, Value>,
    /// The checkout the workspace was opened for, if it was.
    worktree: Option<Bound>,
    tabs: Vec<Tab>,
    active_tab: String,
    /// The last tab and pane numbers given in this workspace.
    tab_numbers: usize,
    pane_numbers: usize,
}

struct Tab {
    id: String,
    label: String,
    root: PaneLayoutNodeSnapshot,
    focused: String,
    zoomed: bool,
    panes: BTreeMap<String, Pane>,
}

/// One pane as the engine holds it.
#[derive(Clone, Debug, Default)]
pub(super) struct Pane {
    /// The folder it was started in, in the wire spelling.
    pub(super) cwd: String,
    pub(super) label: Option<String>,
    pub(super) tokens: BTreeMap<String, Value>,
    /// What it was started with, given back when its tab is exported; kept
    /// in memory only, since a pane's environment can carry what is not the
    /// session's to save.
    pub(super) command: Option<Vec<String>>,
    pub(super) env: BTreeMap<String, String>,
    /// The agent the pane runs and its session, saved for a restore.
    pub(super) agent: Option<PaneAgent>,
    /// What its holder last said of it.
    pub(super) live: PaneLive,
    /// Why no holder runs the pane, once a restore could not start one.
    pub(super) failed: Option<String>,
    /// Its holder is being started by the call that drew it (a new tab or
    /// split is drawn before its holder starts, as Herdr's pane exists
    /// once it forked). Whoever takes the pane out meanwhile leaves its
    /// holder to that call, which ends it once it finds the pane gone.
    /// Kept in memory only.
    pub(super) starting: Option<std::sync::Arc<super::starts::Starting>>,
}

impl Pane {
    pub(super) fn is_starting(&self) -> bool {
        self.starting.as_ref().is_some_and(|start| start.pending())
    }

    pub(super) fn new(cwd: &str, label: Option<&str>) -> Self {
        Self {
            cwd: cwd.to_owned(),
            label: label.map(str::to_owned),
            ..Self::default()
        }
    }

    /// A pane drawn before its holder starts, with `command` and `env`: its
    /// `starting` field says so.
    pub(super) fn starting(
        cwd: &str,
        label: Option<&str>,
        command: Option<&[String]>,
        env: &BTreeMap<String, String>,
    ) -> (Self, std::sync::Arc<super::starts::Starting>) {
        let starting = std::sync::Arc::default();
        (
            Self {
                starting: Some(std::sync::Arc::clone(&starting)),
                ..Self::started(cwd, label, command, env)
            },
            starting,
        )
    }

    /// A pane started with `command` and `env`, which its export gives back.
    pub(super) fn started(
        cwd: &str,
        label: Option<&str>,
        command: Option<&[String]>,
        env: &BTreeMap<String, String>,
    ) -> Self {
        Self {
            command: command.map(<[String]>::to_vec),
            env: env.clone(),
            ..Self::new(cwd, label)
        }
    }
}

/// The checkout a workspace is bound to, in the wire spelling.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Bound {
    /// The repository's shared `.git` folder, which every checkout of it
    /// names alike.
    pub(super) repo_key: String,
    pub(super) repo_name: String,
    pub(super) repo_root: String,
    pub(super) checkout_path: String,
    pub(super) linked: bool,
}

/// The ids a new workspace will have once its first pane runs.
pub(super) struct NewWorkspace {
    pub(super) workspace: String,
    pub(super) tab: String,
    pub(super) pane: String,
}

/// What closing a pane closed with it.
#[derive(Debug, Default, Eq, PartialEq)]
pub(super) struct Closed {
    pub(super) tab: Option<String>,
    pub(super) workspace: Option<String>,
}

/// A workspace bound to a checkout, as an open of that checkout answers it.
pub(super) struct BoundWorkspace {
    pub(super) workspace: String,
    pub(super) label: String,
    pub(super) tab: String,
    pub(super) tab_label: String,
    pub(super) pane: String,
}

/// A layout to apply, its new panes reserved: the tree with every leaf
/// named, which leaves are new (with how each starts) and which the tab it
/// replaces already runs.
pub(super) struct Planned {
    pub(super) workspace: String,
    pub(super) tab: String,
    pub(super) label: String,
    pub(super) root: PaneLayoutNodeSnapshot,
    pub(super) new: Vec<(String, Pane)>,
    pub(super) kept: BTreeSet<String>,
}

pub(super) fn refused(code: &str, message: String) -> EngineError {
    EngineError::Refused {
        code: code.to_owned(),
        message,
    }
}

fn no_workspace(workspace: &str) -> EngineError {
    refused(
        "workspace_not_found",
        format!("workspace {workspace} does not exist"),
    )
}

fn no_tab(tab: &str) -> EngineError {
    refused("tab_not_found", format!("tab {tab} does not exist"))
}

fn no_pane(pane: &str) -> EngineError {
    refused("pane_not_found", format!("pane {pane} does not exist"))
}

impl Topology {
    /// Gives the next workspace number, with its first tab's and pane's ids.
    pub(super) fn reserve_workspace(&mut self) -> NewWorkspace {
        self.workspace_numbers += 1;
        let workspace = ids::workspace_id(self.workspace_numbers);
        NewWorkspace {
            tab: ids::tab_id(&workspace, 1),
            pane: ids::pane_id(&workspace, 1),
            workspace,
        }
    }

    /// Adds the workspace `ids` reserved, with its first pane running.
    pub(super) fn add_workspace(
        &mut self,
        ids: &NewWorkspace,
        label: &str,
        pane: Pane,
        focus: bool,
    ) {
        self.workspaces.push(Workspace {
            id: ids.workspace.clone(),
            label: label.to_owned(),
            tokens: BTreeMap::new(),
            worktree: None,
            tabs: vec![Tab::new(&ids.tab, "", &ids.pane, pane)],
            active_tab: ids.tab.clone(),
            tab_numbers: 1,
            pane_numbers: 1,
        });
        if focus || self.focused_workspace.is_none() {
            self.focused_workspace = Some(ids.workspace.clone());
            self.shown.created = Some(ids.tab.clone());
        }
    }

    /// Gives a new pane id in `workspace`.
    pub(super) fn reserve_pane(&mut self, workspace: &str) -> Result<String, EngineError> {
        let workspace = self.workspace_mut(workspace)?;
        workspace.pane_numbers += 1;
        Ok(ids::pane_id(&workspace.id, workspace.pane_numbers))
    }

    /// Gives a new tab id in `workspace`, with its first pane's id.
    pub(super) fn reserve_tab(&mut self, workspace: &str) -> Result<(String, String), EngineError> {
        let tab = self.reserve_tab_only(workspace)?;
        Ok((tab, self.reserve_pane(workspace)?))
    }

    /// Gives a new tab id in `workspace` for panes it already has.
    fn reserve_tab_only(&mut self, workspace: &str) -> Result<String, EngineError> {
        let workspace = self.workspace_mut(workspace)?;
        workspace.tab_numbers += 1;
        Ok(ids::tab_id(&workspace.id, workspace.tab_numbers))
    }

    /// Adds the tab `reserve_tab` gave, last in its workspace, with its
    /// first pane running.
    pub(super) fn add_tab(
        &mut self,
        workspace: &str,
        (tab, pane_id): (&str, &str),
        label: &str,
        pane: Pane,
        focus: bool,
    ) -> Result<(), EngineError> {
        let found = self.workspace_mut(workspace)?;
        found.tabs.push(Tab::new(tab, label, pane_id, pane));
        if focus {
            found.active_tab = tab.to_owned();
            self.focused_workspace = Some(workspace.to_owned());
            self.shown.created = Some(tab.to_owned());
        }
        Ok(())
    }

    /// The tab the keyboard's workspace shows.
    fn shown_tab(&self) -> Option<(&str, &str)> {
        let workspace = self.focused_workspace.as_deref()?;
        let found = self.workspace(workspace).ok()?;
        Some((workspace, found.active_tab.as_str()))
    }

    /// Records a move when the shown tab is not the one last recorded, as
    /// Herdr publishes `tab_focused` for every tab it moves to and none for
    /// a focus of the tab it shows. The engine calls it as each hold of its
    /// lock ends, so every change, from any caller, is recorded under the
    /// lock that made it and in the order they were made.
    pub(super) fn note_shown(&mut self) {
        let created = self.shown.created.take();
        let Some((workspace, tab)) = self.shown_tab() else {
            return;
        };
        let moved = self
            .shown
            .last
            .as_ref()
            .is_none_or(|last| last.workspace_id != workspace || last.tab_id != tab);
        if !moved {
            return;
        }
        let (workspace, tab) = (workspace.to_owned(), tab.to_owned());
        self.shown.moves.record(tab.clone());
        self.shown.last = Some(SessionTabFocus {
            generation: self.shown.moves.generation,
            workspace_id: workspace,
            revision: self.shown.moves.applied,
            creation: created.as_deref() == Some(tab.as_str()),
            tab_id: tab,
        });
    }

    /// Splits `target` along `direction`, `created` second, `target` keeping
    /// `ratio` of the room.
    pub(super) fn add_split(
        &mut self,
        target: &str,
        direction: PaneLayoutDirection,
        ratio: f32,
        created: &str,
        pane: Pane,
        focus: bool,
    ) -> Result<(), EngineError> {
        let (workspace_index, tab_index) = self.position(target)?;
        let workspace = &mut self.workspaces[workspace_index];
        let tab = &mut workspace.tabs[tab_index];
        split_leaf_at(&mut tab.root, target, direction, created, ratio);
        tab.panes.insert(created.to_owned(), pane);
        // A split shows its new pane, so no zoom survives it.
        tab.zoomed = false;
        if focus {
            tab.focused = created.to_owned();
            workspace.active_tab = tab.id.clone();
            self.focused_workspace = Some(workspace.id.clone());
        }
        Ok(())
    }

    /// Takes `pane` out: its sibling takes its parent's place, the last pane
    /// of a tab takes the tab and the last tab of a workspace the workspace.
    pub(super) fn remove_pane(&mut self, pane: &str) -> Result<Closed, EngineError> {
        let (workspace_index, tab_index) = self.position(pane)?;
        Ok(self.detach(workspace_index, tab_index, pane).1)
    }

    /// Takes `pane` out of the tab at those indexes: the pane and what its
    /// leaving closed.
    fn detach(
        &mut self,
        workspace_index: usize,
        tab_index: usize,
        pane: &str,
    ) -> (Option<Pane>, Closed) {
        let workspace = &mut self.workspaces[workspace_index];
        let tab = &mut workspace.tabs[tab_index];
        let taken = tab.panes.remove(pane);
        if let Some(sibling) = remove_leaf(&mut tab.root, pane) {
            if tab.focused == pane {
                tab.focused = sibling;
                tab.zoomed = false;
            }
            return (taken, Closed::default());
        }
        let tab = tab.id.clone();
        let workspace = self.remove_tab_at(workspace_index, tab_index);
        (
            taken,
            Closed {
                tab: Some(tab),
                workspace,
            },
        )
    }

    /// Takes the tab out, and its workspace with it when it was the last:
    /// the closed workspace's id, if it closed.
    pub(super) fn remove_tab(&mut self, tab: &str) -> Result<Option<String>, EngineError> {
        let (workspace_index, tab_index) = self.tab_position(tab)?;
        Ok(self.remove_tab_at(workspace_index, tab_index))
    }

    fn remove_tab_at(&mut self, workspace_index: usize, tab_index: usize) -> Option<String> {
        let workspace = &mut self.workspaces[workspace_index];
        let removed = workspace.tabs.remove(tab_index);
        if workspace.tabs.is_empty() {
            let id = workspace.id.clone();
            self.workspaces.remove(workspace_index);
            if self.focused_workspace.as_deref() == Some(id.as_str()) {
                self.focused_workspace = self
                    .workspaces
                    .get(workspace_index.min(self.workspaces.len().saturating_sub(1)))
                    .map(|workspace| workspace.id.clone());
            }
            return Some(id);
        }
        if workspace.active_tab == removed.id {
            let next = tab_index.min(workspace.tabs.len() - 1);
            workspace.active_tab = workspace.tabs[next].id.clone();
        }
        None
    }

    /// Takes the workspace out with every tab it holds.
    pub(super) fn remove_workspace(&mut self, workspace: &str) -> Result<(), EngineError> {
        let index = self
            .workspaces
            .iter()
            .position(|found| found.id == workspace)
            .ok_or_else(|| no_workspace(workspace))?;
        self.workspaces.remove(index);
        if self.focused_workspace.as_deref() == Some(workspace) {
            self.focused_workspace = self
                .workspaces
                .get(index.min(self.workspaces.len().saturating_sub(1)))
                .map(|found| found.id.clone());
        }
        Ok(())
    }

    pub(super) fn tab_label(&self, tab: &str) -> Result<String, EngineError> {
        let (workspace, index) = self.tab_position(tab)?;
        Ok(self.workspaces[workspace].tabs[index].label.clone())
    }

    /// The panes of a tab, in its tree's order.
    pub(super) fn tab_panes(&self, tab: &str) -> Result<Vec<String>, EngineError> {
        let (workspace, index) = self.tab_position(tab)?;
        Ok(leaves(&self.workspaces[workspace].tabs[index].root))
    }

    /// The panes of a workspace, in its tabs' and trees' order.
    pub(super) fn workspace_panes(&self, workspace: &str) -> Result<Vec<String>, EngineError> {
        Ok(self
            .workspace(workspace)?
            .tabs
            .iter()
            .flat_map(|tab| leaves(&tab.root))
            .collect())
    }

    /// Every pane, by id.
    pub(super) fn panes(&self) -> impl Iterator<Item = (&str, &Pane)> {
        self.workspaces.iter().flat_map(|workspace| {
            workspace
                .tabs
                .iter()
                .flat_map(|tab| tab.panes.iter().map(|(id, pane)| (id.as_str(), pane)))
        })
    }

    pub(super) fn pane(&self, pane: &str) -> Result<&Pane, EngineError> {
        let (workspace, tab) = self.position(pane)?;
        self.workspaces[workspace].tabs[tab]
            .panes
            .get(pane)
            .ok_or_else(|| no_pane(pane))
    }

    pub(super) fn pane_mut(&mut self, pane: &str) -> Result<&mut Pane, EngineError> {
        let (workspace, tab) = self.position(pane)?;
        self.workspaces[workspace].tabs[tab]
            .panes
            .get_mut(pane)
            .ok_or_else(|| no_pane(pane))
    }

    /// The workspace and tab that hold `pane`.
    pub(super) fn pane_place(&self, pane: &str) -> Result<(String, String), EngineError> {
        let (workspace, tab) = self.position(pane)?;
        let workspace = &self.workspaces[workspace];
        Ok((workspace.id.clone(), workspace.tabs[tab].id.clone()))
    }

    /// Whether `tab` is in `workspace`.
    pub(super) fn tab_in(&self, workspace: &str, tab: &str) -> Result<bool, EngineError> {
        Ok(self
            .workspace(workspace)?
            .tabs
            .iter()
            .any(|found| found.id == tab))
    }

    pub(super) fn active_tab(&self, workspace: &str) -> Result<String, EngineError> {
        Ok(self.workspace(workspace)?.active_tab.clone())
    }

    pub(super) fn focused_workspace(&self) -> Option<String> {
        self.focused_workspace.clone()
    }

    pub(super) fn workspace_tokens(&self, token: &str) -> Vec<(String, Option<String>)> {
        self.workspaces
            .iter()
            .map(|workspace| {
                (
                    workspace.id.clone(),
                    workspace
                        .tokens
                        .get(token)
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                )
            })
            .collect()
    }

    pub(super) fn set_workspace_token(
        &mut self,
        workspace: &str,
        token: &str,
        value: Option<&str>,
    ) -> Result<(), EngineError> {
        let tokens = &mut self.workspace_mut(workspace)?.tokens;
        match value {
            Some(value) => tokens.insert(token.to_owned(), Value::String(value.to_owned())),
            None => tokens.remove(token),
        };
        Ok(())
    }

    pub(super) fn pane_token(
        &self,
        pane: &str,
        token: &str,
    ) -> Result<Option<String>, EngineError> {
        Ok(self
            .pane(pane)?
            .tokens
            .get(token)
            .and_then(Value::as_str)
            .map(str::to_owned))
    }

    pub(super) fn set_pane_tokens(
        &mut self,
        pane: &str,
        tokens: &BTreeMap<String, Option<String>>,
    ) -> Result<(), EngineError> {
        let held = &mut self.pane_mut(pane)?.tokens;
        for (token, value) in tokens {
            match value {
                Some(value) => held.insert(token.clone(), Value::String(value.clone())),
                None => held.remove(token),
            };
        }
        Ok(())
    }

    pub(super) fn rename_workspace(
        &mut self,
        workspace: &str,
        label: &str,
    ) -> Result<(), EngineError> {
        self.workspace_mut(workspace)?.label = label.to_owned();
        Ok(())
    }

    pub(super) fn rename_tab(&mut self, tab: &str, label: &str) -> Result<(), EngineError> {
        let (workspace, index) = self.tab_position(tab)?;
        self.workspaces[workspace].tabs[index].label = label.to_owned();
        Ok(())
    }

    pub(super) fn rename_pane(
        &mut self,
        pane: &str,
        label: Option<&str>,
    ) -> Result<(), EngineError> {
        self.pane_mut(pane)?.label = label.map(str::to_owned);
        Ok(())
    }

    pub(super) fn focus_workspace(&mut self, workspace: &str) -> Result<(), EngineError> {
        self.workspace(workspace)?;
        self.focused_workspace = Some(workspace.to_owned());
        Ok(())
    }

    pub(super) fn focus_tab(&mut self, tab: &str) -> Result<(), EngineError> {
        let (workspace, _) = self.tab_position(tab)?;
        let workspace = &mut self.workspaces[workspace];
        workspace.active_tab = tab.to_owned();
        self.focused_workspace = Some(workspace.id.clone());
        Ok(())
    }

    pub(super) fn focus_pane(&mut self, pane: &str) -> Result<(), EngineError> {
        let (workspace_index, tab_index) = self.position(pane)?;
        let workspace = &mut self.workspaces[workspace_index];
        let tab = &mut workspace.tabs[tab_index];
        tab.focused = pane.to_owned();
        workspace.active_tab = tab.id.clone();
        self.focused_workspace = Some(workspace.id.clone());
        Ok(())
    }

    /// Zooms `pane`'s tab onto it, or ends the zoom; `None` turns it the
    /// other way. Either way the pane takes the tab's focus, as the drawing
    /// ahead predicts. A tab of one pane is never zoomed, as Herdr answers it
    /// unchanged. Whether anything changed.
    pub(super) fn zoom(&mut self, pane: &str, zoomed: Option<bool>) -> Result<bool, EngineError> {
        let (workspace, index) = self.position(pane)?;
        let tab = &mut self.workspaces[workspace].tabs[index];
        if matches!(tab.root, PaneLayoutNodeSnapshot::Pane { .. }) {
            return Ok(false);
        }
        let zoomed = zoomed.unwrap_or(!tab.zoomed);
        let changed = tab.zoomed != zoomed || tab.focused != pane;
        tab.zoomed = zoomed;
        tab.focused = pane.to_owned();
        Ok(changed)
    }

    /// Moves `pane`'s edge by `amount` toward `direction` (layout's
    /// `resized`): whether a split moved.
    pub(super) fn resize(
        &mut self,
        pane: &str,
        direction: PaneResizeDirection,
        amount: f32,
    ) -> Result<bool, EngineError> {
        let (workspace, index) = self.position(pane)?;
        let tab = &mut self.workspaces[workspace].tabs[index];
        let Some((axis, ratio)) = resized(&tab.root, pane, direction, amount) else {
            return Ok(false);
        };
        let before = tab.root.clone();
        set_nearest_split_ratio(&mut tab.root, pane, axis, ratio);
        Ok(tab.root != before)
    }

    /// Trades the places of two panes of one tab; each keeps its id, its
    /// program and whether it has the focus.
    pub(super) fn swap(&mut self, first: &str, second: &str) -> Result<(), EngineError> {
        let (workspace, index) = self.position(first)?;
        if self.position(second)? != (workspace, index) {
            return Err(refused(
                "panes_in_different_tabs",
                format!("panes {first} and {second} are not in one tab"),
            ));
        }
        if first == second {
            return Err(refused(
                "same_pane",
                format!("pane {first} cannot change places with itself"),
            ));
        }
        swap_leaves(
            &mut self.workspaces[workspace].tabs[index].root,
            first,
            second,
        );
        Ok(())
    }

    /// Moves `pane`, unfocused, into a new tab of `workspace` named `label`.
    /// A pane of a zoomed tab is not moved: the answer says so.
    pub(super) fn move_to_new_tab(
        &mut self,
        pane: &str,
        workspace: &str,
        label: &str,
    ) -> Result<PaneJoinResult, EngineError> {
        let (from_workspace, from_tab) = self.position(pane)?;
        let previous = self.place_of(from_workspace, from_tab, pane);
        if self.workspaces[from_workspace].tabs[from_tab].zoomed {
            return Ok(unchanged(previous, ZOOMED_TAB));
        }
        let tab = self.reserve_tab_only(workspace)?;
        let moved = self.workspaces[from_workspace].tabs[from_tab].panes[pane].clone();
        // The new tab comes first, so a pane alone in its workspace's last
        // tab moves before that tab, and the workspace, would close.
        self.workspace_mut(workspace)?
            .tabs
            .push(Tab::new(&tab, label, pane, moved));
        let from_tab = self.tab_position(&previous.1)?.1;
        let (_, closed) = self.detach(from_workspace, from_tab, pane);
        Ok(moved_result(
            (workspace.to_owned(), tab.clone(), pane.to_owned()),
            previous,
            closed,
            Some(tab),
        ))
    }

    /// Moves `pane`, unfocused, beside `target_pane` in `target_tab`, to its
    /// right or `down` at half. A pane already in that tab, or a move out of
    /// or into a zoomed tab, is not moved: the answer says so.
    pub(super) fn move_beside(
        &mut self,
        pane: &str,
        target_tab: &str,
        target_pane: &str,
        down: bool,
    ) -> Result<PaneJoinResult, EngineError> {
        let (from_workspace, from_tab) = self.position(pane)?;
        let previous = self.place_of(from_workspace, from_tab, pane);
        let (to_workspace, to_tab) = self.tab_position(target_tab)?;
        if (to_workspace, to_tab) == (from_workspace, from_tab) {
            return Ok(unchanged(previous, SAME_TAB));
        }
        if self.workspaces[from_workspace].tabs[from_tab].zoomed
            || self.workspaces[to_workspace].tabs[to_tab].zoomed
        {
            return Ok(unchanged(previous, ZOOMED_TAB));
        }
        if !self.workspaces[to_workspace].tabs[to_tab]
            .panes
            .contains_key(target_pane)
        {
            return Err(refused(
                "target_pane_not_found",
                format!("pane {target_pane} is not in tab {target_tab}"),
            ));
        }
        let moved = self.workspaces[from_workspace].tabs[from_tab].panes[pane].clone();
        let direction = if down {
            PaneLayoutDirection::Down
        } else {
            PaneLayoutDirection::Right
        };
        let target = &mut self.workspaces[to_workspace].tabs[to_tab];
        split_leaf_at(&mut target.root, target_pane, direction, pane, 0.5);
        target.panes.insert(pane.to_owned(), moved);
        let to = (
            self.workspaces[to_workspace].id.clone(),
            target_tab.to_owned(),
            pane.to_owned(),
        );
        let from_tab = self.tab_position(&previous.1)?.1;
        let (_, closed) = self.detach(from_workspace, from_tab, pane);
        Ok(moved_result(to, previous, closed, None))
    }

    fn place_of(&self, workspace: usize, tab: usize, pane: &str) -> (String, String, String) {
        let workspace = &self.workspaces[workspace];
        (
            workspace.id.clone(),
            workspace.tabs[tab].id.clone(),
            pane.to_owned(),
        )
    }

    /// The tab's whole layout, each pane with its folder, name, command and
    /// environment, to give back with [`Topology::plan_layout`].
    pub(super) fn export(&self, tab: &str) -> Result<ClosedLayout, EngineError> {
        let (workspace, index) = self.tab_position(tab)?;
        let workspace = &self.workspaces[workspace];
        let tab = &workspace.tabs[index];
        Ok(ClosedLayout {
            workspace_id: workspace.id.clone(),
            tab_id: tab.id.clone(),
            zoomed: tab.zoomed,
            focused_pane_id: tab.focused.clone(),
            root: closed_node(&tab.root, &tab.panes)?,
        })
    }

    /// Reserves what applying `root` needs: a new tab of `workspace` (named
    /// `label`, else the name of the tab it replaces), a new pane for every
    /// leaf the replaced tab does not run, starting in the leaf's folder or
    /// the replaced tab's focused pane's, and the leaves it does run kept as
    /// they are, so applying a layout over a tab never ends a pane it keeps.
    pub(super) fn plan_layout(
        &mut self,
        workspace: &str,
        replace: Option<&str>,
        label: Option<&str>,
        root: &ClosedLayoutNode,
    ) -> Result<Planned, EngineError> {
        let fallback_cwd = {
            let found = self.workspace(workspace)?;
            let tab = match replace {
                Some(replace) => found
                    .tabs
                    .iter()
                    .find(|tab| tab.id == replace)
                    .ok_or_else(|| no_tab(replace))?,
                None => found
                    .tabs
                    .iter()
                    .find(|tab| tab.id == found.active_tab)
                    .ok_or_else(|| no_tab(&found.active_tab))?,
            };
            tab.panes
                .get(&tab.focused)
                .map(|pane| pane.cwd.clone())
                .ok_or_else(|| no_pane(&tab.focused))?
        };
        let (runs, replaced_label) = match replace {
            Some(replace) => {
                let (index, tab) = self.tab_position(replace)?;
                let tab = &self.workspaces[index].tabs[tab];
                (
                    tab.panes.keys().cloned().collect::<BTreeSet<_>>(),
                    tab.label.clone(),
                )
            }
            None => (BTreeSet::new(), String::new()),
        };
        let tab = self.reserve_tab_only(workspace)?;
        let mut new = Vec::new();
        let mut kept = BTreeSet::new();
        let root =
            self.reserve_leaves(workspace, root, &runs, &fallback_cwd, &mut new, &mut kept)?;
        Ok(Planned {
            workspace: workspace.to_owned(),
            tab,
            label: label.map_or(replaced_label, str::to_owned),
            root,
            new,
            kept,
        })
    }

    fn reserve_leaves(
        &mut self,
        workspace: &str,
        node: &ClosedLayoutNode,
        runs: &BTreeSet<String>,
        fallback_cwd: &str,
        new: &mut Vec<(String, Pane)>,
        kept: &mut BTreeSet<String>,
    ) -> Result<PaneLayoutNodeSnapshot, EngineError> {
        Ok(match node {
            ClosedLayoutNode::Pane {
                pane_id: Some(pane_id),
                ..
            } if runs.contains(pane_id) && kept.insert(pane_id.clone()) => {
                PaneLayoutNodeSnapshot::Pane {
                    pane_id: pane_id.clone(),
                }
            }
            ClosedLayoutNode::Pane {
                label,
                cwd,
                command,
                env,
                ..
            } => {
                let pane_id = self.reserve_pane(workspace)?;
                let cwd = cwd
                    .as_deref()
                    .filter(|cwd| !cwd.trim().is_empty())
                    .unwrap_or(fallback_cwd);
                new.push((
                    pane_id.clone(),
                    Pane::started(cwd, label.as_deref(), command.as_deref(), env),
                ));
                PaneLayoutNodeSnapshot::Pane { pane_id }
            }
            ClosedLayoutNode::Split {
                direction,
                ratio,
                first,
                second,
            } => PaneLayoutNodeSnapshot::Split {
                direction: match direction {
                    ClosedSplitDirection::Right => PaneLayoutDirection::Right,
                    ClosedSplitDirection::Down => PaneLayoutDirection::Down,
                },
                ratio: super::layout::valid_ratio(*ratio),
                first: Box::new(self.reserve_leaves(
                    workspace,
                    first,
                    runs,
                    fallback_cwd,
                    new,
                    kept,
                )?),
                second: Box::new(self.reserve_leaves(
                    workspace,
                    second,
                    runs,
                    fallback_cwd,
                    new,
                    kept,
                )?),
            },
        })
    }

    /// Adds the tab `planned` laid out, its new panes running, focused and
    /// last in its workspace, with the panes it keeps moved in from the tab
    /// it replaces, which then closes: the panes of that tab it did not keep,
    /// for the engine to end, and the layout made.
    pub(super) fn add_planned(
        &mut self,
        planned: Planned,
        replace: Option<&str>,
    ) -> Result<(Vec<String>, ClosedLayout), EngineError> {
        let mut panes: BTreeMap<String, Pane> = planned.new.into_iter().collect();
        let mut dropped = Vec::new();
        if let Some(replace) = replace {
            let (workspace, index) = self.tab_position(replace)?;
            let replaced = &mut self.workspaces[workspace].tabs[index];
            for (id, pane) in std::mem::take(&mut replaced.panes) {
                if planned.kept.contains(&id) {
                    panes.insert(id, pane);
                } else {
                    dropped.push(id);
                }
            }
        }
        let focused = first_leaf(&planned.root).to_owned();
        let found = self.workspace_mut(&planned.workspace)?;
        found.tabs.push(Tab {
            id: planned.tab.clone(),
            label: planned.label,
            root: planned.root,
            focused,
            zoomed: false,
            panes,
        });
        found.active_tab = planned.tab.clone();
        self.focused_workspace = Some(planned.workspace.clone());
        if let Some(replace) = replace {
            let (workspace, index) = self.tab_position(replace)?;
            self.workspaces[workspace].tabs.remove(index);
        }
        Ok((dropped, self.export(&planned.tab)?))
    }

    /// Binds `workspace` to the checkout `bound` names.
    pub(super) fn bind(&mut self, workspace: &str, bound: Bound) -> Result<(), EngineError> {
        self.workspace_mut(workspace)?.worktree = Some(bound);
        Ok(())
    }

    /// The checkout `workspace` is bound to, if any.
    pub(super) fn bound(&self, workspace: &str) -> Result<Option<Bound>, EngineError> {
        Ok(self.workspace(workspace)?.worktree.clone())
    }

    /// The workspace bound to the checkout at `path`, if one is open.
    pub(super) fn bound_to(&self, path: &str) -> Option<BoundWorkspace> {
        let workspace = self.workspaces.iter().find(|workspace| {
            workspace
                .worktree
                .as_ref()
                .is_some_and(|bound| bound.checkout_path == path)
        })?;
        let tab = workspace
            .tabs
            .iter()
            .find(|tab| tab.id == workspace.active_tab)?;
        Some(BoundWorkspace {
            workspace: workspace.id.clone(),
            label: workspace.label.clone(),
            tab: tab.id.clone(),
            tab_label: tab.label.clone(),
            pane: tab.focused.clone(),
        })
    }

    /// Moves `tab` to `index` among its workspace's tabs, the last place
    /// when past it: the tabs in their new order.
    pub(super) fn move_tab(&mut self, tab: &str, index: usize) -> Result<Vec<String>, EngineError> {
        let (workspace, from) = self.tab_position(tab)?;
        let tabs = &mut self.workspaces[workspace].tabs;
        let moved = tabs.remove(from);
        tabs.insert(index.min(tabs.len()), moved);
        Ok(tabs.iter().map(|tab| tab.id.clone()).collect())
    }

    /// The layout of the tab that holds `pane`.
    pub(super) fn pane_layout(&self, pane: &str) -> Result<SessionLayoutPayload, EngineError> {
        let (workspace, tab) = self.position(pane)?;
        let workspace = &self.workspaces[workspace];
        workspace.tabs[tab].layout(&workspace.id)
    }

    /// The whole session, as a snapshot of Herdr's would project it, its
    /// agents named by `catalog`'s definitions.
    pub(super) fn state(&self, catalog: &Catalog) -> Result<ProjectionState, EngineError> {
        let now = Instant::now();
        let mut state = ProjectionState {
            tab_focus: self.shown.last.clone(),
            tab_moves: Some(self.shown.moves.clone()),
            focused_pane_id: None,
            focused_workspace_id: self.focused_workspace.clone(),
            workspaces: Vec::new(),
            tabs: Vec::new(),
            panes: Vec::new(),
            layouts: Vec::new(),
            agents: Vec::new(),
            herdr_version: None,
        };
        for workspace in &self.workspaces {
            state.workspaces.push(ProjectedWorkspace {
                workspace_id: workspace.id.clone(),
                label: workspace.label.clone(),
                active_tab_id: workspace.active_tab.clone(),
                tokens: workspace.tokens.clone(),
                worktree: workspace.worktree.as_ref().map(|bound| ProjectedWorktree {
                    repo_key: bound.repo_key.clone(),
                    repo_name: bound.repo_name.clone(),
                    repo_root: bound.repo_root.clone(),
                    checkout_path: bound.checkout_path.clone(),
                    is_linked_worktree: bound.linked,
                }),
            });
            for (number, tab) in workspace.tabs.iter().enumerate() {
                state.tabs.push(ProjectedTab {
                    number: u32::try_from(number + 1).unwrap_or(u32::MAX),
                    tab_id: tab.id.clone(),
                    workspace_id: workspace.id.clone(),
                    label: tab.label.clone(),
                });
                if self.focused_workspace.as_deref() == Some(workspace.id.as_str())
                    && workspace.active_tab == tab.id
                {
                    state.focused_pane_id = Some(tab.focused.clone());
                }
                for pane_id in leaves(&tab.root) {
                    let pane = tab.panes.get(&pane_id).ok_or_else(|| {
                        EngineError::Answer(format!(
                            "tab {} lays out pane {pane_id}, which it does not hold",
                            tab.id
                        ))
                    })?;
                    let listed = pane.agent.as_ref().filter(|agent| agent.listed(now));
                    let title = pane.live.facts.title.clone();
                    state.panes.push(ProjectedPane {
                        terminal_id: pane_id.clone(),
                        restore_error: pane.failed.clone(),
                        foreground_process: pane
                            .live
                            .facts
                            .foreground
                            .as_ref()
                            .map(|program| program.name.clone()),
                        agent_status: listed
                            .map_or(Status::Unknown, |agent| agent.live.status)
                            .as_str()
                            .to_owned(),
                        workspace_id: workspace.id.clone(),
                        tab_id: tab.id.clone(),
                        cwd: Some(pane.cwd.clone()),
                        tokens: pane.tokens.clone(),
                        label: pane.label.clone(),
                        terminal_title_stripped: title
                            .as_deref()
                            .and_then(super::shell_line::stripped_title),
                        terminal_title: title,
                        pane_id: pane_id.clone(),
                    });
                    if let Some(agent) = listed {
                        state.agents.push(projected_agent(
                            &workspace.id,
                            &tab.id,
                            &pane_id,
                            pane,
                            agent,
                            catalog,
                            now,
                        ));
                    }
                }
                state.layouts.push(tab.layout(&workspace.id)?);
            }
        }
        Ok(state)
    }

    /// The session as its saved form writes it (`store`).
    pub(super) fn to_saved(&self) -> Saved {
        Saved {
            format: FORMAT,
            workspace_numbers: self.workspace_numbers,
            focused_workspace: self.focused_workspace.clone(),
            workspaces: self
                .workspaces
                .iter()
                .map(|workspace| SavedWorkspace {
                    id: workspace.id.clone(),
                    label: workspace.label.clone(),
                    tokens: workspace.tokens.clone(),
                    worktree: workspace.worktree.as_ref().map(|bound| SavedWorktree {
                        repo_key: bound.repo_key.clone(),
                        repo_name: bound.repo_name.clone(),
                        repo_root: bound.repo_root.clone(),
                        checkout_path: bound.checkout_path.clone(),
                        linked: bound.linked,
                    }),
                    active_tab: workspace.active_tab.clone(),
                    tab_numbers: workspace.tab_numbers,
                    pane_numbers: workspace.pane_numbers,
                    tabs: workspace
                        .tabs
                        .iter()
                        .map(|tab| SavedTab {
                            id: tab.id.clone(),
                            label: tab.label.clone(),
                            focused: tab.focused.clone(),
                            zoomed: tab.zoomed,
                            root: saved_node(&tab.root),
                            panes: tab
                                .panes
                                .iter()
                                .map(|(id, pane)| SavedPane {
                                    id: id.clone(),
                                    cwd: pane.cwd.clone(),
                                    label: pane.label.clone(),
                                    tokens: pane.tokens.clone(),
                                    agent: pane.agent.as_ref().map(|agent| SavedAgent {
                                        kind: agent.kind.clone(),
                                        name: agent.name.clone(),
                                        session: agent.session.clone(),
                                    }),
                                })
                                .collect(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    /// The session a saved form holds, refused when it does not hold
    /// together: every tab lays out exactly the panes it holds, no id is
    /// held twice, every focus and active tab names what is there. Every
    /// counter is taken past every id the form holds, whatever it says.
    pub(super) fn from_saved(saved: Saved) -> Result<Self, String> {
        if saved.format != FORMAT {
            return Err(format!(
                "the saved session is format {}, this build reads format {FORMAT}",
                saved.format
            ));
        }
        let mut seen = BTreeSet::new();
        let mut topology = Topology {
            workspaces: Vec::new(),
            focused_workspace: saved.focused_workspace,
            workspace_numbers: saved.workspace_numbers,
            shown: Shown::default(),
            agent_seq: 0,
        };
        for workspace in saved.workspaces {
            if ids::workspace_number(&workspace.id).is_none() || !seen.insert(workspace.id.clone())
            {
                return Err(format!(
                    "workspace id {:?} is malformed or repeated",
                    workspace.id
                ));
            }
            let mut tabs = Vec::new();
            for tab in workspace.tabs {
                if !seen.insert(tab.id.clone()) {
                    return Err(format!("tab id {:?} is repeated", tab.id));
                }
                let root = tree_of(&tab.root)?;
                let laid: BTreeSet<String> = leaves(&root).into_iter().collect();
                let mut panes = BTreeMap::new();
                for pane in tab.panes {
                    if !seen.insert(pane.id.clone()) {
                        return Err(format!("pane id {:?} is repeated", pane.id));
                    }
                    panes.insert(
                        pane.id,
                        Pane {
                            cwd: pane.cwd,
                            label: pane.label,
                            tokens: pane.tokens,
                            agent: pane.agent.map(|agent| PaneAgent {
                                kind: agent.kind,
                                name: agent.name,
                                session: agent.session,
                                ..PaneAgent::default()
                            }),
                            ..Pane::default()
                        },
                    );
                }
                if laid.len() != leaves(&root).len()
                    || laid != panes.keys().cloned().collect::<BTreeSet<_>>()
                {
                    return Err(format!(
                        "tab {} does not lay out exactly the panes it holds",
                        tab.id
                    ));
                }
                if !laid.contains(&tab.focused) {
                    return Err(format!("tab {} focuses a pane it does not hold", tab.id));
                }
                tabs.push(Tab {
                    id: tab.id,
                    label: tab.label,
                    root,
                    focused: tab.focused,
                    zoomed: tab.zoomed,
                    panes,
                });
            }
            if !tabs.iter().any(|tab| tab.id == workspace.active_tab) {
                return Err(format!(
                    "workspace {} shows a tab it does not hold",
                    workspace.id
                ));
            }
            topology.workspaces.push(Workspace {
                id: workspace.id,
                label: workspace.label,
                tokens: workspace.tokens,
                worktree: workspace.worktree.map(|bound| Bound {
                    repo_key: bound.repo_key,
                    repo_name: bound.repo_name,
                    repo_root: bound.repo_root,
                    checkout_path: bound.checkout_path,
                    linked: bound.linked,
                }),
                tabs,
                active_tab: workspace.active_tab,
                tab_numbers: workspace.tab_numbers,
                pane_numbers: workspace.pane_numbers,
            });
        }
        if let Some(focused) = &topology.focused_workspace
            && topology.workspace(focused).is_err()
        {
            topology.focused_workspace = topology.workspaces.first().map(|w| w.id.clone());
        }
        topology.count_past(seen.iter().map(String::as_str));
        Ok(topology)
    }

    /// Takes every counter past the ids given, so none is given again: a
    /// workspace id moves the workspace count, a tab or pane id its
    /// workspace's count when that workspace is held. An id of another shape
    /// moves nothing.
    pub(super) fn count_past<'a>(&mut self, given: impl IntoIterator<Item = &'a str>) {
        for id in given {
            if let Some(number) = ids::workspace_number(id) {
                self.workspace_numbers = self.workspace_numbers.max(number);
                continue;
            }
            let (kind, (workspace, number)) = match ids::numbered(id, 't') {
                Some(found) => ('t', found),
                None => match ids::numbered(id, 'p') {
                    Some(found) => ('p', found),
                    None => continue,
                },
            };
            if let Some(number_of) = ids::workspace_number(workspace) {
                self.workspace_numbers = self.workspace_numbers.max(number_of);
            }
            if let Ok(found) = self.workspace_mut(workspace) {
                match kind {
                    't' => found.tab_numbers = found.tab_numbers.max(number),
                    _ => found.pane_numbers = found.pane_numbers.max(number),
                }
            }
        }
    }

    /// Gathers `panes`, which run with no place in the session, into one new
    /// tab of each workspace their ids name, so none is lost: split side by
    /// side in turn, at most a tab's limit of panes to a tab. A workspace no
    /// longer held is made again under its own id. The tabs made.
    pub(super) fn gather(&mut self, panes: Vec<(String, Pane)>, label: &str) -> Vec<String> {
        let mut by_workspace: BTreeMap<String, Vec<(String, Pane)>> = BTreeMap::new();
        for (id, pane) in panes {
            let workspace = ids::numbered(&id, 'p')
                .map(|(workspace, _)| workspace.to_owned())
                .unwrap_or_else(|| {
                    // Not an id of this engine's: the first workspace takes
                    // it, or a new one.
                    self.workspaces
                        .first()
                        .map(|workspace| workspace.id.clone())
                        .unwrap_or_else(|| ids::workspace_id(self.workspace_numbers + 1))
                });
            by_workspace.entry(workspace).or_default().push((id, pane));
        }
        let mut made = Vec::new();
        for (workspace, panes) in by_workspace {
            if self.workspace(&workspace).is_err() {
                self.workspaces.push(Workspace {
                    id: workspace.clone(),
                    label: label.to_owned(),
                    tokens: BTreeMap::new(),
                    worktree: None,
                    tabs: Vec::new(),
                    active_tab: String::new(),
                    tab_numbers: 0,
                    pane_numbers: 0,
                });
                if self.focused_workspace.is_none() {
                    self.focused_workspace = Some(workspace.clone());
                }
            }
            self.count_past(
                std::iter::once(workspace.as_str()).chain(panes.iter().map(|(id, _)| id.as_str())),
            );
            for chunk in panes.chunks(super::layout::MAX_PANES) {
                let Ok(tab) = self.reserve_tab_only(&workspace) else {
                    continue;
                };
                let mut iter = chunk.iter();
                let Some((first_id, first)) = iter.next() else {
                    continue;
                };
                let mut gathered = Tab::new(&tab, label, first_id, first.clone());
                let mut target = first_id.clone();
                for (turn, (id, pane)) in iter.enumerate() {
                    let direction = if turn % 2 == 0 {
                        PaneLayoutDirection::Right
                    } else {
                        PaneLayoutDirection::Down
                    };
                    split_leaf_at(&mut gathered.root, &target, direction, id, 0.5);
                    gathered.panes.insert(id.clone(), pane.clone());
                    target = id.clone();
                }
                let Ok(found) = self.workspace_mut(&workspace) else {
                    continue;
                };
                if found.tabs.is_empty() {
                    found.active_tab = tab.clone();
                }
                found.tabs.push(gathered);
                made.push(tab);
            }
        }
        made
    }

    fn workspace(&self, workspace: &str) -> Result<&Workspace, EngineError> {
        self.workspaces
            .iter()
            .find(|found| found.id == workspace)
            .ok_or_else(|| no_workspace(workspace))
    }

    fn workspace_mut(&mut self, workspace: &str) -> Result<&mut Workspace, EngineError> {
        self.workspaces
            .iter_mut()
            .find(|found| found.id == workspace)
            .ok_or_else(|| no_workspace(workspace))
    }

    fn position(&self, pane: &str) -> Result<(usize, usize), EngineError> {
        self.workspaces
            .iter()
            .enumerate()
            .find_map(|(workspace_index, workspace)| {
                workspace
                    .tabs
                    .iter()
                    .position(|tab| tab.panes.contains_key(pane))
                    .map(|tab_index| (workspace_index, tab_index))
            })
            .ok_or_else(|| no_pane(pane))
    }

    fn tab_position(&self, tab: &str) -> Result<(usize, usize), EngineError> {
        self.workspaces
            .iter()
            .enumerate()
            .find_map(|(workspace_index, workspace)| {
                workspace
                    .tabs
                    .iter()
                    .position(|found| found.id == tab)
                    .map(|tab_index| (workspace_index, tab_index))
            })
            .ok_or_else(|| no_tab(tab))
    }
}

/// A move that changed nothing, with Herdr's reason.
fn unchanged((workspace, tab, pane): (String, String, String), reason: &str) -> PaneJoinResult {
    PaneJoinResult {
        changed: false,
        reason: Some(reason.to_owned()),
        workspace_id: workspace.clone(),
        tab_id: tab.clone(),
        terminal_id: pane.clone(),
        pane_id: pane.clone(),
        previous_workspace_id: workspace,
        previous_tab_id: tab,
        previous_pane_id: pane,
        closed_tab_id: None,
        closed_workspace_id: None,
        created_tab_id: None,
    }
}

fn moved_result(
    (workspace, tab, pane): (String, String, String),
    (previous_workspace, previous_tab, previous_pane): (String, String, String),
    closed: Closed,
    created_tab: Option<String>,
) -> PaneJoinResult {
    PaneJoinResult {
        changed: true,
        reason: None,
        workspace_id: workspace,
        tab_id: tab,
        terminal_id: pane.clone(),
        pane_id: pane,
        previous_workspace_id: previous_workspace,
        previous_tab_id: previous_tab,
        previous_pane_id: previous_pane,
        closed_tab_id: closed.tab,
        closed_workspace_id: closed.workspace,
        created_tab_id: created_tab,
    }
}

fn leaves(root: &PaneLayoutNodeSnapshot) -> Vec<String> {
    let mut ids = Vec::new();
    root.collect_pane_ids(&mut ids);
    ids.into_iter().map(str::to_owned).collect()
}

fn closed_node(
    node: &PaneLayoutNodeSnapshot,
    panes: &BTreeMap<String, Pane>,
) -> Result<ClosedLayoutNode, EngineError> {
    Ok(match node {
        PaneLayoutNodeSnapshot::Pane { pane_id } => {
            let pane = panes.get(pane_id).ok_or_else(|| no_pane(pane_id))?;
            ClosedLayoutNode::Pane {
                pane_id: Some(pane_id.clone()),
                label: pane.label.clone(),
                cwd: Some(pane.cwd.clone()),
                command: pane.command.clone(),
                env: pane.env.clone(),
            }
        }
        PaneLayoutNodeSnapshot::Split {
            direction,
            ratio,
            first,
            second,
        } => ClosedLayoutNode::Split {
            direction: match direction {
                PaneLayoutDirection::Right => ClosedSplitDirection::Right,
                PaneLayoutDirection::Down => ClosedSplitDirection::Down,
            },
            ratio: *ratio,
            first: Box::new(closed_node(first, panes)?),
            second: Box::new(closed_node(second, panes)?),
        },
    })
}

fn saved_node(node: &PaneLayoutNodeSnapshot) -> SavedNode {
    match node {
        PaneLayoutNodeSnapshot::Pane { pane_id } => SavedNode::Pane {
            id: pane_id.clone(),
        },
        PaneLayoutNodeSnapshot::Split {
            direction,
            ratio,
            first,
            second,
        } => SavedNode::Split {
            direction: match direction {
                PaneLayoutDirection::Right => SavedDirection::Right,
                PaneLayoutDirection::Down => SavedDirection::Down,
            },
            ratio: *ratio,
            first: Box::new(saved_node(first)),
            second: Box::new(saved_node(second)),
        },
    }
}

fn tree_of(node: &SavedNode) -> Result<PaneLayoutNodeSnapshot, String> {
    Ok(match node {
        SavedNode::Pane { id } => PaneLayoutNodeSnapshot::Pane {
            pane_id: id.clone(),
        },
        SavedNode::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            if !(super::layout::RATIO_MIN..=super::layout::RATIO_MAX).contains(ratio) {
                return Err(format!("a split's ratio {ratio} is out of its limits"));
            }
            PaneLayoutNodeSnapshot::Split {
                direction: match direction {
                    SavedDirection::Right => PaneLayoutDirection::Right,
                    SavedDirection::Down => PaneLayoutDirection::Down,
                },
                ratio: *ratio,
                first: Box::new(tree_of(first)?),
                second: Box::new(tree_of(second)?),
            }
        }
    })
}

impl Tab {
    fn new(id: &str, label: &str, pane_id: &str, pane: Pane) -> Self {
        Self {
            id: id.to_owned(),
            label: label.to_owned(),
            root: PaneLayoutNodeSnapshot::Pane {
                pane_id: pane_id.to_owned(),
            },
            focused: pane_id.to_owned(),
            zoomed: false,
            panes: BTreeMap::from([(pane_id.to_owned(), pane)]),
        }
    }

    fn layout(&self, workspace: &str) -> Result<SessionLayoutPayload, EngineError> {
        let mut panes = Vec::new();
        let mut splits = Vec::new();
        lay_out(&self.root, NOMINAL_AREA, &mut panes, &mut splits).map_err(|why| {
            EngineError::Answer(format!("tab {} cannot be laid out: {why}", self.id))
        })?;
        let focused = if contains(&self.root, &self.focused) {
            self.focused.clone()
        } else {
            first_leaf(&self.root).to_owned()
        };
        Ok(SessionLayoutPayload {
            workspace_id: workspace.to_owned(),
            tab_id: self.id.clone(),
            zoomed: self.zoomed,
            area: NOMINAL_AREA,
            focused_pane_id: focused,
            panes,
            splits,
        })
    }
}

/// What the engine says of the agent a pane runs, as Herdr's agent list
/// says it: its parent and lineage are read from the pane's tokens the
/// spawner wrote, as for a Herdr agent.
fn projected_agent(
    workspace: &str,
    tab: &str,
    pane_id: &str,
    pane: &Pane,
    agent: &PaneAgent,
    catalog: &Catalog,
    now: Instant,
) -> ProjectedAgent {
    use crate::engine::tokens::{
        CHILD_SESSION_TOKEN, PARENT_MACHINE_TOKEN, PARENT_PANE_TOKEN, PARENT_SESSION_TOKEN,
        lineage_parent, session_digest, session_holds,
    };
    let token = |name: &str| pane.tokens.get(name).and_then(Value::as_str);
    let own_session = agent.session.as_deref().and_then(session_digest);
    let child_holds = session_holds(
        lineage_parent(token(CHILD_SESSION_TOKEN)).as_deref(),
        own_session.as_deref(),
    );
    let declared = |name: &str| child_holds.then(|| lineage_parent(token(name))).flatten();
    ProjectedAgent {
        pane_id: pane_id.to_owned(),
        terminal_id: pane_id.to_owned(),
        name: agent.name.clone(),
        workspace_id: workspace.to_owned(),
        tab_id: tab.to_owned(),
        cwd: Some(pane.cwd.clone()),
        agent: Some(
            agents::definition(catalog, &agent.kind)
                .map_or_else(|| agent.kind.clone(), agents::kind_name),
        ),
        agent_status: Some(agent.live.status.as_str().to_owned()),
        agent_session: agent
            .session
            .as_ref()
            .map(|session| SessionAgentSessionPayload {
                kind: match agents::session_kind(session) {
                    PaneSessionKind::Id => "id",
                    PaneSessionKind::Path => "path",
                }
                .to_owned(),
                value: session.clone(),
            }),
        spawned_from_pane_id: declared(PARENT_PANE_TOKEN),
        spawned_from_machine_id: declared(PARENT_MACHINE_TOKEN),
        declared_parent_session: declared(PARENT_SESSION_TOKEN),
        lineage_session: own_session,
        state_change_seq: agent.live.seq,
        completion_seq: agent.live.completion,
        launch_pending: agent.starting(now),
        tokens: pane.tokens.clone(),
    }
}

/// The pane an agent target names: an agent's name, or a pane id.
fn target_matches(pane_id: &str, pane: &Pane, target: &str) -> bool {
    pane_id == target
        || pane
            .agent
            .as_ref()
            .and_then(|agent| agent.name.as_deref())
            .is_some_and(|name| name == target)
}

impl Topology {
    /// Lays a pane's holder's `facts` on it ([`agents::apply`]); a pane the
    /// engine does not hold is refused.
    pub(super) fn apply_facts(
        &mut self,
        pane: &str,
        facts: &PaneFacts,
        catalog: &Catalog,
    ) -> Result<agents::Applied, EngineError> {
        let mut seq = self.agent_seq;
        let held = self.pane_mut(pane)?;
        let applied = agents::apply(
            catalog,
            &mut held.cwd,
            &mut held.agent,
            &mut held.live,
            facts,
            &mut seq,
        );
        self.agent_seq = seq;
        Ok(applied)
    }

    /// The pane an agent target names, and its agent while it runs or
    /// starts: refused `agent_not_found` otherwise, as Herdr refuses it.
    pub(super) fn agent(&self, target: &str) -> Result<(String, &Pane, &PaneAgent), EngineError> {
        let now = Instant::now();
        self.panes()
            .filter(|(id, pane)| target_matches(id, pane, target))
            .find_map(|(id, pane)| {
                pane.agent
                    .as_ref()
                    .filter(|agent| agent.listed(now))
                    .map(|agent| (id.to_owned(), pane, agent))
            })
            .ok_or_else(|| {
                refused(
                    "agent_not_found",
                    format!("no agent named {target} runs on the core's own engine"),
                )
            })
    }

    /// Whether another pane than `pane` runs or starts an agent named
    /// `name`.
    pub(super) fn name_taken(&self, name: &str, pane: &str) -> bool {
        let now = Instant::now();
        self.panes().any(|(id, held)| {
            id != pane
                && held
                    .agent
                    .as_ref()
                    .is_some_and(|agent| agent.listed(now) && agent.name.as_deref() == Some(name))
        })
    }

    /// Records that the engine typed `agent`'s start into `pane`.
    pub(super) fn start_agent(&mut self, pane: &str, agent: PaneAgent) -> Result<(), EngineError> {
        self.pane_mut(pane)?.agent = Some(agent);
        Ok(())
    }

    /// Records the session the `kind` agent in `pane` runs: on the agent the
    /// pane runs, or on a new record of it when the pane's agent has not
    /// shown yet (a start's report can come first). Whether anything moved.
    pub(super) fn set_agent_session(
        &mut self,
        pane: &str,
        kind: &str,
        session: &str,
    ) -> Result<bool, EngineError> {
        let held = self.pane_mut(pane)?;
        match held.agent.as_mut() {
            Some(agent) if agent.kind == kind => {
                let changed = agent.session.as_deref() != Some(session);
                agent.session = Some(session.to_owned());
                Ok(changed)
            }
            _ => {
                held.agent = Some(PaneAgent {
                    kind: kind.to_owned(),
                    session: Some(session.to_owned()),
                    ..PaneAgent::default()
                });
                Ok(true)
            }
        }
    }

    /// The panes that run or start an agent, with its kind.
    pub(super) fn agent_kinds(&self) -> Vec<(String, String)> {
        let now = Instant::now();
        self.panes()
            .filter_map(|(id, pane)| {
                pane.agent
                    .as_ref()
                    .filter(|agent| agent.listed(now))
                    .map(|agent| (id.to_owned(), agent.kind.clone()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(topology: &mut Topology) -> NewWorkspace {
        let ids = topology.reserve_workspace();
        topology.add_workspace(&ids, "w", Pane::new("/w", None), false);
        ids
    }

    fn pane_ids(topology: &Topology) -> Vec<String> {
        topology
            .state(&Catalog::read(Vec::new()))
            .unwrap()
            .panes
            .into_iter()
            .map(|pane| pane.pane_id)
            .collect()
    }

    fn split(topology: &mut Topology, target: &str, direction: PaneLayoutDirection) -> String {
        let (workspace, _) = topology.pane_place(target).unwrap();
        let created = topology.reserve_pane(&workspace).unwrap();
        topology
            .add_split(
                target,
                direction,
                0.5,
                &created,
                Pane::new("/w", None),
                false,
            )
            .unwrap();
        created
    }

    fn new_tab(topology: &mut Topology, workspace: &str) -> (String, String) {
        let (tab, pane) = topology.reserve_tab(workspace).unwrap();
        topology
            .add_tab(workspace, (&tab, &pane), "", Pane::new("/w", None), false)
            .unwrap();
        (tab, pane)
    }

    fn tree(topology: &Topology, pane: &str) -> PaneLayoutNodeSnapshot {
        crate::live::project_layout(&topology.pane_layout(pane).unwrap())
            .unwrap()
            .root
    }

    #[test]
    fn closing_the_last_pane_closes_its_tab_and_the_last_tab_its_workspace() {
        let mut topology = Topology::default();
        let first = workspace(&mut topology);
        let (tab, pane) = topology.reserve_tab(&first.workspace).unwrap();
        topology
            .add_tab(
                &first.workspace,
                (&tab, &pane),
                "",
                Pane::new("/w", None),
                true,
            )
            .unwrap();
        assert_eq!(topology.active_tab(&first.workspace).unwrap(), tab);

        assert_eq!(
            topology.remove_pane(&pane).unwrap(),
            Closed {
                tab: Some(tab),
                workspace: None
            }
        );
        assert_eq!(topology.active_tab(&first.workspace).unwrap(), first.tab);
        assert_eq!(
            topology.remove_pane(&first.pane).unwrap(),
            Closed {
                tab: Some(first.tab),
                workspace: Some(first.workspace.clone())
            }
        );
        assert!(
            topology
                .state(&Catalog::read(Vec::new()))
                .unwrap()
                .workspaces
                .is_empty()
        );
        assert_eq!(topology.focused_workspace(), None);
    }

    #[test]
    fn a_closed_pane_hands_focus_to_its_sibling_and_its_id_is_never_given_again() {
        let mut topology = Topology::default();
        let first = workspace(&mut topology);
        let beside = topology.reserve_pane(&first.workspace).unwrap();
        topology
            .add_split(
                &first.pane,
                PaneLayoutDirection::Right,
                0.5,
                &beside,
                Pane::new("/w", None),
                true,
            )
            .unwrap();
        assert_eq!(pane_ids(&topology), [first.pane.clone(), beside.clone()]);
        assert_eq!(
            topology
                .state(&Catalog::read(Vec::new()))
                .unwrap()
                .focused_pane_id
                .as_deref(),
            Some(beside.as_str())
        );

        assert_eq!(topology.remove_pane(&beside).unwrap(), Closed::default());
        assert_eq!(
            topology
                .state(&Catalog::read(Vec::new()))
                .unwrap()
                .focused_pane_id
                .as_deref(),
            Some(first.pane.as_str())
        );
        let next = topology.reserve_pane(&first.workspace).unwrap();
        assert_eq!(next, "w1:p3");
        assert!(matches!(
            topology.pane(&beside),
            Err(EngineError::Refused { code, .. }) if code == "pane_not_found"
        ));
    }

    #[test]
    fn a_split_lays_out_as_the_projection_reads_it_back() {
        let mut topology = Topology::default();
        let first = workspace(&mut topology);
        let below = split(&mut topology, &first.pane, PaneLayoutDirection::Down);
        let layout = topology.pane_layout(&below).unwrap();
        let read = crate::live::project_layout(&layout).unwrap();
        assert_eq!(
            read.root,
            PaneLayoutNodeSnapshot::Split {
                direction: PaneLayoutDirection::Down,
                ratio: 0.5,
                first: Box::new(PaneLayoutNodeSnapshot::Pane {
                    pane_id: first.pane.clone()
                }),
                second: Box::new(PaneLayoutNodeSnapshot::Pane { pane_id: below }),
            }
        );
        assert_eq!(read.focused_pane_id, first.pane);
    }

    #[test]
    fn a_zoom_takes_the_focus_ends_on_a_split_and_never_zooms_a_lone_pane() {
        let mut topology = Topology::default();
        let first = workspace(&mut topology);
        assert!(!topology.zoom(&first.pane, None).unwrap());
        assert!(!topology.pane_layout(&first.pane).unwrap().zoomed);

        let beside = split(&mut topology, &first.pane, PaneLayoutDirection::Right);
        assert!(topology.zoom(&beside, None).unwrap());
        let zoomed = topology.pane_layout(&beside).unwrap();
        assert!(zoomed.zoomed);
        assert_eq!(zoomed.focused_pane_id, beside);
        assert!(!topology.zoom(&beside, Some(true)).unwrap());

        split(&mut topology, &first.pane, PaneLayoutDirection::Down);
        assert!(!topology.pane_layout(&beside).unwrap().zoomed);
    }

    #[test]
    fn a_resize_moves_the_split_the_drawing_ahead_predicts_and_stops_at_its_limit() {
        let mut topology = Topology::default();
        let first = workspace(&mut topology);
        let beside = split(&mut topology, &first.pane, PaneLayoutDirection::Right);
        assert!(
            topology
                .resize(&beside, PaneResizeDirection::Right, 0.3)
                .unwrap()
        );
        let PaneLayoutNodeSnapshot::Split { ratio, .. } = tree(&topology, &beside) else {
            panic!("the tab stays split");
        };
        assert!((ratio - 0.8).abs() < 1e-3, "{ratio}");
        assert!(
            topology
                .resize(&beside, PaneResizeDirection::Right, 0.5)
                .unwrap()
        );
        assert!(
            !topology
                .resize(&beside, PaneResizeDirection::Right, 0.5)
                .unwrap(),
            "a split at its limit moves no further"
        );
        assert!(
            !topology
                .resize(&beside, PaneResizeDirection::Up, 0.1)
                .unwrap(),
            "no split of that axis holds the pane"
        );
    }

    #[test]
    fn a_swap_trades_places_within_a_tab_and_refuses_panes_of_two_tabs() {
        let mut topology = Topology::default();
        let first = workspace(&mut topology);
        let beside = split(&mut topology, &first.pane, PaneLayoutDirection::Right);
        topology.swap(&first.pane, &beside).unwrap();
        assert_eq!(pane_ids(&topology), [beside.clone(), first.pane.clone()]);

        let (_, other) = new_tab(&mut topology, &first.workspace);
        assert!(matches!(
            topology.swap(&first.pane, &other),
            Err(EngineError::Refused { code, .. }) if code == "panes_in_different_tabs"
        ));
    }

    #[test]
    fn a_pane_moves_to_a_new_tab_and_beside_another_keeping_its_id() {
        let mut topology = Topology::default();
        let first = workspace(&mut topology);
        let beside = split(&mut topology, &first.pane, PaneLayoutDirection::Right);

        let moved = topology
            .move_to_new_tab(&beside, &first.workspace, "moved")
            .unwrap();
        assert!(moved.changed);
        assert_eq!(moved.pane_id, beside);
        assert_eq!(moved.previous_tab_id, first.tab);
        let created = moved.created_tab_id.clone().unwrap();
        assert_eq!(created, "w1:t2");
        assert_eq!(
            topology.tab_panes(&created).unwrap(),
            std::slice::from_ref(&beside)
        );
        // Unfocused: the workspace still shows its first tab.
        assert_eq!(topology.active_tab(&first.workspace).unwrap(), first.tab);

        let back = topology
            .move_beside(&beside, &first.tab, &first.pane, true)
            .unwrap();
        assert!(back.changed);
        assert_eq!(back.closed_tab_id.as_deref(), Some(created.as_str()));
        assert_eq!(
            tree(&topology, &beside),
            PaneLayoutNodeSnapshot::Split {
                direction: PaneLayoutDirection::Down,
                ratio: 0.5,
                first: Box::new(PaneLayoutNodeSnapshot::Pane {
                    pane_id: first.pane.clone()
                }),
                second: Box::new(PaneLayoutNodeSnapshot::Pane {
                    pane_id: beside.clone()
                }),
            }
        );

        let same = topology
            .move_beside(&beside, &first.tab, &first.pane, false)
            .unwrap();
        assert_eq!(
            (same.changed, same.reason.as_deref()),
            (false, Some("same_tab"))
        );
        topology.zoom(&first.pane, Some(true)).unwrap();
        let zoomed = topology
            .move_to_new_tab(&beside, &first.workspace, "x")
            .unwrap();
        assert_eq!(
            (zoomed.changed, zoomed.reason.as_deref()),
            (false, Some("zoomed_tab"))
        );
    }

    #[test]
    fn the_last_pane_of_a_workspace_moves_to_a_new_tab_without_closing_it() {
        let mut topology = Topology::default();
        let first = workspace(&mut topology);
        let moved = topology
            .move_to_new_tab(&first.pane, &first.workspace, "alone")
            .unwrap();
        assert!(moved.changed);
        assert_eq!(moved.closed_tab_id.as_deref(), Some(first.tab.as_str()));
        assert_eq!(moved.closed_workspace_id, None);
        assert_eq!(pane_ids(&topology), [first.pane]);
    }

    #[test]
    fn an_exported_tab_applied_again_keeps_the_panes_it_runs_and_starts_the_rest() {
        let mut topology = Topology::default();
        let first = workspace(&mut topology);
        let beside = split(&mut topology, &first.pane, PaneLayoutDirection::Right);
        let exported = topology.export(&first.tab).unwrap();
        // The layout given back wraps the kept tree beside one new pane.
        let root = ClosedLayoutNode::Split {
            direction: ClosedSplitDirection::Down,
            ratio: 0.6,
            first: Box::new(exported.root.clone()),
            second: Box::new(ClosedLayoutNode::Pane {
                pane_id: None,
                label: Some("new".to_owned()),
                cwd: None,
                command: None,
                env: Default::default(),
            }),
        };
        let planned = topology
            .plan_layout(&first.workspace, Some(&first.tab), None, &root)
            .unwrap();
        assert_eq!(
            planned.kept,
            BTreeSet::from([first.pane.clone(), beside.clone()])
        );
        assert_eq!(planned.new.len(), 1);
        let (new_pane, started) = &planned.new[0];
        assert_eq!(new_pane, "w1:p3");
        assert_eq!(started.cwd, "/w");
        let (dropped, applied) = topology.add_planned(planned, Some(&first.tab)).unwrap();
        assert!(dropped.is_empty());
        assert_eq!(applied.tab_id, "w1:t2");
        assert_eq!(applied.root.pane_count(), 3);
        assert!(matches!(
            topology.tab_panes(&first.tab),
            Err(EngineError::Refused { code, .. }) if code == "tab_not_found"
        ));
        assert_eq!(topology.active_tab(&first.workspace).unwrap(), "w1:t2");
    }

    #[test]
    fn a_saved_session_reads_back_as_it_was_with_its_counters() {
        let mut topology = Topology::default();
        let first = workspace(&mut topology);
        let beside = split(&mut topology, &first.pane, PaneLayoutDirection::Right);
        topology.remove_pane(&beside).unwrap();
        let again = split(&mut topology, &first.pane, PaneLayoutDirection::Down);
        topology
            .set_pane_tokens(
                &again,
                &BTreeMap::from([("purpose".to_owned(), Some("kept".to_owned()))]),
            )
            .unwrap();
        let saved = serde_json::to_string(&topology.to_saved()).unwrap();
        let mut read = Topology::from_saved(serde_json::from_str(&saved).unwrap()).unwrap();
        assert_eq!(
            read.state(&Catalog::read(Vec::new())).unwrap(),
            topology.state(&Catalog::read(Vec::new())).unwrap()
        );
        assert_eq!(read.reserve_pane(&first.workspace).unwrap(), "w1:p4");
        assert_eq!(read.reserve_workspace().workspace, "w2");
    }

    #[test]
    fn a_saved_session_that_does_not_hold_together_is_refused() {
        let mut topology = Topology::default();
        let first = workspace(&mut topology);
        let mut saved = topology.to_saved();
        saved.workspaces[0].tabs[0].focused = "w1:p9".to_owned();
        assert!(Topology::from_saved(saved).is_err());
        let mut saved = topology.to_saved();
        saved.workspaces[0].tabs[0].panes.clear();
        assert!(Topology::from_saved(saved).is_err());
        let mut saved = topology.to_saved();
        saved.format = 2;
        let refused = Topology::from_saved(saved).err().unwrap();
        assert!(refused.contains("format 2"), "{refused}");
        let _ = first;
    }

    #[test]
    fn panes_with_no_place_gather_in_a_new_tab_of_their_workspace_and_move_its_counters() {
        let mut topology = Topology::default();
        let first = workspace(&mut topology);
        let made = topology.gather(
            vec![
                ("w1:p7".to_owned(), Pane::new("/a", None)),
                ("w1:p8".to_owned(), Pane::new("/b", None)),
                ("w5:p2".to_owned(), Pane::new("/c", None)),
            ],
            "restored",
        );
        assert_eq!(made, ["w1:t2", "w5:t1"]);
        assert_eq!(
            topology.tab_panes("w1:t2").unwrap(),
            ["w1:p7".to_owned(), "w1:p8".to_owned()]
        );
        assert_eq!(topology.reserve_pane(&first.workspace).unwrap(), "w1:p9");
        assert_eq!(topology.reserve_pane("w5").unwrap(), "w5:p3");
        assert_eq!(topology.reserve_workspace().workspace, "w6");
    }
}
