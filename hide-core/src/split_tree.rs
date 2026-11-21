//! Shared ordered-area binary tree. Items supply identity and domain policy;
//! topology, focus, movement, collapse and structural repair have one owner.
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MIN_SPLIT_RATIO: f32 = 0.15;
pub const MAX_SPLIT_RATIO: f32 = 0.85;
const DEFAULT_SPLIT_RATIO: f32 = 0.5;

#[derive(Clone, Copy, Debug)]
pub struct TreeLimits {
    pub areas: usize,
    pub depth: usize,
    pub items: usize,
}

pub trait AreaItem: Clone {
    const LIMITS: TreeLimits;
    /// Domain-owned identities (the engine tabs) must never be reminted by repair.
    const MINTED_IDENTITY: bool;
    fn id(&self) -> &str;
    fn id_mut(&mut self) -> &mut String;
    fn focus_stamp(&self) -> u64;
    fn set_focus_stamp(&mut self, stamp: u64);
    fn keep_open(&mut self);
    fn same_content(&self, other: &Self) -> bool;
    fn repair_area(items: &mut [Self], active: Option<&str>, notes: &mut Vec<String>);
}

/// `row` puts the first child left of the second, `column` above it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitAxis {
    Row,
    Column,
}

/// A side of an area: where a split puts the new area, or which way a
/// neighbour lies.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    Left,
    Right,
    Up,
    Down,
}

impl Edge {
    fn axis(self) -> SplitAxis {
        match self {
            Self::Left | Self::Right => SplitAxis::Row,
            Self::Up | Self::Down => SplitAxis::Column,
        }
    }

    /// Whether this side comes second in a split: right of or below.
    fn after(self) -> bool {
        matches!(self, Self::Right | Self::Down)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(bound(deserialize = "I: Deserialize<'de>"))]
pub struct Area<I> {
    pub id: String,
    /// The display the area shows; `None` only when it holds none.
    #[serde(default)]
    pub active: Option<String>,
    #[serde(default)]
    pub displays: Vec<I>,
}

impl<I: AreaItem> Area<I> {
    pub(crate) fn empty(id: String) -> Self {
        Self {
            id,
            active: None,
            displays: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(bound(deserialize = "I: Deserialize<'de>"))]
pub struct Split<I> {
    pub id: String,
    pub axis: SplitAxis,
    pub ratio: f32,
    pub first: Box<Node<I>>,
    pub second: Box<Node<I>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(bound(deserialize = "I: Deserialize<'de>"))]
#[serde(rename_all = "snake_case")]
pub enum Node<I> {
    Area(Area<I>),
    Split(Split<I>),
}

impl<I: AreaItem> Node<I> {
    fn collect_areas<'a>(&'a self, out: &mut Vec<&'a Area<I>>) {
        match self {
            Self::Area(area) => out.push(area),
            Self::Split(split) => {
                split.first.collect_areas(out);
                split.second.collect_areas(out);
            }
        }
    }

    fn collect_displays_mut<'a>(&'a mut self, out: &mut Vec<&'a mut I>) {
        match self {
            Self::Area(area) => out.extend(area.displays.iter_mut()),
            Self::Split(split) => {
                split.first.collect_displays_mut(out);
                split.second.collect_displays_mut(out);
            }
        }
    }

    fn area_mut(&mut self, id: &str) -> Option<&mut Area<I>> {
        match self {
            Self::Area(area) => (area.id == id).then_some(area),
            Self::Split(split) => match split.first.area_mut(id) {
                Some(area) => Some(area),
                None => split.second.area_mut(id),
            },
        }
    }

    fn split_mut(&mut self, id: &str) -> Option<&mut Split<I>> {
        let Self::Split(split) = self else {
            return None;
        };
        if split.id == id {
            return Some(split);
        }
        match split.first.split_mut(id) {
            Some(found) => Some(found),
            None => split.second.split_mut(id),
        }
    }

    fn first_area(&self) -> &Area<I> {
        match self {
            Self::Area(area) => area,
            Self::Split(split) => split.first.first_area(),
        }
    }

