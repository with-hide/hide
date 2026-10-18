//! Hide-owned Agent groups. Herdr tab identity and delegated canvas policy
//! adapt the shared tree; no operation here changes Herdr topology.
use crate::split_tree::{AreaItem, SplitTree, TreeLimits};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Tab {
    pub id: String,
    #[serde(default)]
    focused: u64,
}
impl Tab {
    pub fn new(id: String) -> Self {
        Self { id, focused: 0 }
    }
}
impl AreaItem for Tab {
    const LIMITS: TreeLimits = TreeLimits {
        areas: 6,
        depth: 3,
        items: 64,
    };
    const MINTED_IDENTITY: bool = false;
    fn id(&self) -> &str {
        &self.id
    }
    fn id_mut(&mut self) -> &mut String {
        &mut self.id
    }
    fn focus_stamp(&self) -> u64 {
        self.focused
    }
    fn set_focus_stamp(&mut self, stamp: u64) {
        self.focused = stamp;
    }
    fn keep_open(&mut self) {}
    fn same_content(&self, other: &Self) -> bool {
        self.id == other.id
    }
    fn repair_area(_: &mut [Self], _: Option<&str>, _: &mut Vec<String>) {}
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Layout {
    #[serde(flatten)]
    pub tree: SplitTree<Tab>,
    /// A delegated tab occupies a canvas without acquiring a strip slot.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub canvases: BTreeMap<String, String>,
    #[serde(skip)]
    pub waiting: usize,
}
impl Layout {
    pub fn repair(&mut self) -> Vec<String> {
        let mut seen = HashSet::new();
        let ids: Vec<_> = self
            .tree
            .areas()
            .iter()
            .map(|area| area.id.clone())
            .collect();
        for id in ids {
            if let Some(area) = self.tree.area_mut(&id) {
                area.displays
                    .retain(|tab| !tab.id.is_empty() && seen.insert(tab.id.clone()));
            }
        }
        let notes = self.tree.repair();
        self.canvases
            .retain(|area, _| self.tree.area(area).is_some());
        notes
    }

    /// Called only with authoritative topology, never with a startup placeholder.
    /// New external tabs append without changing what any area shows.
    pub fn reconcile(
        &mut self,
        tabs: &[(String, bool)],
    ) -> Result<bool, crate::split_tree::LayoutError> {
        self.reconcile_admissions(tabs, 0, &HashSet::new())
    }

    pub fn reconcile_admissions(
        &mut self,
        tabs: &[(String, bool)],
        reserved: usize,
        admitted: &HashSet<String>,
    ) -> Result<bool, crate::split_tree::LayoutError> {
        let before = self.clone();
        let members: HashSet<_> = tabs
            .iter()
            .filter(|(_, delegated)| !delegated)
            .map(|(id, _)| id.as_str())
            .collect();
        let gone: Vec<_> = self
            .tree
            .displays()
            .filter(|tab| !members.contains(tab.id.as_str()))
            .map(|tab| tab.id.clone())
            .collect();
        for id in gone {
            self.tree.remove(&id);
        }
        self.canvases.retain(|area, tab| {
            self.tree.area(area).is_some()
                && tabs.iter().any(|(id, delegated)| id == tab && *delegated)
        });
        self.waiting = 0;
        // Confirmed local effects own their reserved slots even if an external
        // creation was observed first. Unknown effects keep capacity reserved.
        let absent = admitted
            .iter()
            .filter(|id| !members.contains(id.as_str()))
            .count();
        let capacity = Tab::LIMITS.items.saturating_sub(reserved + absent);
        for (id, delegated) in tabs
            .iter()
            .filter(|(id, _)| admitted.contains(id))
            .chain(tabs.iter().filter(|(id, _)| !admitted.contains(id)))
        {
            if !delegated && self.tree.display(id).is_none() {
                if self.tree.displays().count() >= capacity {
                    self.waiting += 1;
                    continue;
                }
                let area = self.tree.active_area.clone();
                if let Err(error) = self.tree.append(
                    &area,
                    Tab {
                        id: id.clone(),
                        focused: 0,
                    },
                ) {
                    *self = before;
                    return Err(error);
                }
            }
        }
        Ok(*self != before)
    }

    pub fn active(&self) -> Option<&str> {
        self.shown_in(&self.tree.active_area)
    }
    pub fn shown_in(&self, area: &str) -> Option<&str> {
        self.canvases
            .get(area)
            .map(String::as_str)
            .or_else(|| self.tree.area(area)?.active.as_deref())
    }
    pub fn shown(&self) -> Vec<String> {
        self.tree
            .areas()
            .iter()
            .filter_map(|area| self.shown_in(&area.id).map(str::to_owned))
            .collect()
    }
    pub fn select(
        &mut self,
        id: &str,
        delegated: bool,
        stamp: u64,
    ) -> Result<bool, crate::split_tree::LayoutError> {
        if delegated {
            self.canvases
                .retain(|area, tab| tab != id || area == &self.tree.active_area);
            return Ok(self
                .canvases
                .insert(self.tree.active_area.clone(), id.to_owned())
                .as_deref()
                != Some(id));
        }
        let changed = self.tree.focus(id, stamp)?;
        Ok(self.canvases.remove(&self.tree.active_area).is_some() || changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::split_tree::Edge;
    fn tabs(ids: &[&str]) -> Vec<(String, bool)> {
        ids.iter().map(|id| (id.to_string(), false)).collect()
    }
    #[test]
    fn topology_reorder_does_not_reorder_groups_and_new_tabs_do_not_steal_focus() {
        let mut layout = Layout::default();
        layout
            .reconcile(&tabs(&["w1:t1", "w1:t2", "w1:t3"]))
            .unwrap();
        let right = layout.tree.split("w1:t2", "a1", Edge::Right, 1).unwrap();
        layout
            .reconcile(&tabs(&["w1:t3", "w1:t1", "w1:t2", "w1:t4"]))
            .unwrap();
        assert_eq!(layout.active(), Some("w1:t2"));
        assert_eq!(
            layout
                .tree
                .area(&right)
                .unwrap()
                .displays
                .iter()
                .map(|t| t.id.as_str())
                .collect::<Vec<_>>(),
            ["w1:t2", "w1:t4"]
        );
        layout.reconcile(&tabs(&["w1:t1", "w1:t3"])).unwrap();
        assert_eq!(layout.tree.area_count(), 1);
        assert_eq!(layout.shown(), ["w1:t1"]);
    }
    #[test]
    fn delegated_tab_occupies_only_a_canvas_and_saved_layout_keeps_stable_tab_ids() {
        let mut layout = Layout::default();
        let mut topology = tabs(&["w1:t1", "w1:t2"]);
        topology.push(("w1:t3".into(), true));
        layout.reconcile(&topology).unwrap();
        layout.tree.split("w1:t2", "a1", Edge::Down, 1).unwrap();
        layout.select("w1:t3", true, 2).unwrap();
        let mut restored: Layout =
            serde_json::from_str(&serde_json::to_string(&layout).unwrap()).unwrap();
        restored.repair();
        restored.reconcile(&topology).unwrap();
        assert_eq!(restored.active(), Some("w1:t3"));
        assert!(restored.tree.display("w1:t3").is_none());
        assert_eq!(restored.shown(), ["w1:t1", "w1:t3"]);
        restored.select("w1:t2", false, 3).unwrap();
        assert_eq!(restored.shown(), ["w1:t1", "w1:t2"]);
    }
}
