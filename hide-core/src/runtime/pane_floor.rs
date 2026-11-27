//! The pane floor (`docs/UI_BEHAVIOR.md`, The pane floor): how many columns
//! each terminal of a tab holds in a layout, from what the window drawing the
//! tab measured, and whether an operation that changes the panes' widths
//! leaves every one of them at the floor or wider.
//!
//! One judgment, made here only. A split right, a fork to the side, `hide pane
//! open --beside`, `hide agent spawn --beside`, a device pane's split and a
//! sideways resize all ask it before they are sent; the terminal menu's Split
//! right and Agent Views' minimum beside the desk read what it says through
//! the snapshot (`PaneFloorSnapshot`). Split down and an upward or downward
//! resize change heights only and are never judged.
//!
//! A terminal's columns are what the web shell's fit gives it: the pane's
//! share of the tab's width, less the inset its terminal stands from either
//! side, in whole cells. The share is read from the core's own layout (the
//! engine's arithmetic, `engine/own/layout.rs`); the window reports only what
//! the core cannot know: the width the tab's panes are laid out in, the inset
//! and the cell widths (`Event::TerminalResize`). A tab no window has measured
//! is judged by nothing: nothing has drawn it, so nothing is drawn under the
//! floor (`docs/INSTALL.md`, What each system supports).

use std::collections::HashMap;

use hide_agent_adapter::composer::PANE_FLOOR_COLUMNS;
use serde::Deserialize;

use super::Runtime;
use crate::engine::own::layout::{resized, set_resized_ratio, split_leaf};
use crate::live::PaneResizeDirection;
use crate::model::{
    DEFAULT_PANE_TEXT_SCALE, PaneFloorRefusal, PaneFloorSnapshot, PaneLayoutDirection,
    PaneLayoutNodeSnapshot, PaneLayoutSnapshot, RemotePaneLayoutFrame, RemotePaneLayoutSnapshot,
};

/// What the window drawing a tab measured, in CSS pixels: the width the tab's
/// panes are laid out in, the inset a terminal stands from each side of its
/// pane, and a terminal cell's width at no per-pane text scale, which is the
/// cell of every pane with no scale of its own, a new one included.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TabMeasure {
    pub(crate) canvas_px: f64,
    pub(crate) inset_px: f64,
    pub(crate) cell_px: f64,
}

impl TabMeasure {
    /// A measure the window could have drawn: every width positive and finite.
    pub(crate) fn new(canvas_px: f64, inset_px: f64, cell_px: f64) -> Option<Self> {
        let usable = |value: f64| value.is_finite() && value >= 0.0;
        (usable(canvas_px)
            && usable(inset_px)
            && usable(cell_px)
            && canvas_px > 0.0
            && cell_px > 0.0)
            .then_some(Self {
                canvas_px,
                inset_px,
                cell_px,
            })
    }

    /// The columns a terminal in `share` of the tab's width holds at `cell_px`.
    pub(crate) fn columns(&self, share: f64, cell_px: f64) -> u16 {
        let inner = share * self.canvas_px - 2.0 * self.inset_px;
        (inner / cell_px).floor().clamp(0.0, f64::from(u16::MAX)) as u16
    }

    /// The least pane width, in pixels, whose terminal holds the floor at `cell_px`.
    pub(crate) fn floor_px(&self, cell_px: f64) -> f64 {
        f64::from(PANE_FLOOR_COLUMNS) * cell_px + 2.0 * self.inset_px
    }
}