    fn last_area(&self) -> &Area<I> {
        match self {
            Self::Area(area) => area,
            Self::Split(split) => split.second.last_area(),
        }
    }

    fn depth(&self) -> usize {
        match self {
            Self::Area(_) => 0,
            Self::Split(split) => 1 + split.first.depth().max(split.second.depth()),
        }
    }

    /// Puts `fresh` at `edge` of the area `area_id`, the two sharing that
    /// area's space in a new split. Returns whether the area was found.
    fn wrap_area(
        &mut self,
        area_id: &str,
        edge: Edge,
        fresh: &mut Option<Area<I>>,
        split_id: &str,
    ) -> bool {
        if matches!(self, Self::Area(area) if area.id == area_id) {
            let Some(fresh) = fresh.take() else {
                return false;
            };
            let old = std::mem::replace(self, Self::Area(Area::empty(String::new())));
            let fresh = Self::Area(fresh);
            let (first, second) = if edge.after() {
                (old, fresh)
            } else {
                (fresh, old)
            };
            *self = Self::Split(Split {
                id: split_id.to_owned(),
                axis: edge.axis(),
                ratio: DEFAULT_SPLIT_RATIO,
                first: Box::new(first),
                second: Box::new(second),
            });
            return true;
        }
        match self {
            Self::Area(_) => false,
            Self::Split(split) => {
                split.first.wrap_area(area_id, edge, fresh, split_id)
                    || split.second.wrap_area(area_id, edge, fresh, split_id)
            }
        }
    }

    /// Replaces the split that directly holds the area `area_id` with its
    /// other child, which takes the space. Returns the area of that child
    /// nearest the removed one.
    fn remove_area(&mut self, area_id: &str) -> Option<String> {
        let (first_is, second_is) = match self {
            Self::Area(_) => return None,
            Self::Split(split) => (
                matches!(&*split.first, Self::Area(area) if area.id == area_id),
                matches!(&*split.second, Self::Area(area) if area.id == area_id),
            ),
        };
        if first_is || second_is {
            let Self::Split(split) =
                std::mem::replace(self, Self::Area(Area::empty(String::new())))
            else {
                unreachable!("matched a split above");
            };
            let sibling = if first_is {
                *split.second
            } else {
                *split.first
            };
            let nearest = if first_is {
                sibling.first_area().id.clone()
            } else {
                sibling.last_area().id.clone()
            };
            *self = sibling;
            return Some(nearest);
        }
        match self {
            Self::Area(_) => None,
            Self::Split(split) => split
                .first
                .remove_area(area_id)
                .or_else(|| split.second.remove_area(area_id)),
        }
    }

    /// Replaces every split whose areas would sit deeper than the cap with
    /// one area holding their displays in order. `depth` is this node's.
    fn flatten_below(&mut self, depth: usize, notes: &mut Vec<String>) {
        if !matches!(self, Self::Split(_)) {
            return;
        }
        if depth >= I::LIMITS.depth {
            let merged = merge_areas(self);
            notes.push(format!(
                "areas deeper than {} splits were merged into {}",
                I::LIMITS.depth,
                merged.id
            ));
            *self = Self::Area(merged);
            return;
        }
        if let Self::Split(split) = self {
            split.first.flatten_below(depth + 1, notes);
            split.second.flatten_below(depth + 1, notes);
        }
    }

    /// Merges the last split whose children are both areas into one area.
    fn merge_last_pair(&mut self) -> Option<String> {
        let Self::Split(split) = self else {
            return None;
        };
        if let Some(merged) = split.second.merge_last_pair() {
            return Some(merged);
        }
        if let Some(merged) = split.first.merge_last_pair() {
            return Some(merged);
        }
        let merged = merge_areas(self);
        let id = merged.id.clone();
        *self = Self::Area(merged);
        Some(id)
    }

