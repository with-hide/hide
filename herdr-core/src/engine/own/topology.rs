//! What the own engine holds of its session (PRD engine-bundle-3-core-owns
//! D-3): its workspaces in order, each workspace's tabs in order, each tab's
//! pane tree, focus and panes, the keyboard's workspace, Hide's tokens, and
//! the counters ids are given from. It is plain data behind the engine's
//! own lock, never the runtime's, and starts and ends no program: the engine
//! reserves an id here, starts the pane's holder outside the lock, and adds
//! the pane only once it runs.

use std::collections::BTreeMap;

use serde_json::Value;

use super::ids;
use super::layout::{contains, first_leaf, lay_out, remove_leaf, split_leaf};
use crate::engine::EngineError;
use crate::model::{PaneLayoutDirection, PaneLayoutNodeSnapshot};
use crate::session_sync::{ProjectedPane, ProjectedTab, ProjectedWorkspace, ProjectionState};
use crate::sidebar::{SessionLayoutPayload, SessionLayoutRect};

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

#[derive(Default)]
pub(super) struct Topology {
    workspaces: Vec<Workspace>,
    focused_workspace: Option<String>,
    /// The last workspace number given.
    workspace_numbers: usize,
}

struct Workspace {
    id: String,
    label: String,
    tokens: BTreeMap<String, Value>,
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
#[derive(Clone, Debug)]
pub(super) struct Pane {
    /// The folder it was started in, in the wire spelling.
    pub(super) cwd: String,
    pub(super) label: Option<String>,
    pub(super) tokens: BTreeMap<String, Value>,
}

impl Pane {
    pub(super) fn new(cwd: &str, label: Option<&str>) -> Self {
        Self {
            cwd: cwd.to_owned(),
            label: label.map(str::to_owned),
            tokens: BTreeMap::new(),
        }
    }
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

fn refused(code: &str, message: String) -> EngineError {
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
            tabs: vec![Tab::new(&ids.tab, "", &ids.pane, pane)],
            active_tab: ids.tab.clone(),
            tab_numbers: 1,
            pane_numbers: 1,
        });
        if focus || self.focused_workspace.is_none() {
            self.focused_workspace = Some(ids.workspace.clone());
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
        let workspace = self.workspace_mut(workspace)?;
        workspace.tab_numbers += 1;
        workspace.pane_numbers += 1;
        Ok((
            ids::tab_id(&workspace.id, workspace.tab_numbers),
            ids::pane_id(&workspace.id, workspace.pane_numbers),
        ))
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
        }
        Ok(())
    }

    /// Splits `target` along `direction`, `created` second at ratio 0.5.
    pub(super) fn add_split(
        &mut self,
        target: &str,
        direction: PaneLayoutDirection,
        created: &str,
        pane: Pane,
        focus: bool,
    ) -> Result<(), EngineError> {
        let (workspace_index, tab_index) = self.position(target)?;
        let workspace = &mut self.workspaces[workspace_index];
        let tab = &mut workspace.tabs[tab_index];
        split_leaf(&mut tab.root, target, direction, created);
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
        let workspace = &mut self.workspaces[workspace_index];
        let tab = &mut workspace.tabs[tab_index];
        tab.panes.remove(pane);
        if let Some(sibling) = remove_leaf(&mut tab.root, pane) {
            if tab.focused == pane {
                tab.focused = sibling;
                tab.zoomed = false;
            }
            return Ok(Closed::default());
        }
        let tab = tab.id.clone();
        let workspace = self.remove_tab_at(workspace_index, tab_index);
        Ok(Closed {
            tab: Some(tab),
            workspace,
        })
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

    /// The panes of a tab, in its tree's order.
    pub(super) fn tab_panes(&self, tab: &str) -> Result<Vec<String>, EngineError> {
        let (workspace, index) = self.tab_position(tab)?;
        let mut panes = Vec::new();
        self.workspaces[workspace].tabs[index]
            .root
            .collect_pane_ids(&mut panes);
        Ok(panes.into_iter().map(str::to_owned).collect())
    }

    pub(super) fn pane(&self, pane: &str) -> Result<&Pane, EngineError> {
        let (workspace, tab) = self.position(pane)?;
        self.workspaces[workspace].tabs[tab]
            .panes
            .get(pane)
            .ok_or_else(|| no_pane(pane))
    }

    fn pane_mut(&mut self, pane: &str) -> Result<&mut Pane, EngineError> {
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

    /// The whole session, as a snapshot of Herdr's would project it.
    pub(super) fn state(&self) -> Result<ProjectionState, EngineError> {
        let mut state = ProjectionState {
            tab_focus: None,
            tab_moves: None,
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
                worktree: None,
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
                let mut order = Vec::new();
                tab.root.collect_pane_ids(&mut order);
                for pane_id in order {
                    let pane = tab.panes.get(pane_id).ok_or_else(|| {
                        EngineError::Answer(format!(
                            "tab {} lays out pane {pane_id}, which it does not hold",
                            tab.id
                        ))
                    })?;
                    state.panes.push(ProjectedPane {
                        terminal_id: pane_id.to_owned(),
                        restore_error: None,
                        foreground_process: None,
                        agent_status: "unknown".to_owned(),
                        pane_id: pane_id.to_owned(),
                        workspace_id: workspace.id.clone(),
                        tab_id: tab.id.clone(),
                        cwd: Some(pane.cwd.clone()),
                        tokens: pane.tokens.clone(),
                        label: pane.label.clone(),
                        terminal_title: None,
                        terminal_title_stripped: None,
                    });
                }
                state.layouts.push(tab.layout(&workspace.id)?);
            }
        }
        Ok(state)
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
            .state()
            .unwrap()
            .panes
            .into_iter()
            .map(|pane| pane.pane_id)
            .collect()
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
        assert!(topology.state().unwrap().workspaces.is_empty());
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
                &beside,
                Pane::new("/w", None),
                true,
            )
            .unwrap();
        assert_eq!(pane_ids(&topology), [first.pane.clone(), beside.clone()]);
        assert_eq!(
            topology.state().unwrap().focused_pane_id.as_deref(),
            Some(beside.as_str())
        );

        assert_eq!(topology.remove_pane(&beside).unwrap(), Closed::default());
        assert_eq!(
            topology.state().unwrap().focused_pane_id.as_deref(),
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
        let below = topology.reserve_pane(&first.workspace).unwrap();
        topology
            .add_split(
                &first.pane,
                PaneLayoutDirection::Down,
                &below,
                Pane::new("/w", None),
                false,
            )
            .unwrap();
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
}
