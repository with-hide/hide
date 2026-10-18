//! Checkout-owned named collections. Every collection reuses the Agent area
//! tree; moving between them never changes a Herdr tab or its processes.
use crate::agent_layout::{Layout, Tab};
use crate::split_tree::LayoutError;
use crate::workspace_views::WorkspaceView;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};

pub const GROUP_LIMIT: usize = 32;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub layout: Layout,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Groups {
    #[serde(default)]
    next_id: u64,
    #[serde(default)]
    pub items: Vec<Group>,
    #[serde(default)]
    pub selected: Option<String>,
    /// These are Hide identities, never Herdr topology identities.
    #[serde(default)]
    pub drafts: BTreeSet<String>,
    #[serde(skip)]
    undo: Option<Box<Undo>>,
}

#[derive(Clone, Debug, PartialEq)]
struct Undo {
    group: Group,
    index: usize,
    selected: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum GroupError {
    MissingGroup,
    InvalidName,
    DuplicateName,
    GroupLimit,
    MissingTab,
    NoUndo,
    StartPending,
    Layout(LayoutError),
}

impl From<LayoutError> for GroupError {
    fn from(error: LayoutError) -> Self {
        Self::Layout(error)
    }
}

impl WorkspaceView {
    pub fn agent_groups_snapshot(&self) -> crate::model::AgentGroupsSnapshot {
        crate::model::AgentGroupsSnapshot {
            selected: self.agent_groups.selected.clone(),
            groups: std::iter::once(crate::model::AgentGroupSnapshot {
                id: None,
                name: None,
                tabs: self
                    .agent_layout
                    .tree
                    .displays()
                    .map(|t| t.id.clone())
                    .collect(),
            })
            .chain(
                self.agent_groups
                    .items
                    .iter()
                    .map(|g| crate::model::AgentGroupSnapshot {
                        id: Some(g.id.clone()),
                        name: Some(g.name.clone()),
                        tabs: g.layout.tree.displays().map(|t| t.id.clone()).collect(),
                    }),
            )
            .collect(),
            drafts: self.agent_groups.drafts.iter().cloned().collect(),
            start_receipts: crate::runtime::group_starts::receipts(self),
        }
    }
    pub fn reconcile_agent_groups(
        &mut self,
        topology: &[(String, bool)],
        reserved: usize,
        admitted: &HashSet<String>,
        unconfirmed: &HashSet<&str>,
    ) -> Result<bool, LayoutError> {
        let before = (self.agent_layout.clone(), self.agent_groups.clone());
        let mut seen_changed = false;
        let actual_topology = topology;
        let mut topology = std::borrow::Cow::Borrowed(topology);
        for start in self.agent_starts.values_mut() {
            if start.state != crate::runtime::group_starts::StartState::Ready
                || start.seen_in_topology
            {
                continue;
            }
            if let Some(id) = start.live_tab_id.as_ref() {
                if actual_topology.iter().any(|(tab, _)| tab == id)
                    && !unconfirmed.contains(id.as_str())
                {
                    start.seen_in_topology = true;
                    seen_changed = true;
                } else if !actual_topology.iter().any(|(tab, _)| tab == id) {
                    // An acknowledged create can beat the next topology
                    // publication. Its receipt holds the slot only until the
                    // real tab has been observed once.
                    topology.to_mut().push((id.clone(), false));
                }
            }
        }
        // Named groups admit only their existing members. Ordinary arrivals
        // belong to ungrouped until an acknowledged placement moves them.
        let mut grouped = HashSet::new();
        for group in &mut self.agent_groups.items {
            let mut members: Vec<_> = topology
                .iter()
                .filter(|(id, delegated)| *delegated || group.layout.tree.display(id).is_some())
                .cloned()
                .collect();
            members.extend(
                group
                    .layout
                    .tree
                    .displays()
                    .filter(|t| self.agent_groups.drafts.contains(&t.id))
                    .map(|t| (t.id.clone(), false)),
            );
            group
                .layout
                .reconcile_admissions(&members, 0, &HashSet::new())?;
            grouped.extend(group.layout.tree.displays().map(|t| t.id.clone()));
        }
        let mut ungrouped: Vec<_> = topology
            .iter()
            .filter(|(id, _)| !grouped.contains(id))
            .cloned()
            .collect();
        ungrouped.extend(
            self.agent_layout
                .tree
                .displays()
                .filter(|t| self.agent_groups.drafts.contains(&t.id))
                .map(|t| (t.id.clone(), false)),
        );
        self.agent_layout
            .reconcile_admissions(&ungrouped, reserved + grouped.len(), admitted)?;
        Ok(seen_changed || self.agent_layout != before.0 || self.agent_groups != before.1)
    }