    fn visit_mut(&mut self, visit: &mut impl FnMut(&mut Self)) {
        visit(self);
        if let Self::Split(split) = self {
            split.first.visit_mut(visit);
            split.second.visit_mut(visit);
        }
    }
}

/// One area holding every display of `node` in order, named after its first
/// area.
fn merge_areas<I: AreaItem>(node: &Node<I>) -> Area<I> {
    let mut areas = Vec::new();
    node.collect_areas(&mut areas);
    let mut merged = Area::empty(areas[0].id.clone());
    merged.active = areas[0].active.clone();
    for area in areas {
        merged.displays.extend(area.displays.iter().cloned());
    }
    merged
}

/// Why an operation was refused. Nothing changed when one is returned.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LayoutError {
    UnknownDisplay(String),
    UnknownArea(String),
    UnknownSplit(String),
    AreaLimit,
    DepthLimit,
    DisplayLimit,
    NothingToSplit,
    InvalidRatio,
}

/// A split ratio inside the bounds, or `None` for a value that is not one.
pub fn clamp_ratio(ratio: f32) -> Option<f32> {
    ratio
        .is_finite()
        .then(|| ratio.clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO))
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(bound(deserialize = "I: Deserialize<'de>"))]
pub struct SplitTree<I> {
    pub root: Node<I>,
    /// The area used last: where an open lands (B1).
    pub active_area: String,
    /// The next number an id takes; ids are never reused in a Workspace.
    pub next_id: u64,
}

impl<I: AreaItem> Default for SplitTree<I> {
    /// One empty area, which is what a Workspace with no View has.
    fn default() -> Self {
        Self {
            root: Node::Area(Area::empty("a1".to_owned())),
            active_area: "a1".to_owned(),
            next_id: 2,
        }
    }
}

impl<I: AreaItem> SplitTree<I> {
    /// The areas in tree order: left to right, top to bottom.
    pub fn areas(&self) -> Vec<&Area<I>> {
        let mut areas = Vec::new();
        self.root.collect_areas(&mut areas);
        areas
    }

    pub fn area(&self, id: &str) -> Option<&Area<I>> {
        self.areas().into_iter().find(|area| area.id == id)
    }

    pub fn area_mut(&mut self, id: &str) -> Option<&mut Area<I>> {
        self.root.area_mut(id)
    }

    /// The area in use; the first one if the stored name is stale.
    pub fn active_area(&self) -> &Area<I> {
        self.area(&self.active_area)
            .unwrap_or_else(|| self.root.first_area())
    }

    pub fn area_count(&self) -> usize {
        self.areas().len()
    }

    pub fn depth(&self) -> usize {
        self.root.depth()
    }

    /// Every display in tree order.
    pub fn displays(&self) -> impl Iterator<Item = &I> {
        self.areas()
            .into_iter()
            .flat_map(|area| area.displays.iter())
    }

    pub fn displays_mut(&mut self) -> Vec<&mut I> {
        let mut displays = Vec::new();
        self.root.collect_displays_mut(&mut displays);
        displays
    }

    pub fn display_count(&self) -> usize {
        self.areas().iter().map(|area| area.displays.len()).sum()
    }

    pub fn display(&self, id: &str) -> Option<&I> {
        self.displays().find(|display| display.id() == id)
    }

    pub fn display_mut(&mut self, id: &str) -> Option<&mut I> {
        self.displays_mut()
            .into_iter()
            .find(|display| display.id() == id)
    }

    /// The area holding the display `id`.
    pub fn area_of(&self, display_id: &str) -> Option<&Area<I>> {
        self.areas().into_iter().find(|area| {
            area.displays
                .iter()
                .any(|display| display.id() == display_id)
        })
    }

    /// A focus stamp later than every one in the tree, so the most recently
    /// focused display is unambiguous even within one millisecond.
    pub fn next_stamp(&self, now_unix_ms: u64) -> u64 {
        let latest = self
            .displays()
            .map(|display| display.focus_stamp())
            .max()
            .unwrap_or(0);
        now_unix_ms.max(latest + 1)
    }

    pub(crate) fn mint(&mut self, prefix: char) -> String {
        let id = format!("{prefix}{}", self.next_id);
        self.next_id += 1;
        id
    }

