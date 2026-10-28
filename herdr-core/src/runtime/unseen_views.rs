//! What an agent opened for the operator that they have not looked at yet
//! (PRD unseen-agent-views).
//!
//! An agent's `hide file open`, `hide diff open` or `hide browser open` marks
//! the View it lands in unseen (`Display::unseen`, D-06, D-11); nothing the
//! operator opens and no `hide browser` action does. The View is seen once it
//! is in front of the operator (D-07): a window that has the keyboard draws
//! the Workspace in front, its lane shows the documents, and the View is the
//! selected tab of the active area in the strip of the session at the
//! keyboard. The core knows all of that but the window, which the daemon
//! tells it, from every window at once, only when it changes
//! (`window_focus`).
//!
//! The record lives on the View, so it is saved with it, closes with it and
//! never outnumbers the open Views (D-09, D-12, B32). The rows read it before
//! the lineage: an operator's own row goes to Needs You with `볼 것` (D-05),
//! and a delegated row hands each View up to its ancestors as a descendant
//! signal, which an ancestor's own read through the CLI turns off (D-10,
//! D-22, D-23).

use std::collections::HashMap;

use super::workspace_view::WorkspaceKey;
use super::*;
use crate::model::UnseenViewFact;
use crate::view_feedback::{Opener, Unseen};

impl Runtime {
    /// Whether a window holding the keyboard draws the Workspace in front,
    /// as the daemon summed every window's report.
    pub(super) fn set_window_focused(&mut self, focused: bool) -> bool {
        if self.window_focused == focused {
            return false;
        }
        self.window_focused = focused;
        self.settle_seen_views();
        true
    }

    /// An agent's open landed in `view_id` of `key`: the operator has a new
    /// View to look at, unless it is the one in front of them now, which is
    /// decided here, in the same event, so it never shows (B5, B14).
    pub(super) fn mark_view_unseen(&mut self, key: &WorkspaceKey, view_id: &str) {
        self.settle_view_sessions();
        if self.looked_at_view().as_ref() == Some(&(key.clone(), view_id.to_owned())) {
            return;
        }
        let now = unix_milliseconds();
        let marked = self.change_view_layout(key, |layout, _| {
            let display = layout
                .display_mut(view_id)
                .ok_or_else(|| crate::view_layout::LayoutError::UnknownDisplay(view_id.into()))?;
            display.unseen = Some(Unseen::opened(now, display.unseen.as_ref()));
            Ok(((), true))
        });
        match marked {
            Ok(()) => {
                self.refresh_unseen_rows();
            }
            Err(error) => crate::diagnostic!(serde_json::json!({
                "component": "unseen_views",
                "kind": "unseen.mark_failed",
                "device": key.0,
                "view": view_id,
                "error": error.kind(),
            })),
        }
    }

    /// Settles before a snapshot read: the View in front is seen, and when a
    /// View list moved since the rows were laid (a View closed, one opened
    /// on another path) while anything is unseen, the rows are laid again.
    pub(super) fn settle_unseen_views(&mut self) {
        self.settle_seen_views();
        let Some(generation) = self.workspace_views.as_ref().map(|store| store.generation) else {
            return;
        };
        if generation == self.unseen_laid_generation {
            return;
        }
        self.unseen_laid_generation = generation;
        let rows = self
            .snapshot
            .navigator
            .agents
            .iter()
            .chain(
                self.snapshot
                    .status
                    .remote
                    .iter()
                    .filter_map(|status| status.session.as_ref())
                    .flat_map(|session| &session.agents),
            )
            .any(|agent| !agent.unseen_views.is_empty());
        let views = self
            .every_view_layout()
            .any(|(_, layout)| layout.displays().any(|display| display.unseen.is_some()));
        if rows || views {
            self.refresh_unseen_rows();
        }
    }

    /// The View in front of the operator now, if a window with the keyboard
    /// shows one (D-07).
    fn looked_at_view(&self) -> Option<(WorkspaceKey, String)> {
        if !self.window_focused || self.snapshot.ui_state.detail.shown {
            return None;
        }
        let key = self.front_workspace_key()?;
        let view = self.workspace_views.as_ref()?.views.get(&key.0, &key.1)?;
        if !view.open || !view.documents {
            return None;
        }
        let area = view.layout.active_area();
        let display = area
            .active
            .as_deref()
            .and_then(|id| area.displays.iter().find(|display| display.id == id))?;
        display
            .shown_to(self.strip_session(&key))
            .then(|| (key, display.id.clone()))
    }