/// Each pane of `root` with its share of the tab's width, first to last: a
/// split right gives each side its ratio of the width, and a split down gives
/// both the whole of it.
pub(crate) fn tree_shares(root: &PaneLayoutNodeSnapshot) -> Vec<(String, f64)> {
    fn walk(node: &PaneLayoutNodeSnapshot, share: f64, out: &mut Vec<(String, f64)>) {
        match node {
            PaneLayoutNodeSnapshot::Pane { pane_id } => out.push((pane_id.clone(), share)),
            PaneLayoutNodeSnapshot::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let ratio = f64::from(*ratio);
                let (one, two) = match direction {
                    PaneLayoutDirection::Right => (share * ratio, share * (1.0 - ratio)),
                    PaneLayoutDirection::Down => (share, share),
                };
                walk(first, one, out);
                walk(second, two, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, 1.0, &mut out);
    out
}

/// Each pane of a device's tab with its share of the tab's width, read off
/// the rectangles that device's engine reports.
pub(crate) fn frame_shares(frames: &[RemotePaneLayoutFrame]) -> Vec<(String, f64)> {
    let whole = frames
        .iter()
        .map(|frame| frame.x + frame.width)
        .fold(0.0_f64, f64::max);
    if whole <= 0.0 {
        return Vec::new();
    }
    frames
        .iter()
        .map(|frame| (frame.pane_id.clone(), frame.width / whole))
        .collect()
}

/// The pane holding the fewest columns among `shares`, with them.
pub(crate) fn narrowest(
    shares: &[(String, f64)],
    measure: &TabMeasure,
    cell_of: impl Fn(&str) -> f64,
) -> Option<(String, u16)> {
    shares
        .iter()
        .map(|(pane, share)| (pane.clone(), measure.columns(*share, cell_of(pane))))
        .min_by_key(|(_, cols)| *cols)
}

/// The least width, in pixels, the tab's panes need for each to keep the
/// floor: Agent Views' minimum beside a standing desk. A zoomed tab draws its
/// focused pane alone, so it needs that pane's floor and no more.
pub(crate) fn min_width_px(
    layout: &PaneLayoutSnapshot,
    measure: &TabMeasure,
    cell_of: impl Fn(&str) -> f64,
) -> f64 {
    if layout.zoomed {
        return measure.floor_px(cell_of(&layout.focused_pane_id));
    }
    shares_min_width(&tree_shares(&layout.root), measure, cell_of)
}

/// The same for a device's tab, from its rectangles.
pub(crate) fn frames_min_width_px(
    frames: &[RemotePaneLayoutFrame],
    zoomed: Option<&str>,
    measure: &TabMeasure,
    cell_of: impl Fn(&str) -> f64,
) -> f64 {
    if let Some(focused) = zoomed {
        return measure.floor_px(cell_of(focused));
    }
    shares_min_width(&frame_shares(frames), measure, cell_of)
}

fn shares_min_width(
    shares: &[(String, f64)],
    measure: &TabMeasure,
    cell_of: impl Fn(&str) -> f64,
) -> f64 {
    shares
        .iter()
        .filter(|(_, share)| *share > 0.0)
        .map(|(pane, share)| measure.floor_px(cell_of(pane)) / share)
        .fold(0.0_f64, f64::max)
}

/// Why a width-changing operation is refused: the pane it would leave under
/// the floor and the columns it would hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TooNarrow {
    pub(crate) pane_id: String,
    pub(crate) columns: u16,
}

/// The id the pane a split makes is judged under before it exists; it takes
/// the tab's cell, since a new pane has no text scale of its own.
const NEW_PANE: &str = "\u{0}pane-floor-new";

/// Whether splitting `target` right in `layout` leaves every pane of the tab
/// at the floor. The engine shows the new pane, so the split is judged on the
/// tab with its zoom undone, as the engine splits it.
pub(crate) fn split_right(
    layout: &PaneLayoutSnapshot,
    target: &str,
    measure: &TabMeasure,
    cell_of: impl Fn(&str) -> f64,
) -> Result<(), TooNarrow> {
    let mut root = layout.root.clone();
    split_leaf(&mut root, target, PaneLayoutDirection::Right, NEW_PANE);
    let cell = |pane: &str| {
        if pane == NEW_PANE {
            measure.cell_px
        } else {
            cell_of(pane)
        }
    };
    match narrowest(&tree_shares(&root), measure, cell) {
        Some((pane_id, columns)) if columns < PANE_FLOOR_COLUMNS => Err(TooNarrow {
            pane_id: if pane_id == NEW_PANE {
                target.to_owned()
            } else {
                pane_id
            },
            columns,
        }),
        _ => Ok(()),
    }
}

/// Whether splitting the device pane `target` right leaves both halves at
/// the floor; the device's engine halves the pane's rectangle.
pub(crate) fn split_right_frames(
    frames: &[RemotePaneLayoutFrame],
    target: &str,
    measure: &TabMeasure,
    cell_of: impl Fn(&str) -> f64,
) -> Result<(), TooNarrow> {
    let Some((_, share)) = frame_shares(frames)
        .into_iter()
        .find(|(pane, _)| pane == target)
    else {
        return Ok(());
    };
    let half = share / 2.0;
    let columns = measure
        .columns(half, cell_of(target))
        .min(measure.columns(half, measure.cell_px));
    if columns < PANE_FLOOR_COLUMNS {
        return Err(TooNarrow {
            pane_id: target.to_owned(),
            columns,
        });
    }
    Ok(())
}

/// The part of a sideways resize of `pane` by `amount` toward `direction`
/// that keeps every pane at the floor, or at the width the narrowest already
/// has when the window left it under the floor: the divider stops at the
/// floor instead of the resize being refused. An upward or downward resize
/// changes heights and is returned whole.
pub(crate) fn resize_amount(
    layout: &PaneLayoutSnapshot,
    pane: &str,
    direction: PaneResizeDirection,
    amount: f32,
    measure: &TabMeasure,
    cell_of: impl Fn(&str) -> f64,
) -> f32 {
    if matches!(
        direction,
        PaneResizeDirection::Up | PaneResizeDirection::Down
    ) {
        return amount;
    }
    let narrowest_after = |amount: f32| {
        let mut root = layout.root.clone();
        if let Some(ratio) = resized(&root, pane, direction, amount) {
            set_resized_ratio(&mut root, pane, direction, ratio);
        }
        narrowest(&tree_shares(&root), measure, &cell_of).map_or(u16::MAX, |(_, cols)| cols)
    };
    let keep = narrowest_after(0.0).min(PANE_FLOOR_COLUMNS);
    if narrowest_after(amount) >= keep {
        return amount;
    }
    // The pane that shrinks only shrinks as the amount grows, so the amounts
    // that keep the floor are the ones below a bound, found by halving.
    let (mut kept, mut lost) = (0.0_f32, amount);
    for _ in 0..24 {
        let middle = (kept + lost) / 2.0;
        if narrowest_after(middle) >= keep {
            kept = middle;
        } else {
            lost = middle;
        }
    }
    kept
}

/// What a window measured for one pane along with its grid
/// (`terminal_resize`'s `floor`), in CSS pixels: the pane's own cell, and the
/// tab's canvas, inset and cell at no per-pane scale. It reaches the core
/// only from the screen whose grid the pane runs at (`hided::pane_sizes`),
/// so the latest one is the window's that draws it.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FloorReport {
    pub(super) cell_px: f64,
    pub(super) canvas_px: f64,
    pub(super) inset_px: f64,
    pub(super) base_cell_px: f64,
}