    /// Adds `display` to the end of `area_id` as its active display, and
    /// makes that area the one in use.
    pub fn insert(&mut self, area_id: &str, mut display: I, stamp: u64) -> Result<(), LayoutError> {
        self.check_room()?;
        let area = self
            .root
            .area_mut(area_id)
            .ok_or_else(|| LayoutError::UnknownArea(area_id.to_owned()))?;
        display.set_focus_stamp(stamp);
        area.active = Some(display.id().to_owned());
        area.displays.push(display);
        self.active_area = area_id.to_owned();
        Ok(())
    }

    /// Adds `display` to the end of `area_id` without moving the focus: the
    /// area shows it only when it had nothing to show.
    pub fn append(&mut self, area_id: &str, display: I) -> Result<(), LayoutError> {
        self.check_room()?;
        let area = self
            .root
            .area_mut(area_id)
            .ok_or_else(|| LayoutError::UnknownArea(area_id.to_owned()))?;
        if area.active.is_none() {
            area.active = Some(display.id().to_owned());
        }
        area.displays.push(display);
        Ok(())
    }

    /// Takes a display out. Its area shows its most recently focused
    /// remaining display; an area left empty collapses and its sibling takes
    /// the space, except the last area, which stays empty (B10).
    pub fn remove(&mut self, display_id: &str) -> Option<I> {
        let area_id = self.area_of(display_id)?.id.clone();
        let display = self.take(&area_id, display_id)?;
        self.collapse_if_empty(&area_id);
        Some(display)
    }

    fn take(&mut self, area_id: &str, display_id: &str) -> Option<I> {
        let area = self.root.area_mut(area_id)?;
        let index = area
            .displays
            .iter()
            .position(|display| display.id() == display_id)?;
        let display = area.displays.remove(index);
        if area.active.as_deref() == Some(display_id) {
            area.active = successor(&area.displays, index);
        }
        Some(display)
    }

    fn collapse_if_empty(&mut self, area_id: &str) {
        if !self
            .area(area_id)
            .is_some_and(|area| area.displays.is_empty())
        {
            return;
        }
        if let Some(nearest) = self.root.remove_area(area_id)
            && self.active_area == area_id
        {
            self.active_area = nearest;
        }
    }

    /// Makes a display its area's active display and that area the one in
    /// use. Returns whether that changed anything; focusing the display
    /// already in use is a no-op and stamps nothing.
    pub fn focus(&mut self, display_id: &str, stamp: u64) -> Result<bool, LayoutError> {
        let area_id = self
            .area_of(display_id)
            .ok_or_else(|| LayoutError::UnknownDisplay(display_id.to_owned()))?
            .id
            .clone();
        let area = self.root.area_mut(&area_id).expect("found above");
        if self.active_area == area_id && area.active.as_deref() == Some(display_id) {
            return Ok(false);
        }
        area.active = Some(display_id.to_owned());
        if let Some(display) = area
            .displays
            .iter_mut()
            .find(|display| display.id() == display_id)
        {
            display.set_focus_stamp(stamp);
        }
        self.active_area = area_id;
        Ok(true)
    }

    /// Makes a display its area's active display and leaves the area in use
    /// where it is: what a bookmark restore does, so the keyboard's area never
    /// moves. Returns whether that changed anything.
    pub fn show(&mut self, display_id: &str, stamp: u64) -> Result<bool, LayoutError> {
        let area_id = self
            .area_of(display_id)
            .ok_or_else(|| LayoutError::UnknownDisplay(display_id.to_owned()))?
            .id
            .clone();
        let area = self.root.area_mut(&area_id).expect("found above");
        if area.active.as_deref() == Some(display_id) {
            return Ok(false);
        }
        area.active = Some(display_id.to_owned());
        if let Some(display) = area
            .displays
            .iter_mut()
            .find(|display| display.id() == display_id)
        {
            display.set_focus_stamp(stamp);
        }
        Ok(true)
    }

