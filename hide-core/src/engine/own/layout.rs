// Parts derived from Herdr v0.9.5 (https://github.com/herdrdev/herdr, commit
// 2ecf19a): `swap_leaves` from src/layout.rs (`swap_pane_ids`),
// `valid_ratio` from src/layout.rs (`valid_split_ratio`), and `check_layout`
// with its limits from src/app/api/layouts.rs (`validate_layout_tree`,
// `MAX_LAYOUT_PANES`, `MAX_LAYOUT_DEPTH`).
// Copyright 2025 herdr contributors. Licensed under the Apache License,
// Version 2.0; see NOTICE at the repository root.
// Changed by Hide: the rules work on Hide's own trees
// (`PaneLayoutNodeSnapshot`, `ClosedLayoutNode`) instead of ratatui's
// `Direction` and Herdr's `Node`; a check's message is Hide's; the rest of
// this file (split, close, resize, laying out) is Hide's own.

//! A tab's pane tree and the rules that change it, the one place both the
//! core's own engine and the drawing ahead of the engine read them (PRD
//! engine-bundle-3-core-owns D-4): a split puts the new pane second at its
//! ratio (0.5 unless asked), a close puts the closed pane's sibling in its
//! parent's place, a resize moves the innermost split of its axis that holds
//! the pane, a swap trades two leaves and keeps every split, every ratio is
//! kept between [`RATIO_MIN`] and [`RATIO_MAX`], and a tree is laid out into
//! the rects a session carries with the arithmetic the projection reads them
//! back with (`live::split_areas`). A layout given to be applied holds at
//! most [`MAX_PANES`] panes and [`MAX_DEPTH`] levels.

use crate::live::PaneResizeDirection;
use crate::model::{PaneLayoutDirection, PaneLayoutNodeSnapshot};
use crate::recent_closed::ClosedLayoutNode;
use crate::sidebar::{SessionLayoutPanePayload, SessionLayoutRect, SessionLayoutSplitPayload};

/// Every split is kept between these ratios, as Herdr keeps them.
pub(crate) const RATIO_MIN: f32 = 0.1;
pub(crate) const RATIO_MAX: f32 = 0.9;

/// The most panes and levels a layout given to be applied holds.
pub(crate) const MAX_PANES: usize = 24;
pub(crate) const MAX_DEPTH: usize = 16;

/// `ratio` as a split keeps it: clamped, and half for one that is no number.
pub(crate) fn valid_ratio(ratio: f32) -> f32 {
    if ratio.is_finite() {
        ratio.clamp(RATIO_MIN, RATIO_MAX)
    } else {
        0.5
    }
}

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

/// Splits `target` along `direction` at half, `created` second.
pub(crate) fn split_leaf(
    node: &mut PaneLayoutNodeSnapshot,
    target: &str,
    direction: PaneLayoutDirection,
    created: &str,
) {
    split_leaf_at(node, target, direction, created, 0.5);
}

/// Splits `target` along `direction`, `created` second and `target` taking
/// `ratio` of the room ([`valid_ratio`] of it). A pane moved beside another
/// is put in its place this way, as a new one is.
pub(crate) fn split_leaf_at(
    node: &mut PaneLayoutNodeSnapshot,
    target: &str,
    direction: PaneLayoutDirection,
    created: &str,
    ratio: f32,
) {
    match node {
        PaneLayoutNodeSnapshot::Pane { pane_id } if pane_id == target => {
            let kept = PaneLayoutNodeSnapshot::Pane {
                pane_id: pane_id.clone(),
            };
            *node = PaneLayoutNodeSnapshot::Split {
                direction,
                ratio: valid_ratio(ratio),
                first: Box::new(kept),
                second: Box::new(PaneLayoutNodeSnapshot::Pane {
                    pane_id: created.to_owned(),
                }),
            };
        }
        PaneLayoutNodeSnapshot::Pane { .. } => {}
        PaneLayoutNodeSnapshot::Split { first, second, .. } => {
            split_leaf_at(first, target, direction, created, ratio);
            split_leaf_at(second, target, direction, created, ratio);
        }
    }
}

/// Removes `pane` and puts its sibling in the parent's place. Returns the
/// sibling's first leaf, the pane that takes focus when the closed one had focus (as in Herdr).
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