/// Each pane's latest report, numbered in the order they came, so a tab's
/// canvas is the one its panes reported last.
#[derive(Debug, Default)]
pub(super) struct FloorReports {
    panes: HashMap<String, (u64, FloorReport)>,
    next: u64,
}

impl FloorReports {
    /// Notes `pane`'s report; a grid without one, from a page that does not
    /// measure, takes the pane's away. Whether anything changed.
    pub(super) fn note(&mut self, pane: &str, report: Option<FloorReport>) -> bool {
        match report {
            Some(report) => {
                if self
                    .panes
                    .get(pane)
                    .is_some_and(|(_, held)| *held == report)
                {
                    return false;
                }
                self.next += 1;
                self.panes.insert(pane.to_owned(), (self.next, report));
                true
            }
            None => self.panes.remove(pane).is_some(),
        }
    }

    /// Kept for the panes that still exist, as every pane-keyed terminal
    /// fact is (`Runtime::retain_terminal_pane_state`).
    pub(super) fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        self.panes.retain(|pane, _| keep(pane));
    }

    /// A window attached anew: what an earlier one measured is not its.
    pub(super) fn clear(&mut self) {
        self.panes.clear();
    }

    #[cfg(test)]
    pub(super) fn holds(&self, pane: &str) -> bool {
        self.panes.contains_key(pane)
    }
}