    /// Makes an area the one in use; its active display counts as focused.
    pub fn focus_area(&mut self, area_id: &str, stamp: u64) -> Result<bool, LayoutError> {
        if self.area(area_id).is_none() {
            return Err(LayoutError::UnknownArea(area_id.to_owned()));
        }
        if self.active_area == area_id {
            return Ok(false);
        }
        self.active_area = area_id.to_owned();
        let area = self.root.area_mut(area_id).expect("found above");
        if let Some(active) = area.active.clone()
            && let Some(display) = area
                .displays
                .iter_mut()
                .find(|display| display.id() == active)
        {
            display.set_focus_stamp(stamp);
        }
        Ok(true)
    }

    /// Puts a display at `index` of `area_id` (clamped), reordering within
    /// its area or moving it to another; it becomes that area's active
    /// display, the area becomes the one in use, and an area it leaves empty
    /// collapses. A display that changes place is kept open (a drag keeps a
    /// preview, D-02). The index is the display's place after the move, so
    /// the same move twice changes nothing the second time. A display moved
    /// into an area that already shows its document replaces that twin,
    /// which goes: only Open to the side makes a second view of a document,
    /// and never within one area (D-03, B6). The index still counts the
    /// area without the twin, so the sender names the final place.
    pub fn move_display(
        &mut self,
        display_id: &str,
        area_id: &str,
        index: usize,
        stamp: u64,
    ) -> Result<bool, LayoutError> {
        let source = self
            .area_of(display_id)
            .ok_or_else(|| LayoutError::UnknownDisplay(display_id.to_owned()))?;
        let source_id = source.id.clone();
        let from = source
            .displays
            .iter()
            .position(|display| display.id() == display_id)
            .expect("the area holds it");
        let target = self
            .area(area_id)
            .ok_or_else(|| LayoutError::UnknownArea(area_id.to_owned()))?;
        let to = if source_id == area_id {
            index.min(target.displays.len() - 1)
        } else {
            index.min(target.displays.len())
        };
        let placed = source_id == area_id && from == to;
        if placed && self.active_area == area_id && target.active.as_deref() == Some(display_id) {
            return Ok(false);
        }
        let mut display = self.take(&source_id, display_id).expect("found above");
        if !placed {
            display.keep_open();
        }
        display.set_focus_stamp(stamp);
        let area = self.root.area_mut(area_id).expect("found above");
        if source_id != area_id
            && let Some(twin) = area
                .displays
                .iter()
                .position(|other| other.same_content(&display))
        {
            area.displays.remove(twin);
        }
        area.active = Some(display_id.to_owned());
        area.displays.insert(to.min(area.displays.len()), display);
        self.active_area = area_id.to_owned();
        if source_id != area_id {
            self.collapse_if_empty(&source_id);
        }
        Ok(true)
    }

    /// Moves a display into a new area at `edge` of `area_id`, which gives
    /// the new area half its space (B7). Refused when the result would pass
    /// the area or depth cap, or when the display is the only one of the
    /// area it would split. Returns the new area's id.
    pub fn split(
        &mut self,
        display_id: &str,
        area_id: &str,
        edge: Edge,
        stamp: u64,
    ) -> Result<String, LayoutError> {
        let source = self
            .area_of(display_id)
            .ok_or_else(|| LayoutError::UnknownDisplay(display_id.to_owned()))?;
        let source_id = source.id.clone();
        let alone = source.displays.len() == 1;
        if self.area(area_id).is_none() {
            return Err(LayoutError::UnknownArea(area_id.to_owned()));
        }
        if source_id == area_id && alone {
            return Err(LayoutError::NothingToSplit);
        }
        let mut next = self.clone();
        let mut display = next.take(&source_id, display_id).expect("found above");
        display.keep_open();
        let fresh = next.put_beside(area_id, edge, display, stamp);
        next.collapse_if_empty(&source_id);
        next.check_caps()?;
        *self = next;
        Ok(fresh)
    }