    pub fn group_layout(&self, group: Option<&str>) -> Option<&Layout> {
        match group {
            None => Some(&self.agent_layout),
            Some(id) => self
                .agent_groups
                .items
                .iter()
                .find(|g| g.id == id)
                .map(|g| &g.layout),
        }
    }

    pub fn group_layout_mut(&mut self, group: Option<&str>) -> Option<&mut Layout> {
        match group {
            None => Some(&mut self.agent_layout),
            Some(id) => self
                .agent_groups
                .items
                .iter_mut()
                .find(|g| g.id == id)
                .map(|g| &mut g.layout),
        }
    }

    pub fn selected_agent_layout(&self) -> &Layout {
        self.group_layout(self.agent_groups.selected.as_deref())
            .expect("selected group is kept valid by group mutation and load repair")
    }

    pub fn selected_agent_layout_mut(&mut self) -> &mut Layout {
        let selected = self.agent_groups.selected.clone();
        self.group_layout_mut(selected.as_deref())
            .expect("selected group is kept valid by group mutation and load repair")
    }

    /// Outer None means absent; inner None means the ungrouped collection.
    pub fn agent_tab_group(&self, tab: &str) -> Option<Option<&str>> {
        if self.agent_layout.tree.display(tab).is_some()
            || self.agent_layout.canvases.values().any(|id| id == tab)
        {
            return Some(None);
        }
        self.agent_groups
            .items
            .iter()
            .find(|g| {
                g.layout.tree.display(tab).is_some()
                    || g.layout.canvases.values().any(|id| id == tab)
            })
            .map(|g| Some(g.id.as_str()))
    }

    pub fn agent_tab_count(&self) -> usize {
        self.agent_layout.tree.display_count()
            + self
                .agent_groups
                .items
                .iter()
                .map(|g| g.layout.tree.display_count())
                .sum::<usize>()
    }