    /// Records the View in front of the operator as seen. Runs before every
    /// snapshot read, after the strip is settled, and costs one lookup when
    /// nothing is unseen there.
    pub(super) fn settle_seen_views(&mut self) -> bool {
        let Some((key, view_id)) = self.looked_at_view() else {
            return false;
        };
        let unseen = self
            .view_layout_of(&key)
            .and_then(|layout| layout.display(&view_id))
            .is_some_and(|display| display.unseen.is_some());
        if !unseen {
            return false;
        }
        let seen = self.change_view_layout(&key, |layout, _| {
            let display = layout
                .display_mut(&view_id)
                .ok_or_else(|| crate::view_layout::LayoutError::UnknownDisplay(view_id.clone()))?;
            Ok(((), display.unseen.take().is_some()))
        });
        if let Err(error) = seen {
            crate::diagnostic!(serde_json::json!({
                "component": "unseen_views",
                "kind": "unseen.seen_failed",
                "device": key.0,
                "view": view_id,
                "error": error.kind(),
            }));
            return false;
        }
        self.refresh_unseen_rows();
        true
    }

    /// An ancestor of the agent that opened `view_id` read it through the CLI
    /// (D-22): its signal goes off on every ancestor, and the View stays
    /// unseen for the operator (D-23). Anyone else's read changes nothing.
    pub(super) fn note_ancestor_read(&mut self, key: &WorkspaceKey, view_id: &str, caller: &str) {
        let Some(opener) = self
            .view_layout_of(key)
            .and_then(|layout| layout.display(view_id))
            .filter(|display| display.unseen.as_ref().is_some_and(|u| !u.ancestors_read))
            .and_then(|display| display.opener.clone())
        else {
            return;
        };
        let Some(opener_pane) = self.opener_pane(&opener) else {
            return;
        };
        let ancestor = self
            .machine_agents(&opener.device_id)
            .iter()
            .find(|agent| agent.pane_id == opener_pane)
            .is_some_and(|agent| agent.lineage_path_pane_ids.iter().any(|pane| pane == caller));
        if !ancestor {
            return;
        }
        let read = self.change_view_layout(key, |layout, _| {
            let display = layout
                .display_mut(view_id)
                .ok_or_else(|| crate::view_layout::LayoutError::UnknownDisplay(view_id.into()))?;
            let unseen = display
                .unseen
                .as_mut()
                .ok_or_else(|| crate::view_layout::LayoutError::UnknownDisplay(view_id.into()))?;
            unseen.ancestors_read = true;
            Ok(((), true))
        });
        match read {
            Ok(()) => {
                self.refresh_unseen_rows();
            }
            Err(error) => crate::diagnostic!(serde_json::json!({
                "component": "unseen_views",
                "kind": "unseen.ancestor_read_failed",
                "device": key.0,
                "view": view_id,
                "error": error.kind(),
            })),
        }
    }

    /// The pane an opener's agent is in now, by its terminal and else its
    /// native session, never by a stored pane id (view-opener-feedback D-23).
    fn opener_pane(&self, opener: &Opener) -> Option<String> {
        let agents = self.machine_agents(&opener.device_id);
        opener_pane_among(self, opener, agents)
    }

    /// Lays on each row the Views its agent opened last that nobody has
    /// looked at, newest first. Called on every projection before the
    /// lineage; with no unseen View anywhere it only clears.
    pub(super) fn lay_unseen_views(&self, agents: &mut [SidebarAgentSnapshot]) {
        for agent in agents.iter_mut() {
            agent.unseen_views.clear();
        }
        let unseen = self
            .every_view_layout()
            .flat_map(|(key, layout)| {
                layout
                    .displays()
                    .filter(|display| display.unseen.is_some() && display.opener.is_some())
                    .map(move |display| (key.clone(), display))
            })
            .collect::<Vec<_>>();
        if unseen.is_empty() {
            return;
        }
        let mut by_pane: HashMap<String, Vec<UnseenViewFact>> = HashMap::new();
        for (key, display) in unseen {
            let (Some(opener), Some(record)) = (&display.opener, &display.unseen) else {
                continue;
            };
            let Some(pane) = opener_pane_among(self, opener, agents) else {
                continue;
            };
            let ids = self.agent_workspace_ids(&key);
            by_pane.entry(pane).or_default().push(UnseenViewFact {
                device_id: key.0.clone(),
                path: key.1.clone(),
                view_id: display.id.clone(),
                label: super::view_areas::display_label(display),
                workspace_id: ids.as_ref().map(|(workspace, _)| workspace.clone()),
                checkout_id: ids.map(|(_, checkout)| checkout),
                opened_unix_ms: record.opened_unix_ms,
                ancestors_read: record.ancestors_read,
            });
        }
        for agent in agents.iter_mut() {
            if let Some(mut views) = by_pane.remove(&agent.pane_id) {
                views.sort_by(|left, right| right.opened_unix_ms.cmp(&left.opened_unix_ms));
                agent.unseen_views = views;
            }
        }
    }