    /// Puts a display that is in no area yet into a new area at `edge` of
    /// `area_id` (Open to the side, B4). Returns the new area's id.
    pub fn split_new(
        &mut self,
        area_id: &str,
        edge: Edge,
        display: I,
        stamp: u64,
    ) -> Result<String, LayoutError> {
        if self.area(area_id).is_none() {
            return Err(LayoutError::UnknownArea(area_id.to_owned()));
        }
        self.check_room()?;
        let mut next = self.clone();
        let fresh = next.put_beside(area_id, edge, display, stamp);
        next.check_caps()?;
        *self = next;
        Ok(fresh)
    }

    /// Whether a new area at `edge` of `area_id` would stay inside the caps.
    pub fn can_split(&self, area_id: &str, edge: Edge) -> Result<(), LayoutError> {
        if self.area(area_id).is_none() {
            return Err(LayoutError::UnknownArea(area_id.to_owned()));
        }
        self.check_room()?;
        let mut probe = self.clone();
        let fresh = probe.mint('a');
        let split = probe.mint('s');
        probe
            .root
            .wrap_area(area_id, edge, &mut Some(Area::empty(fresh)), &split);
        probe.check_caps()
    }

    fn put_beside(&mut self, area_id: &str, edge: Edge, mut display: I, stamp: u64) -> String {
        let fresh = self.mint('a');
        let split_id = self.mint('s');
        display.set_focus_stamp(stamp);
        let mut area = Some(Area {
            id: fresh.clone(),
            active: Some(display.id().to_owned()),
            displays: vec![display],
        });
        let wrapped = self.root.wrap_area(area_id, edge, &mut area, &split_id);
        debug_assert!(wrapped, "the caller checked the area exists");
        self.active_area = fresh.clone();
        fresh
    }

    /// Every operation that adds a display refuses the one past the cap, so
    /// no path can take a Workspace there.
    fn check_room(&self) -> Result<(), LayoutError> {
        if self.display_count() >= I::LIMITS.items {
            return Err(LayoutError::DisplayLimit);
        }
        Ok(())
    }

    fn check_caps(&self) -> Result<(), LayoutError> {
        if self.area_count() > I::LIMITS.areas {
            return Err(LayoutError::AreaLimit);
        }
        if self.depth() > I::LIMITS.depth {
            return Err(LayoutError::DepthLimit);
        }
        Ok(())
    }

    /// Sets a split's first-child share, clamped. A target state: the same
    /// ratio twice changes nothing the second time.
    pub fn resize(&mut self, split_id: &str, ratio: f32) -> Result<bool, LayoutError> {
        let ratio = clamp_ratio(ratio).ok_or(LayoutError::InvalidRatio)?;
        let split = self
            .root
            .split_mut(split_id)
            .ok_or_else(|| LayoutError::UnknownSplit(split_id.to_owned()))?;
        if split.ratio == ratio {
            return Ok(false);
        }
        split.ratio = ratio;
        Ok(true)
    }

    /// The area next to `area_id` toward `edge` in the tree: the nearest one
    /// across the closest enclosing split along that axis.
    pub fn neighbour(&self, area_id: &str, edge: Edge) -> Option<String> {
        fn path_to<'a, I: AreaItem>(
            node: &'a Node<I>,
            area_id: &str,
            path: &mut Vec<(&'a Split<I>, bool)>,
        ) -> bool {
            match node {
                Node::Area(area) => area.id == area_id,
                Node::Split(split) => {
                    path.push((split, true));
                    if path_to(&split.first, area_id, path) {
                        return true;
                    }
                    path.pop();
                    path.push((split, false));
                    if path_to(&split.second, area_id, path) {
                        return true;
                    }
                    path.pop();
                    false
                }
            }
        }
        let mut path = Vec::new();
        if !path_to(&self.root, area_id, &mut path) {
            return None;
        }
        path.iter().rev().find_map(|(split, in_first)| {
            if split.axis != edge.axis() {
                return None;
            }
            match (edge.after(), in_first) {
                (true, true) => Some(split.second.first_area().id.clone()),
                (false, false) => Some(split.first.last_area().id.clone()),
                _ => None,
            }
        })
    }