    fn group_name(&self, name: &str, excluding: Option<&str>) -> Result<String, GroupError> {
        if name
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '\u{2028}' | '\u{2029}'))
        {
            return Err(GroupError::InvalidName);
        }
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 80 {
            return Err(GroupError::InvalidName);
        }
        if self
            .agent_groups
            .items
            .iter()
            .any(|g| Some(g.id.as_str()) != excluding && g.name == name)
        {
            return Err(GroupError::DuplicateName);
        }
        Ok(name.to_owned())
    }

    pub fn create_agent_group(&mut self, name: &str) -> Result<String, GroupError> {
        let name = self.group_name(name, None)?;
        if self.agent_groups.items.len() >= GROUP_LIMIT {
            return Err(GroupError::GroupLimit);
        }
        self.agent_groups.next_id += 1;
        let id = format!("g{}", self.agent_groups.next_id);
        self.agent_groups.items.push(Group {
            id: id.clone(),
            name,
            layout: Layout::default(),
        });
        self.agent_groups.selected = Some(id.clone());
        self.agent_groups.undo = None;
        Ok(id)
    }

    pub fn rename_agent_group(&mut self, id: &str, name: &str) -> Result<(), GroupError> {
        let name = self.group_name(name, Some(id))?;
        let group = self
            .agent_groups
            .items
            .iter_mut()
            .find(|g| g.id == id)
            .ok_or(GroupError::MissingGroup)?;
        group.name = name;
        self.agent_groups.undo = None;
        Ok(())
    }

    pub fn select_agent_group(&mut self, id: Option<&str>) -> Result<bool, GroupError> {
        if self.group_layout(id).is_none() {
            return Err(GroupError::MissingGroup);
        }
        let changed = self.agent_groups.selected.as_deref() != id;
        self.agent_groups.selected = id.map(str::to_owned);
        Ok(changed)
    }

    pub fn move_agent_to_group(
        &mut self,
        tab: &str,
        target: Option<&str>,
        stamp: u64,
    ) -> Result<(), GroupError> {
        if self.agent_starts.values().any(|start| {
            start.is_pending()
                && (start.draft_tab_id.as_deref() == Some(tab)
                    || start.live_tab_id.as_deref() == Some(tab))
        }) {
            return Err(GroupError::StartPending);
        }
        let source = self
            .agent_tab_group(tab)
            .ok_or(GroupError::MissingTab)?
            .map(str::to_owned);
        if self.group_layout(target).is_none() {
            return Err(GroupError::MissingGroup);
        }
        if source.as_deref() != target {
            // Validate the destination before removing anything from the source.
            let mut destination = self.group_layout(target).unwrap().clone();
            let item = self
                .group_layout(source.as_deref())
                .unwrap()
                .tree
                .display(tab)
                .ok_or(GroupError::MissingTab)?
                .clone();
            let area = destination.tree.active_area.clone();
            destination.tree.insert(&area, item, stamp)?;
            destination.select(tab, false, stamp)?;
            let source_layout = self.group_layout_mut(source.as_deref()).unwrap();
            source_layout.tree.remove(tab);
            source_layout
                .canvases
                .retain(|area, _| source_layout.tree.area(area).is_some());
            *self.group_layout_mut(target).unwrap() = destination;
        } else {
            self.group_layout_mut(target)
                .unwrap()
                .select(tab, false, stamp)?;
        }
        self.agent_groups.selected = target.map(str::to_owned);
        self.agent_groups.undo = None;
        Ok(())
    }

    pub fn dissolve_agent_group(&mut self, id: &str) -> Result<(), GroupError> {
        if self
            .agent_starts
            .values()
            .any(|start| start.is_pending() && start.group_id.as_deref() == Some(id))
        {
            return Err(GroupError::StartPending);
        }
        let index = self
            .agent_groups
            .items
            .iter()
            .position(|g| g.id == id)
            .ok_or(GroupError::MissingGroup)?;
        let group = self.agent_groups.items[index].clone();
        let mut destination = self.agent_layout.clone();
        let area = destination.tree.active_area.clone();
        for tab in group.layout.tree.displays() {
            destination.tree.append(&area, tab.clone())?;
        }
        // A delegated canvas follows its owning collection without a strip slot.
        if let Some(tab) = group
            .layout
            .active()
            .filter(|id| group.layout.canvases.values().any(|t| t == id))
        {
            // Dissolving a group appends its tabs without replacing what
            // ungrouped already shows. Its prior delegated canvas also has
            // to remain available after Undo restores the removed group.
            destination
                .canvases
                .entry(area)
                .or_insert_with(|| tab.to_owned());
        }
        let undo = Undo {
            group,
            index,
            selected: self.agent_groups.selected.clone(),
        };
        self.agent_layout = destination;
        self.agent_groups.items.remove(index);
        if self.agent_groups.selected.as_deref() == Some(id) {
            self.agent_groups.selected = None;
        }
        self.agent_groups.undo = Some(Box::new(undo));
        Ok(())
    }

    pub fn can_undo_agent_group(&self) -> bool {
        self.agent_groups.undo.is_some()
    }
    pub fn invalidate_agent_group_undo(&mut self) {
        self.agent_groups.undo = None;
    }

    pub fn undo_agent_group(&mut self) -> Result<(), GroupError> {
        let mut undo = self.agent_groups.undo.take().ok_or(GroupError::NoUndo)?;
        let ids: Vec<_> = undo
            .group
            .layout
            .tree
            .displays()
            .map(|t| t.id.clone())
            .collect();
        for id in ids {
            if self.agent_layout.tree.remove(&id).is_none() {
                undo.group.layout.tree.remove(&id);
            }
        }
        self.agent_layout
            .canvases
            .retain(|_, id| !undo.group.layout.canvases.values().any(|tab| tab == id));
        self.agent_layout
            .canvases
            .retain(|area, _| self.agent_layout.tree.area(area).is_some());
        self.agent_groups.items.insert(undo.index, undo.group);
        self.agent_groups.selected = undo.selected;
        Ok(())
    }

    pub fn create_agent_draft(&mut self, area: &str, stamp: u64) -> Result<String, GroupError> {
        if self.agent_tab_count() >= 64 {
            return Err(LayoutError::DisplayLimit.into());
        }
        self.agent_groups.next_id += 1;
        let id = format!("draft:{}", self.agent_groups.next_id);
        self.selected_agent_layout_mut()
            .tree
            .insert(area, Tab::new(id.clone()), stamp)?;
        self.selected_agent_layout_mut().select(&id, false, stamp)?;
        self.agent_groups.drafts.insert(id.clone());
        self.agent_groups.undo = None;
        Ok(id)
    }

    pub fn repair_agent_groups(&mut self) -> Vec<String> {
        let mut notes = self.agent_layout.repair();
        let mut ids = BTreeSet::new();
        let mut names = BTreeSet::new();
        let mut tabs: BTreeSet<String> = self
            .agent_layout
            .tree
            .displays()
            .map(|t| t.id.clone())
            .collect();
        self.agent_groups.items.retain_mut(|g| {
            if g.id.is_empty() || !ids.insert(g.id.clone()) || !names.insert(g.name.clone()) {
                notes.push("duplicate or empty Agent group identity".into());
                return false;
            }
            notes.extend(g.layout.repair());
            let repeated: Vec<_> = g
                .layout
                .tree
                .displays()
                .filter(|t| !tabs.insert(t.id.clone()))
                .map(|t| t.id.clone())
                .collect();
            for tab in repeated {
                g.layout.tree.remove(&tab);
            }
            true
        });
        if self
            .group_layout(self.agent_groups.selected.as_deref())
            .is_none()
        {
            self.agent_groups.selected = None;
            notes.push("missing selected Agent group".into());
        }
        self.agent_groups.drafts.retain(|id| tabs.contains(id));
        notes
    }
}
