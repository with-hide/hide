//! A tab's pane tree and the rules that change it, the one place both the
//! core's own engine and the drawing ahead of Herdr read them (PRD
//! engine-bundle-3-core-owns D-4): a split puts the new pane second at ratio
//! 0.5, a close puts the closed pane's sibling in its parent's place, and a
//! tree is laid out into the rects a session carries with the arithmetic the
//! projection reads them back with (`live::split_areas`).

use crate::model::{PaneLayoutDirection, PaneLayoutNodeSnapshot};
use crate::sidebar::{SessionLayoutPanePayload, SessionLayoutRect, SessionLayoutSplitPayload};

pub(crate) fn lay_out(
    node: &PaneLayoutNodeSnapshot,
    area: SessionLayoutRect,
    panes: &mut Vec<SessionLayoutPanePayload>,
    splits: &mut Vec<SessionLayoutSplitPayload>,
) -> Result<(), String> {
    match node {
        PaneLayoutNodeSnapshot::Pane { pane_id } => panes.push(SessionLayoutPanePayload {
            pane_id: pane_id.clone(),
            rect: area,
        }),
        PaneLayoutNodeSnapshot::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let (first_area, second_area, _) = crate::live::split_areas(area, *direction, *ratio)?;
            splits.push(SessionLayoutSplitPayload {
                direction: *direction,
                ratio: *ratio,
                rect: area,
            });
            lay_out(first, first_area, panes, splits)?;
            lay_out(second, second_area, panes, splits)?;
        }
    }
    Ok(())
}

pub(crate) fn contains(node: &PaneLayoutNodeSnapshot, pane: &str) -> bool {
    match node {
        PaneLayoutNodeSnapshot::Pane { pane_id } => pane_id == pane,
        PaneLayoutNodeSnapshot::Split { first, second, .. } => {
            contains(first, pane) || contains(second, pane)
        }
    }
}

pub(crate) fn first_leaf(node: &PaneLayoutNodeSnapshot) -> &str {
    match node {
        PaneLayoutNodeSnapshot::Pane { pane_id } => pane_id,
        PaneLayoutNodeSnapshot::Split { first, .. } => first_leaf(first),
    }
}

pub(crate) fn split_leaf(
    node: &mut PaneLayoutNodeSnapshot,
    target: &str,
    direction: PaneLayoutDirection,
    created: &str,
) {
    match node {
        PaneLayoutNodeSnapshot::Pane { pane_id } if pane_id == target => {
            let kept = PaneLayoutNodeSnapshot::Pane {
                pane_id: pane_id.clone(),
            };
            *node = PaneLayoutNodeSnapshot::Split {
                direction,
                ratio: 0.5,
                first: Box::new(kept),
                second: Box::new(PaneLayoutNodeSnapshot::Pane {
                    pane_id: created.to_owned(),
                }),
            };
        }
        PaneLayoutNodeSnapshot::Pane { .. } => {}
        PaneLayoutNodeSnapshot::Split { first, second, .. } => {
            split_leaf(first, target, direction, created);
            split_leaf(second, target, direction, created);
        }
    }
}

/// Removes `pane` and puts its sibling in the parent's place. Returns the
/// sibling's first leaf, the pane Herdr focuses when the closed one had focus.
pub(crate) fn remove_leaf(node: &mut PaneLayoutNodeSnapshot, pane: &str) -> Option<String> {
    let PaneLayoutNodeSnapshot::Split { first, second, .. } = node else {
        return None;
    };
    let survivor = match (first.as_ref(), second.as_ref()) {
        (PaneLayoutNodeSnapshot::Pane { pane_id }, _) if pane_id == pane => {
            Some(second.as_ref().clone())
        }
        (_, PaneLayoutNodeSnapshot::Pane { pane_id }) if pane_id == pane => {
            Some(first.as_ref().clone())
        }
        _ => None,
    };
    if let Some(survivor) = survivor {
        let focus = first_leaf(&survivor).to_owned();
        *node = survivor;
        return Some(focus);
    }
    remove_leaf(first, pane).or_else(|| remove_leaf(second, pane))
}