    /// Gives every id a new number in tree order, keeping the display each
    /// area shows and the area in use.
    fn renumber(&mut self) {
        let in_use = self
            .areas()
            .iter()
            .position(|area| area.id == self.active_area);
        let mut next = 1u64;
        let mut mint = |prefix: char| {
            let id = format!("{prefix}{next}");
            next += 1;
            id
        };
        self.root.visit_mut(&mut |node| match node {
            Node::Area(area) => {
                let shown = area.active.as_ref().and_then(|active| {
                    area.displays
                        .iter()
                        .position(|display| display.id() == active)
                });
                area.id = mint('a');
                for display in &mut area.displays {
                    if I::MINTED_IDENTITY {
                        *display.id_mut() = mint('d');
                    }
                }
                area.active = shown.map(|index| area.displays[index].id().to_owned());
            }
            Node::Split(split) => split.id = mint('s'),
        });
        self.next_id = next;
        if let Some(index) = in_use {
            self.active_area = self.areas()[index].id.clone();
        }
    }

    /// Replaces focus stamps near the end of their numbers by their order,
    /// so the display focused last stays last and the next stamp has room.
    /// Returns whether it had to.
    fn renumber_stamps(&mut self) -> bool {
        if self
            .displays()
            .all(|display| display.focus_stamp() < NUMBER_LIMIT)
        {
            return false;
        }
        let mut stamps: Vec<u64> = self
            .displays()
            .map(|display| display.focus_stamp())
            .filter(|stamp| *stamp > 0)
            .collect();
        stamps.sort_unstable();
        stamps.dedup();
        for display in self.displays_mut() {
            if let Ok(rank) = stamps.binary_search(&display.focus_stamp()) {
                display.set_focus_stamp(rank as u64 + 1);
            }
        }
        true
    }