/// Trades the places of two leaves, every split and ratio kept.
pub(crate) fn swap_leaves(node: &mut PaneLayoutNodeSnapshot, one: &str, other: &str) {
    match node {
        PaneLayoutNodeSnapshot::Pane { pane_id } if pane_id == one => *pane_id = other.to_owned(),
        PaneLayoutNodeSnapshot::Pane { pane_id } if pane_id == other => *pane_id = one.to_owned(),
        PaneLayoutNodeSnapshot::Pane { .. } => {}
        PaneLayoutNodeSnapshot::Split { first, second, .. } => {
            swap_leaves(first, one, other);
            swap_leaves(second, one, other);
        }
    }
}

/// The axis and the ratio moving `pane`'s edge by `amount` toward
/// `direction` sets on the split [`nearest_split_ratio`] reads, clamped.
/// None when no split of that axis holds the pane: a resize that changes
/// nothing.
pub(crate) fn resized(
    root: &PaneLayoutNodeSnapshot,
    pane: &str,
    direction: PaneResizeDirection,
    amount: f32,
) -> Option<(PaneLayoutDirection, f32)> {
    let (axis, sign) = match direction {
        PaneResizeDirection::Left => (PaneLayoutDirection::Right, -1.0),
        PaneResizeDirection::Right => (PaneLayoutDirection::Right, 1.0),
        PaneResizeDirection::Up => (PaneLayoutDirection::Down, -1.0),
        PaneResizeDirection::Down => (PaneLayoutDirection::Down, 1.0),
    };
    let current = nearest_split_ratio(root, pane, axis)?;
    Some((axis, (current + sign * amount).clamp(RATIO_MIN, RATIO_MAX)))
}

/// The ratio of the innermost split of `axis` whose subtree holds `pane`.
pub(crate) fn nearest_split_ratio(
    node: &PaneLayoutNodeSnapshot,
    pane: &str,
    axis: PaneLayoutDirection,
) -> Option<f32> {
    let PaneLayoutNodeSnapshot::Split {
        direction,
        ratio,
        first,
        second,
    } = node
    else {
        return None;
    };
    let inner = if contains(first, pane) {
        nearest_split_ratio(first, pane, axis)
    } else if contains(second, pane) {
        nearest_split_ratio(second, pane, axis)
    } else {
        return None;
    };
    inner.or((*direction == axis).then_some(*ratio))
}

/// Sets the ratio of the split [`nearest_split_ratio`] reads. Returns whether
/// it found one.
pub(crate) fn set_nearest_split_ratio(
    node: &mut PaneLayoutNodeSnapshot,
    pane: &str,
    axis: PaneLayoutDirection,
    value: f32,
) -> bool {
    let PaneLayoutNodeSnapshot::Split {
        direction,
        ratio,
        first,
        second,
    } = node
    else {
        return false;
    };
    let inner = if contains(first, pane) {
        set_nearest_split_ratio(first, pane, axis, value)
    } else if contains(second, pane) {
        set_nearest_split_ratio(second, pane, axis, value)
    } else {
        return false;
    };
    if inner {
        return true;
    }
    if *direction == axis {
        *ratio = value;
        return true;
    }
    false
}