/// A tab's place under the pane floor.
enum FloorTab {
    /// No window draws the tab, so nothing is drawn under the floor and
    /// nothing is judged (`docs/INSTALL.md`, What each system supports).
    Undrawn,
    /// A window draws it but has not said its size yet, or not the cell of
    /// a pane with a text scale of its own: a width-changing operation waits
    /// for it, refused as `unmeasured`.
    Unmeasured,
    Measured {
        measure: TabMeasure,
        cells: HashMap<String, f64>,
    },
}

impl FloorTab {
    fn cell_of(&self) -> impl Fn(&str) -> f64 + '_ {
        move |pane| match self {
            FloorTab::Measured { measure, cells } => {
                cells.get(pane).copied().unwrap_or(measure.cell_px)
            }
            _ => 0.0,
        }
    }
}

/// A tab as its engine lays it out: the core's own split tree, or the
/// rectangles a device's engine reports.
enum FloorShape<'a> {
    Tree(&'a PaneLayoutSnapshot),
    Frames(&'a RemotePaneLayoutSnapshot),
}

impl Runtime {
    /// The device tab `tab` as its device reports it.
    fn device_floor_layout(&self, tab: &str) -> Option<&RemotePaneLayoutSnapshot> {
        self.snapshot
            .status
            .remote
            .iter()
            .filter_map(|remote| remote.session.as_ref())
            .flat_map(|session| &session.pane_layouts)
            .find(|layout| layout.tab_id == tab)
    }

    /// The device tab holding `pane`, a device's pane as the core names it.
    pub(super) fn device_floor_layout_of(&self, pane: &str) -> Option<&RemotePaneLayoutSnapshot> {
        self.snapshot
            .status
            .remote
            .iter()
            .filter_map(|remote| remote.session.as_ref())
            .flat_map(|session| &session.pane_layouts)
            .find(|layout| layout.frames.iter().any(|frame| frame.pane_id == pane))
    }

    /// Where `tab`, holding `panes`, stands under the floor.
    fn floor_tab(&self, tab: &str, panes: &[&str]) -> FloorTab {
        if !self.ui_attached || !self.shown_agent_tabs().iter().any(|shown| shown == tab) {
            return FloorTab::Undrawn;
        }
        let Some(latest) = panes
            .iter()
            .filter_map(|pane| self.floor_reports.panes.get(*pane))
            .max_by_key(|(order, _)| *order)
            .map(|(_, report)| *report)
        else {
            return FloorTab::Unmeasured;
        };
        let Some(measure) = TabMeasure::new(latest.canvas_px, latest.inset_px, latest.base_cell_px)
        else {
            return FloorTab::Unmeasured;
        };
        let mut cells = HashMap::new();
        for pane in panes {
            match self.floor_reports.panes.get(*pane) {
                Some((_, report)) if report.cell_px.is_finite() && report.cell_px > 0.0 => {
                    cells.insert((*pane).to_owned(), report.cell_px);
                }
                // A pane with no report of its own draws at the tab's cell
                // only while it has no text scale of its own.
                _ if self
                    .snapshot
                    .ui_state
                    .pane_text_scales
                    .get(*pane)
                    .is_none_or(|scale| *scale == DEFAULT_PANE_TEXT_SCALE) => {}
                _ => return FloorTab::Unmeasured,
            }
        }
        FloorTab::Measured { measure, cells }
    }

    /// Whether splitting `target` right in `shape`, the tab `tab`, keeps
    /// every pane at the floor.
    fn floor_split_right(
        &self,
        tab: &str,
        shape: FloorShape<'_>,
        target: &str,
    ) -> Result<(), PaneFloorRefusal> {
        let panes = match &shape {
            FloorShape::Tree(layout) => layout.pane_ids(),
            FloorShape::Frames(layout) => layout
                .frames
                .iter()
                .map(|frame| frame.pane_id.as_str())
                .collect(),
        };
        let place = self.floor_tab(tab, &panes);
        let FloorTab::Measured { measure, .. } = &place else {
            return match place {
                FloorTab::Undrawn => Ok(()),
                _ => Err(PaneFloorRefusal::Unmeasured),
            };
        };
        let judged = match shape {
            FloorShape::Tree(layout) => split_right(layout, target, measure, place.cell_of()),
            FloorShape::Frames(layout) => {
                split_right_frames(&layout.frames, target, measure, place.cell_of())
            }
        };
        judged.map_err(|narrow| {
            crate::diagnostic!(serde_json::json!({
                "component": "pane_floor", "kind": "pane_floor.split_refused",
                "tab_id": tab, "pane_id": target,
                "narrowest_pane_id": narrow.pane_id, "columns": narrow.columns,
            }));
            PaneFloorRefusal::TooNarrow
        })
    }

    /// Whether `pane` splits right now, on its tab as the line will leave it:
    /// the terminal menu's Split right, a fork to the side, `hide pane open
    /// --beside` and `hide agent spawn --beside` ask this before anything is
    /// queued or made, and a device's pane before its device is asked.
    pub(crate) fn split_right_floor(&self, pane: &str) -> Result<(), PaneFloorRefusal> {
        if let Some(layout) = self.device_floor_layout_of(pane) {
            return self.floor_split_right(&layout.tab_id, FloorShape::Frames(layout), pane);
        }
        let Some(tab) = self.pane_operation_scope(pane) else {
            return Ok(());
        };
        let Some(layout) = self.predicted_tab_layout(&tab) else {
            return Ok(());
        };
        self.floor_split_right(&tab, FloorShape::Tree(&layout), pane)
    }

    /// The same, judged when the split's turn comes, on the tab as the
    /// engine confirmed it: nothing ahead of it is in flight then.
    pub(super) fn split_right_floor_at_turn(
        &self,
        tab: &str,
        pane: &str,
    ) -> Result<(), PaneFloorRefusal> {
        match self.confirmed_floor_layout(tab) {
            Some(layout) => self.floor_split_right(tab, FloorShape::Tree(layout), pane),
            None => Ok(()),
        }
    }

    fn confirmed_floor_layout(&self, tab: &str) -> Option<&PaneLayoutSnapshot> {
        self.confirmed_layouts.get(tab).or_else(|| {
            self.snapshot
                .pane_layouts
                .iter()
                .find(|layout| layout.tab_id == tab)
        })
    }

    /// The part of a sideways resize of `pane` in `layout`, the tab `tab`,
    /// that keeps the floor: whole for an undrawn tab and an upward or
    /// downward one, none while the window has not measured the tab.
    pub(super) fn floor_resize_amount(
        &self,
        tab: &str,
        layout: &PaneLayoutSnapshot,
        pane: &str,
        direction: PaneResizeDirection,
        amount: f32,
    ) -> f32 {
        if matches!(
            direction,
            PaneResizeDirection::Up | PaneResizeDirection::Down
        ) {
            return amount;
        }
        let place = self.floor_tab(tab, &layout.pane_ids());
        match &place {
            FloorTab::Undrawn => amount,
            FloorTab::Unmeasured => 0.0,
            FloorTab::Measured { measure, .. } => {
                resize_amount(layout, pane, direction, amount, measure, place.cell_of())
            }
        }
    }

    /// Lays what the floor says of the tabs on screen into the snapshot, and
    /// whether that changed: the panes whose Split right is refused, and each
    /// measured tab's least width. Run before a snapshot is read and after a
    /// window's report, so the screen never draws a stale answer.
    pub(super) fn refresh_pane_floor(&mut self) -> bool {
        let mut next = PaneFloorSnapshot::default();
        if self.ui_attached {
            for tab in self.shown_agent_tabs() {
                if let Some(layout) = self
                    .snapshot
                    .pane_layouts
                    .iter()
                    .find(|layout| layout.tab_id == tab)
                {
                    let panes = layout.pane_ids();
                    let place = self.floor_tab(&tab, &panes);
                    self.project_floor(&mut next, &panes, &place, |target, measure, cell| {
                        split_right(layout, target, measure, cell).is_ok()
                    });
                    if let FloorTab::Measured { measure, .. } = &place {
                        next.min_width_px
                            .insert(tab.clone(), min_width_px(layout, measure, place.cell_of()));
                    }
                } else if let Some(layout) = self.device_floor_layout(&tab) {
                    let panes = layout
                        .frames
                        .iter()
                        .map(|frame| frame.pane_id.as_str())
                        .collect::<Vec<_>>();
                    let place = self.floor_tab(&tab, &panes);
                    self.project_floor(&mut next, &panes, &place, |target, measure, cell| {
                        split_right_frames(&layout.frames, target, measure, cell).is_ok()
                    });
                    if let FloorTab::Measured { measure, .. } = &place {
                        let zoomed = layout.zoomed.then_some(layout.focused_pane_id.as_str());
                        next.min_width_px.insert(
                            tab.clone(),
                            frames_min_width_px(&layout.frames, zoomed, measure, place.cell_of()),
                        );
                    }
                }
            }
        }
        if next == self.snapshot.pane_floor {
            return false;
        }
        self.snapshot.pane_floor = next;
        true
    }

    fn project_floor(
        &self,
        next: &mut PaneFloorSnapshot,
        panes: &[&str],
        place: &FloorTab,
        splits: impl Fn(&str, &TabMeasure, &dyn Fn(&str) -> f64) -> bool,
    ) {
        for pane in panes {
            let refusal = match place {
                FloorTab::Undrawn => None,
                FloorTab::Unmeasured => Some(PaneFloorRefusal::Unmeasured),
                FloorTab::Measured { measure, .. } => {
                    let cell = place.cell_of();
                    (!splits(pane, measure, &cell)).then_some(PaneFloorRefusal::TooNarrow)
                }
            };
            if let Some(refusal) = refusal {
                next.split_right.insert((*pane).to_owned(), refusal);
            }
        }
    }
}

impl PaneFloorRefusal {
    /// The refusal's code where the terminal menu or an event reports it.
    pub(crate) fn event_code(self) -> &'static str {
        match self {
            PaneFloorRefusal::TooNarrow => "pane.too_narrow",
            PaneFloorRefusal::Unmeasured => "pane.unmeasured",
        }
    }

    /// Why, in words the terminal menu and the event's error show.
    pub(crate) fn message(self) -> &'static str {
        match self {
            PaneFloorRefusal::TooNarrow => {
                "Too narrow to split right: a side would be under the narrowest pane an agent's input box reads in"
            }
            PaneFloorRefusal::Unmeasured => "The panes are not drawn yet; try again in a moment",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CELL: f64 = 8.0;
    const INSET: f64 = 8.0;

    fn pane(id: &str) -> PaneLayoutNodeSnapshot {
        PaneLayoutNodeSnapshot::Pane {
            pane_id: id.to_owned(),
        }
    }

    fn right(
        ratio: f32,
        first: PaneLayoutNodeSnapshot,
        second: PaneLayoutNodeSnapshot,
    ) -> PaneLayoutNodeSnapshot {
        PaneLayoutNodeSnapshot::Split {
            direction: PaneLayoutDirection::Right,
            ratio,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    fn down(
        first: PaneLayoutNodeSnapshot,
        second: PaneLayoutNodeSnapshot,
    ) -> PaneLayoutNodeSnapshot {
        PaneLayoutNodeSnapshot::Split {
            direction: PaneLayoutDirection::Down,
            ratio: 0.5,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    fn tab(root: PaneLayoutNodeSnapshot) -> PaneLayoutSnapshot {
        PaneLayoutSnapshot {
            workspace_id: "w1".to_owned(),
            tab_id: "w1:t1".to_owned(),
            focused_pane_id: "p1".to_owned(),
            zoomed: false,
            root,
        }
    }

    /// The canvas a pane whose terminal holds `cols` columns at the tab's cell is drawn in alone.
    fn canvas_for(cols: u16) -> f64 {
        f64::from(cols) * CELL + 2.0 * INSET
    }

    fn measure(canvas_px: f64) -> TabMeasure {
        TabMeasure::new(canvas_px, INSET, CELL).unwrap()
    }

    const BASE: fn(&str) -> f64 = |_| CELL;

    #[test]
    fn a_terminal_holds_its_share_of_the_tab_less_the_inset_in_whole_cells() {
        let at = measure(canvas_for(36));
        assert_eq!(at.columns(1.0, CELL), 36);
        // Each half of a split loses the inset again: 304 px halved is 152, less 16 is 136, 17 cells.
        assert_eq!(at.columns(0.5, CELL), 17);
        assert_eq!(at.floor_px(CELL), 18.0 * CELL + 16.0);
    }

    #[test]
    fn a_split_right_is_judged_on_the_halves_the_fit_will_give() {
        let one = tab(pane("p1"));
        // A pane of 36 columns splits into halves of 17: refused, though 36 is twice the floor.
        assert_eq!(
            split_right(&one, "p1", &measure(canvas_for(36)), BASE),
            Err(TooNarrow {
                pane_id: "p1".to_owned(),
                columns: 17
            })
        );
        // The least pane that splits: two floors and two insets per half, 38 columns.
        let least = 2.0 * (18.0 * CELL + 2.0 * INSET);
        assert_eq!(split_right(&one, "p1", &measure(least), BASE), Ok(()));
        assert!(split_right(&one, "p1", &measure(least - 1.0), BASE).is_err());
    }

    #[test]
    fn the_new_pane_takes_the_tabs_cell_and_a_scaled_parent_keeps_its_own() {
        let one = tab(pane("p1"));
        let at = measure(2.0 * (18.0 * CELL + 2.0 * INSET));
        // The parent at twice the cell holds 9 columns in its half; the new pane at the tab's cell holds 18.
        let scaled = |id: &str| if id == "p1" { 2.0 * CELL } else { CELL };
        assert_eq!(
            split_right(&one, "p1", &at, scaled),
            Err(TooNarrow {
                pane_id: "p1".to_owned(),
                columns: 9
            })
        );
        // A parent drawn smaller than the tab's cell does not lend the new pane its cell.
        let smaller = |id: &str| if id == "p1" { CELL / 2.0 } else { CELL };
        let tight = measure(2.0 * (18.0 * CELL + 2.0 * INSET) - 1.0);
        assert!(split_right(&one, "p1", &tight, smaller).is_err());
    }

    #[test]
    fn a_split_right_counts_every_pane_the_split_narrows_and_ignores_a_split_down() {
        let wide = measure(4.0 * (18.0 * CELL + 2.0 * INSET));
        // Four panes side by side at a quarter each hold the floor; splitting one gives an eighth.
        let quarters = tab(right(
            0.25,
            pane("p1"),
            right(1.0 / 3.0, pane("p2"), right(0.5, pane("p3"), pane("p4"))),
        ));
        assert!(split_right(&quarters, "p2", &wide, BASE).is_err());
        // Panes stacked by a split down each keep the whole width.
        let stacked = tab(down(pane("p1"), down(pane("p2"), pane("p3"))));
        assert_eq!(
            split_right(
                &stacked,
                "p3",
                &measure(2.0 * (18.0 * CELL + 2.0 * INSET)),
                BASE
            ),
            Ok(())
        );
    }

    #[test]
    fn a_zoomed_pane_is_split_on_the_tab_its_zoom_hides() {
        let mut zoomed = tab(right(0.5, pane("p1"), pane("p2")));
        zoomed.zoomed = true;
        // Alone on screen p1 holds twice its columns; the engine splits its half of the tab.
        let at = measure(2.0 * (18.0 * CELL + 2.0 * INSET));
        assert!(split_right(&zoomed, "p1", &at, BASE).is_err());
        // Agent Views needs only the focused pane's floor while the tab is zoomed.
        assert_eq!(min_width_px(&zoomed, &at, BASE), 18.0 * CELL + 2.0 * INSET);
    }

    #[test]
    fn agent_views_needs_every_pane_at_its_share_of_the_floor() {
        let floor = 18.0 * CELL + 2.0 * INSET;
        let at = measure(1000.0);
        assert_eq!(min_width_px(&tab(pane("p1")), &at, BASE), floor);
        let thirds = tab(right(
            1.0 / 3.0,
            pane("p1"),
            right(0.5, pane("p2"), pane("p3")),
        ));
        assert!((min_width_px(&thirds, &at, BASE) - 3.0 * floor).abs() < 1e-3);
        let scaled = |id: &str| if id == "p2" { 2.0 * CELL } else { CELL };
        let pair = tab(right(0.5, pane("p1"), pane("p2")));
        assert_eq!(
            min_width_px(&pair, &at, scaled),
            2.0 * (18.0 * 2.0 * CELL + 2.0 * INSET)
        );
    }

    #[test]
    fn a_device_pane_splits_on_its_rectangle() {
        let frame = |id: &str, x: f64, width: f64| RemotePaneLayoutFrame {
            pane_id: id.to_owned(),
            x,
            y: 0.0,
            width,
            height: 24.0,
        };
        let frames = [frame("d1", 0.0, 60.0), frame("d2", 60.0, 20.0)];
        // d1 holds three quarters of the tab, d2 a quarter: a pixel short of
        // what d2 needs to split, d1 splits and d2 does not.
        let at = measure(4.0 * (2.0 * (18.0 * CELL + 2.0 * INSET)) - 4.0);
        assert_eq!(split_right_frames(&frames, "d1", &at, BASE), Ok(()));
        assert_eq!(
            split_right_frames(&frames, "d2", &at, BASE),
            Err(TooNarrow {
                pane_id: "d2".to_owned(),
                columns: 17
            })
        );
        let floor = 18.0 * CELL + 2.0 * INSET;
        assert!((frames_min_width_px(&frames, None, &at, BASE) - 4.0 * floor).abs() < 1e-3);
        assert_eq!(frames_min_width_px(&frames, Some("d1"), &at, BASE), floor);
    }

    #[test]
    fn a_sideways_resize_stops_where_a_pane_would_go_under_the_floor() {
        let pair = tab(right(0.5, pane("p1"), pane("p2")));
        let at = measure(1000.0);
        // Moving p1's right edge left by 0.4 would leave p1 a tenth of 1000 px: the move is cut short.
        let amount = resize_amount(&pair, "p1", PaneResizeDirection::Left, 0.4, &at, BASE);
        assert!(amount > 0.0 && amount < 0.4);
        let mut root = pair.root.clone();
        let ratio = resized(&root, "p1", PaneResizeDirection::Left, amount).unwrap();
        set_resized_ratio(&mut root, "p1", PaneResizeDirection::Left, ratio);
        let after = narrowest(&tree_shares(&root), &at, BASE).unwrap();
        assert!(after.1 >= PANE_FLOOR_COLUMNS);
        // A short move that keeps the floor goes through whole, and a vertical one is never judged.
        assert_eq!(
            resize_amount(&pair, "p1", PaneResizeDirection::Left, 0.05, &at, BASE),
            0.05
        );
        assert_eq!(
            resize_amount(&pair, "p1", PaneResizeDirection::Up, 0.4, &at, BASE),
            0.4
        );
    }

    #[test]
    fn a_tab_already_under_the_floor_may_widen_its_narrowest_pane_but_not_narrow_it() {
        // At 256 px p1 holds 7 columns and p2 20, both drawn so by a window
        // made narrower after the panes were.
        let pair = tab(right(0.3, pane("p1"), pane("p2")));
        let at = measure(canvas_for(30));
        // Widening p1 to 10 columns leaves p2 17: neither goes under the 7 there are.
        assert_eq!(
            resize_amount(&pair, "p1", PaneResizeDirection::Right, 0.1, &at, BASE),
            0.1
        );
        // Narrowing p1 stops before it loses a column.
        let amount = resize_amount(&pair, "p1", PaneResizeDirection::Left, 0.1, &at, BASE);
        assert!(amount < 0.02, "{amount}");
    }

    #[test]
    fn a_measure_must_be_one_a_window_could_draw() {
        assert!(TabMeasure::new(0.0, INSET, CELL).is_none());
        assert!(TabMeasure::new(800.0, INSET, 0.0).is_none());
        assert!(TabMeasure::new(f64::NAN, INSET, CELL).is_none());
        assert!(TabMeasure::new(800.0, -1.0, CELL).is_none());
    }
}