    /// Makes a stored tree hold the invariants every operation keeps: unique
    /// ids and a `next_id` past them with room to mint, ratios in bounds, the
    /// display, depth and area caps, no empty area but the root, at most one
    /// preview per area, active names that exist, focus stamps with room.
    /// Returns what it had to change as counts, for the diagnostic log, in
    /// one pass over the tree per rule; a tree this build wrote returns
    /// nothing.
    pub fn repair(&mut self) -> Vec<String> {
        let mut notes = Vec::new();
        let mut highest = 0u64;
        self.root.visit_mut(&mut |node| {
            let mut note = |id: &str| {
                if let Some(number) = id.get(1..).and_then(|digits| digits.parse::<u64>().ok()) {
                    highest = highest.max(number);
                }
            };
            match node {
                Node::Area(area) => {
                    note(&area.id);
                    area.displays.iter().for_each(|display| note(display.id()));
                }
                Node::Split(split) => note(&split.id),
            }
        });
        // Only a crafted file holds numbers this high; minting past them
        // would overflow, so the tree takes new ones.
        if highest >= NUMBER_LIMIT || self.next_id >= NUMBER_LIMIT {
            self.renumber();
            notes.push("ids near the end of their numbers were renumbered".to_owned());
        } else if self.next_id <= highest {
            self.next_id = highest + 1;
        }
        if self.renumber_stamps() {
            notes.push("focus stamps near the end of their numbers were renumbered".to_owned());
        }

        let mut seen = HashSet::new();
        let mut renamed = 0usize;
        let mut ratios = 0usize;
        let SplitTree { root, next_id, .. } = self;
        let mut claim = |id: &mut String, prefix: char| {
            if !seen.insert(id.clone()) {
                *id = format!("{prefix}{next_id}");
                *next_id += 1;
                seen.insert(id.clone());
                renamed += 1;
            }
        };
        root.visit_mut(&mut |node| match node {
            Node::Area(area) => {
                claim(&mut area.id, 'a');
                for display in &mut area.displays {
                    if I::MINTED_IDENTITY {
                        claim(display.id_mut(), 'd');
                    }
                }
            }
            Node::Split(split) => {
                claim(&mut split.id, 's');
                let bounded = clamp_ratio(split.ratio).unwrap_or(DEFAULT_SPLIT_RATIO);
                if bounded != split.ratio {
                    split.ratio = bounded;
                    ratios += 1;
                }
            }
        });
        if renamed > 0 {
            notes.push(format!("{renamed} repeated ids were renamed"));
        }
        if ratios > 0 {
            notes.push(format!("{ratios} split ratios were brought into bounds"));
        }
        let mut kept = 0usize;
        let mut dropped = 0usize;
        self.root.visit_mut(&mut |node| {
            if let Node::Area(area) = node {
                let room = I::LIMITS.items.saturating_sub(kept);
                if area.displays.len() > room {
                    dropped += area.displays.len() - room;
                    area.displays.truncate(room);
                }
                kept += area.displays.len();
            }
        });
        if dropped > 0 {
            notes.push(format!(
                "{dropped} views past the cap of {} were dropped",
                I::LIMITS.items
            ));
        }

        // One pass however many areas a file holds; the last area stays,
        // empty, when every one is.
        let survivor = self.root.first_area().id.clone();
        let root = std::mem::replace(&mut self.root, Node::Area(Area::empty(String::new())));
        let mut emptied = 0usize;
        self.root = without_empty_areas(root, &mut emptied).unwrap_or_else(|| {
            emptied -= 1;
            Node::Area(Area::empty(survivor))
        });
        if emptied > 0 {
            notes.push(format!("{emptied} empty areas were removed"));
        }

        self.root.flatten_below(0, &mut notes);
        while self.area_count() > I::LIMITS.areas {
            let Some(merged) = self.root.merge_last_pair() else {
                break;
            };
            notes.push(format!(
                "areas past the cap of {} were merged into {merged}",
                I::LIMITS.areas
            ));
        }

        let mut actives = 0usize;
        self.root.visit_mut(&mut |node| {
            let Node::Area(area) = node else {
                return;
            };
            let valid = area
                .active
                .as_ref()
                .is_some_and(|active| area.displays.iter().any(|display| display.id() == active));
            if !valid {
                let fixed = successor(&area.displays, 0);
                if area.active != fixed {
                    area.active = fixed;
                    actives += 1;
                }
            }
            I::repair_area(&mut area.displays, area.active.as_deref(), &mut notes);
        });
        if actives > 0 {
            notes.push(format!("{actives} areas named a missing active view"));
        }
        if self.area(&self.active_area).is_none() {
            notes.push(format!(
                "the area in use, {}, was missing",
                self.active_area
            ));
            self.active_area = self.root.first_area().id.clone();
        }
        notes
    }
}

/// Ids and focus stamps stay below this, so minting the next one can never
/// overflow; only a crafted file comes near it, and loading renumbers it.
const NUMBER_LIMIT: u64 = u64::MAX / 2;

/// `node` without its empty areas: a split that loses one side becomes its
/// other side, and `None` when nothing is left. Counts what it dropped.
fn without_empty_areas<I: AreaItem>(node: Node<I>, dropped: &mut usize) -> Option<Node<I>> {
    match node {
        Node::Area(area) if area.displays.is_empty() => {
            *dropped += 1;
            None
        }
        Node::Area(area) => Some(Node::Area(area)),
        Node::Split(split) => {
            let Split {
                id,
                axis,
                ratio,
                first,
                second,
            } = split;
            match (
                without_empty_areas(*first, dropped),
                without_empty_areas(*second, dropped),
            ) {
                (Some(first), Some(second)) => Some(Node::Split(Split {
                    id,
                    axis,
                    ratio,
                    first: Box::new(first),
                    second: Box::new(second),
                })),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => None,
            }
        }
    }
}

/// The display an area shows once `index` left it: its most recently
/// focused remaining display, else the one that took the index, else the
/// last.
pub(crate) fn successor<I: AreaItem>(displays: &[I], index: usize) -> Option<String> {
    displays
        .iter()
        .filter(|display| display.focus_stamp() > 0)
        .max_by_key(|display| display.focus_stamp())
        .or_else(|| displays.get(index).or(displays.last()))
        .map(|display| display.id().to_owned())
}