/// Whether a layout given to be applied can be: at most [`MAX_PANES`] panes
/// and [`MAX_DEPTH`] levels, every ratio a number and every command named.
/// The refusal says which limit it passed.
pub(crate) fn check_layout(root: &ClosedLayoutNode) -> Result<(), String> {
    fn walk(node: &ClosedLayoutNode, depth: usize, panes: &mut usize) -> Result<(), String> {
        if depth > MAX_DEPTH {
            return Err(format!(
                "the layout is deeper than {MAX_DEPTH} levels, the most a tab holds"
            ));
        }
        match node {
            ClosedLayoutNode::Pane { command, .. } => {
                *panes += 1;
                if *panes > MAX_PANES {
                    return Err(format!(
                        "the layout holds more than {MAX_PANES} panes, the most a tab holds"
                    ));
                }
                if command.as_ref().is_some_and(Vec::is_empty) {
                    return Err("a pane of the layout names an empty command".to_owned());
                }
                Ok(())
            }
            ClosedLayoutNode::Split {
                ratio,
                first,
                second,
                ..
            } => {
                if !ratio.is_finite() {
                    return Err("a split of the layout has a ratio that is no number".to_owned());
                }
                walk(first, depth + 1, panes)?;
                walk(second, depth + 1, panes)
            }
        }
    }
    walk(root, 1, &mut 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recent_closed::ClosedSplitDirection;

    fn closed_pane() -> ClosedLayoutNode {
        ClosedLayoutNode::Pane {
            pane_id: None,
            label: None,
            cwd: None,
            command: None,
            env: Default::default(),
        }
    }

    fn closed_split(first: ClosedLayoutNode, second: ClosedLayoutNode) -> ClosedLayoutNode {
        ClosedLayoutNode::Split {
            direction: ClosedSplitDirection::Right,
            ratio: 0.5,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    /// A chain of `panes` panes, each split nesting one level deeper.
    fn chain(panes: usize) -> ClosedLayoutNode {
        (1..panes).fold(closed_pane(), |tree, _| closed_split(closed_pane(), tree))
    }

    #[test]
    fn a_layout_past_the_pane_or_depth_limit_is_refused_and_one_at_it_is_not() {
        // A balanced tree of 24 panes is six levels deep; 25 is one too many.
        fn balanced(panes: usize) -> ClosedLayoutNode {
            if panes == 1 {
                return closed_pane();
            }
            closed_split(balanced(panes / 2), balanced(panes - panes / 2))
        }
        assert_eq!(check_layout(&balanced(MAX_PANES)), Ok(()));
        let too_many = check_layout(&balanced(MAX_PANES + 1)).unwrap_err();
        assert!(too_many.contains("more than 24 panes"), "{too_many}");

        // A chain of 16 panes is 16 levels deep; 17 is one level too many.
        assert_eq!(check_layout(&chain(MAX_DEPTH)), Ok(()));
        let too_deep = check_layout(&chain(MAX_DEPTH + 1)).unwrap_err();
        assert!(too_deep.contains("deeper than 16 levels"), "{too_deep}");
    }

    #[test]
    fn a_layout_with_a_ratio_that_is_no_number_or_an_empty_command_is_refused() {
        let ClosedLayoutNode::Split {
            direction,
            first,
            second,
            ..
        } = closed_split(closed_pane(), closed_pane())
        else {
            unreachable!()
        };
        let no_number = ClosedLayoutNode::Split {
            direction,
            ratio: f32::NAN,
            first,
            second,
        };
        assert!(check_layout(&no_number).is_err());
        let empty = ClosedLayoutNode::Pane {
            pane_id: None,
            label: None,
            cwd: None,
            command: Some(Vec::new()),
            env: Default::default(),
        };
        assert!(check_layout(&empty).is_err());
    }

    #[test]
    fn a_swap_trades_two_leaves_and_keeps_every_split() {
        let pane = |id: &str| PaneLayoutNodeSnapshot::Pane {
            pane_id: id.to_owned(),
        };
        let mut tree = PaneLayoutNodeSnapshot::Split {
            direction: PaneLayoutDirection::Right,
            ratio: 0.3,
            first: Box::new(pane("w1:p1")),
            second: Box::new(PaneLayoutNodeSnapshot::Split {
                direction: PaneLayoutDirection::Down,
                ratio: 0.7,
                first: Box::new(pane("w1:p2")),
                second: Box::new(pane("w1:p3")),
            }),
        };
        swap_leaves(&mut tree, "w1:p1", "w1:p3");
        assert_eq!(
            tree,
            PaneLayoutNodeSnapshot::Split {
                direction: PaneLayoutDirection::Right,
                ratio: 0.3,
                first: Box::new(pane("w1:p3")),
                second: Box::new(PaneLayoutNodeSnapshot::Split {
                    direction: PaneLayoutDirection::Down,
                    ratio: 0.7,
                    first: Box::new(pane("w1:p2")),
                    second: Box::new(pane("w1:p1")),
                }),
            }
        );
    }

    #[test]
    fn a_split_ratio_is_kept_within_the_limits_and_half_when_it_is_no_number() {
        assert_eq!(valid_ratio(0.05), RATIO_MIN);
        assert_eq!(valid_ratio(0.95), RATIO_MAX);
        assert_eq!(valid_ratio(0.3), 0.3);
        assert_eq!(valid_ratio(f32::INFINITY), 0.5);
    }
}
