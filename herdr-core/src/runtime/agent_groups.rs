//! Named-group actions are one core transition, with no Herdr mutation.
use super::view_areas::ViewWorkspace;
use super::*;
use crate::agent_groups::GroupError;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(super) struct AgentGroupPayload {
    workspace: ViewWorkspace,
    request_id: String,
    #[serde(default)]
    group_id: Option<String>,
    #[serde(flatten)]
    action: Action,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Action {
    Create {
        name: String,
        tab_id: Option<String>,
    },
    Rename {
        name: String,
    },
    Select {
        tab_id: Option<String>,
    },
    Move {
        tab_id: String,
    },
    Dissolve,
    Undo,
    NewTab {
        area_id: Option<String>,
    },
    CloseDraft {
        tab_id: String,
    },
}

impl Runtime {
    pub(super) fn apply_agent_group(&mut self, payload: AgentGroupPayload) -> bool {
        let key = (payload.workspace.device_id, payload.workspace.path);
        if payload.request_id.is_empty()
            || payload.request_id.len() > 128
            || payload.request_id.chars().any(char::is_control)
        {
            self.set_error(
                "agent_group.invalid_request",
                "The group request identity is invalid.",
                false,
            );
            return true;
        }
        if !self
            .catalog_workspaces()
            .filter(|workspace| workspace.device_id == key.0)
            .flat_map(|workspace| &workspace.checkouts)
            .any(|checkout| checkout.path == key.1)
        {
            self.set_request_error(
                "agent_group.checkout_missing",
                "The checkout is no longer available.",
                false,
                Some(&payload.request_id),
            );
            return true;
        }
        let request = format!("group:{}", payload.request_id);
        if self.split_request_seen(&key, &request) {
            return false;
        }
        if matches!(payload.action, Action::NewTab { .. })
            && !self.admit_agent_tab_for_workspace(&key)
        {
            return true;
        }
        if let Action::CloseDraft { tab_id } = &payload.action {
            let changed = self.close_agent_draft(&key, tab_id);
            if changed {
                self.remember_split_request(&key, request);
            }
            return changed;
        }
        let Some(store) = self.workspace_views.as_mut() else {
            return false;
        };
        let view = store.views.entry(&key.0, &key.1);
        let before = view.clone();
        let stamp = unix_milliseconds();
        let result = (|| -> Result<(), GroupError> {
            match payload.action {
                Action::Create { name, tab_id } => {
                    let group = view.create_agent_group(&name)?;
                    if let Some(tab) = tab_id {
                        view.move_agent_to_group(&tab, Some(&group), stamp)?;
                    }
                }
                Action::Rename { name } => view.rename_agent_group(
                    payload
                        .group_id
                        .as_deref()
                        .ok_or(GroupError::MissingGroup)?,
                    &name,
                )?,
                Action::Select { tab_id } => {
                    view.select_agent_group(payload.group_id.as_deref())?;
                    if let Some(tab) = tab_id {
                        view.selected_agent_layout_mut()
                            .select(&tab, false, stamp)?;
                    }
                }
                Action::Move { tab_id } => {
                    view.move_agent_to_group(&tab_id, payload.group_id.as_deref(), stamp)?
                }
                Action::Dissolve => view.dissolve_agent_group(
                    payload
                        .group_id
                        .as_deref()
                        .ok_or(GroupError::MissingGroup)?,
                )?,
                Action::Undo => view.undo_agent_group()?,
                Action::NewTab { area_id } => {
                    if view.agent_groups.selected != payload.group_id {
                        return Err(GroupError::MissingGroup);
                    }
                    let area = area_id
                        .unwrap_or_else(|| view.selected_agent_layout().tree.active_area.clone());
                    view.create_agent_draft(&area, stamp)?;
                }
                Action::CloseDraft { .. } => unreachable!("handled before group mutation"),
            }
            Ok(())
        })();
        if let Err(error) = result {
            *view = before;
            let message = match error {
                GroupError::InvalidName => {
                    "Use a group name of 1 to 80 characters without line breaks or control characters."
                }
                GroupError::DuplicateName => "A group with that name already exists.",
                GroupError::GroupLimit => "At most 32 groups can belong to a checkout.",
                GroupError::NoUndo => "The group can no longer be restored.",
                GroupError::StartPending => {
                    "A start is still being confirmed in this tab. Check its result before moving or closing it."
                }
                GroupError::Layout(crate::split_tree::LayoutError::DisplayLimit) => {
                    "64 Agent tabs are placed or opening. Close a tab to make room."
                }
                _ => "The group or tab changed. Select it again and retry.",
            };
            self.set_request_error(
                "agent_group.refused",
                message,
                false,
                Some(&payload.request_id),
            );
            return true;
        }
        let changed = *view != before;
        let active = view.selected_agent_layout().active().map(str::to_owned);
        let live = active.filter(|id| !view.agent_groups.drafts.contains(id));
        self.remember_split_request(&key, request);
        if changed {
            self.persist_workspace_views();
        }
        self.show_agent_workspace_tab(&key, live);
        true
    }

    fn close_agent_draft(&mut self, key: &super::workspace_view::WorkspaceKey, id: &str) -> bool {
        let Some(view) = self
            .workspace_views
            .as_mut()
            .and_then(|s| s.views.get_mut(&key.0, &key.1))
        else {
            return false;
        };
        if !view.agent_groups.drafts.contains(id) {
            return false;
        }
        if view
            .agent_starts
            .values()
            .any(|start| start.is_pending() && start.draft_tab_id.as_deref() == Some(id))
        {
            self.set_error(
                "agent_group.start_pending",
                "A start is still being confirmed in this tab. Check its result before closing it.",
                false,
            );
            return true;
        }
        let Some(group) = view.agent_tab_group(id).map(|g| g.map(str::to_owned)) else {
            return false;
        };
        let layout = view
            .group_layout_mut(group.as_deref())
            .expect("draft has a group");
        let Some(area) = layout.tree.area_of(id) else {
            return false;
        };
        let area_id = area.id.clone();
        let index = area
            .displays
            .iter()
            .position(|t| t.id == id)
            .expect("draft is in area");
        let tab = layout.tree.remove(id).expect("draft is in area");
        view.agent_groups.drafts.remove(id);
        view.invalidate_agent_group_undo();
        let live = view
            .selected_agent_layout()
            .active()
            .filter(|id| !view.agent_groups.drafts.contains(*id))
            .map(str::to_owned);
        self.push_recent_closed(ClosedItem::Draft {
            key: format!("draft-close:{}:{}:{id}", key.0, key.1),
            device_id: key.0.clone(),
            checkout_path: key.1.clone(),
            group_id: group,
            area_id,
            index,
            tab,
        });
        self.persist_workspace_views();
        self.show_agent_workspace_tab(key, live);
        true
    }

    pub(super) fn reopen_agent_draft(&mut self, item: ClosedItem) -> bool {
        let ClosedItem::Draft {
            key,
            device_id,
            checkout_path,
            group_id,
            area_id,
            index,
            tab,
        } = item
        else {
            return false;
        };
        let workspace = (device_id.clone(), checkout_path.clone());
        if !self.admit_agent_tab_for_workspace(&workspace) {
            return true;
        }
        let Some(view) = self
            .workspace_views
            .as_mut()
            .and_then(|s| s.views.get_mut(&device_id, &checkout_path))
        else {
            self.set_error(
                "agent_group.checkout_missing",
                "The closed tab's checkout is no longer available.",
                false,
            );
            return true;
        };
        let before = view.clone();
        let group = group_id
            .clone()
            .filter(|id| view.group_layout(Some(id)).is_some());
        let index = if group == group_id { index } else { usize::MAX };
        let layout = view
            .group_layout_mut(group.as_deref())
            .expect("group fallback checked");
        let area = if group == group_id && layout.tree.area(&area_id).is_some() {
            area_id
        } else {
            layout.tree.active_area.clone()
        };
        let id = tab.id.clone();
        let result = layout
            .tree
            .insert(&area, tab, unix_milliseconds())
            .and_then(|_| {
                layout
                    .tree
                    .move_display(&id, &area, index, unix_milliseconds())
            });
        if let Err(error) = result {
            *view = before;
            self.push_diagnostic("agent_group.reopen_failed", format!("{error:?}"));
            self.set_error(
                "agent_group.reopen_failed",
                "The tab could not be restored. Close a tab and retry.",
                false,
            );
            return true;
        }
        view.agent_groups.drafts.insert(id);
        view.agent_groups.selected = group;
        view.invalidate_agent_group_undo();
        self.consume_recent_closed(&key);
        self.persist_workspace_views();
        self.show_agent_workspace_tab(&workspace, None);
        true
    }
}