    /// Lays the unseen Views again on the rows already shown, after one was
    /// marked, seen, read or closed, and rebuilds what reads them: the
    /// groups and marks, and the ancestors' read axis.
    pub(super) fn refresh_unseen_rows(&mut self) -> bool {
        if let Some(store) = &self.workspace_views {
            self.unseen_laid_generation = store.generation;
        }
        // The lineage lays them on every machine's rows at once.
        self.refresh_agent_lineage() | self.refresh_pane_read_state()
    }

    /// Adds to a report's `body` the Views the session of `pane_id` opened
    /// that are still open, whether the operator has seen each, so the agent
    /// that answers for it can read or show them (D-21). A list that would
    /// not fit the letter is cut, and says how many it left out.
    pub(super) fn list_opened_views(&self, pane_id: &str, body: &mut String) {
        let Some(scope) = self.session_scope_of_pane(pane_id) else {
            return;
        };
        let lines = self
            .every_view_layout()
            .flat_map(|(_, layout)| layout.displays())
            .filter(|display| display.openers.iter().any(|opener| opener.of_session(&scope)))
            .map(|display| {
                let target = display.url.as_deref().unwrap_or(&display.path);
                let seen = if display.unseen.is_some() {
                    "not seen by the operator"
                } else {
                    "seen"
                };
                format!("- {} {} ({seen})", display.id, target)
            })
            .collect::<Vec<_>>();
        if lines.is_empty() {
            return;
        }
        let head = "\n\nViews this agent opened (read one with `hide view status <view-id>` or `hide browser snapshot <view-id>`; open it again from your own pane to show it to the operator):";
        let room = crate::delivery::BODY_LIMIT.saturating_sub(body.len());
        let mut list = head.to_owned();
        let mut left = lines.len();
        for line in &lines {
            // Room is kept for the line that says what was left out.
            if list.len() + line.len() + 64 > room {
                break;
            }
            list.push('\n');
            list.push_str(line);
            left -= 1;
        }
        if left == lines.len() {
            return;
        }
        if left > 0 {
            list.push_str(&format!("\n- and {left} more (hide view list)"));
        }
        body.push_str(&list);
    }

    /// A row the operator pressed in a list that has a View to look at:
    /// that View comes forward in its Workspace, after the pane (B3, D-14).
    pub(super) fn reveal_unseen_view(&mut self, pane_id: &str) {
        let Some(look) = self
            .snapshot
            .navigator
            .agents
            .iter()
            .find(|agent| agent.pane_id == pane_id)
            .and_then(|agent| agent.look.clone())
        else {
            return;
        };
        let (Some(workspace), Some(checkout)) = (look.workspace_id, look.checkout_id) else {
            return;
        };
        self.focus_view_display_of(&workspace, &checkout, &look.view_id);
    }
}

/// The machine a row runs on: a device's rows carry its prefix.
fn agent_device(runtime: &Runtime, agent: &SidebarAgentSnapshot) -> String {
    super::agent_sleep::pane_device(&agent.pane_id)
        .unwrap_or(runtime.node.as_str())
        .to_owned()
}

/// The row among `agents` an opener is now: the same terminal with the same
/// session, else the same session in another pane.
fn opener_pane_among(
    runtime: &Runtime,
    opener: &Opener,
    agents: &[SidebarAgentSnapshot],
) -> Option<String> {
    let mine = || {
        agents
            .iter()
            .filter(|agent| agent_device(runtime, agent) == opener.device_id)
    };
    let in_terminal = runtime
        .catalog_workspaces()
        .filter(|workspace| workspace.device_id == opener.device_id)
        .flat_map(|workspace| &workspace.checkouts)
        .flat_map(|checkout| &checkout.tabs)
        .flat_map(|tab| &tab.panes)
        .find(|pane| pane.terminal_id.as_deref() == Some(opener.terminal_id.as_str()))
        .and_then(|pane| mine().find(|agent| agent.pane_id == pane.id))
        .filter(|agent| agent.agent_kind == opener.provider && agent.lineage_session == opener.session);
    in_terminal
        .or_else(|| {
            let session = opener.session.as_ref()?;
            mine().find(|agent| agent.lineage_session.as_ref() == Some(session))
        })
        .map(|agent| agent.pane_id.clone())
}
