// Parts derived from Herdr v0.9.5 (https://github.com/herdrdev/herdr, commit
// 2ecf19a): `swap_leaves` from src/layout.rs (`swap_pane_ids`),
// `resize_target`'s choice of edge from src/layout.rs (`resize_focused`,
// `nearest_resize_split`), `ratio_at` from src/layout.rs (`get_ratio_at`),
// `valid_ratio` from src/layout.rs (`valid_split_ratio`), and `check_layout`
// with its limits from src/app/api/layouts.rs (`validate_layout_tree`,
// `MAX_LAYOUT_PANES`, `MAX_LAYOUT_DEPTH`).
// Copyright 2025 herdr contributors. Licensed under the Apache License,
// Version 2.0; see NOTICE at the repository root.
// Changed by Hide: the rules work on Hide's own trees
// (`PaneLayoutNodeSnapshot`, `ClosedLayoutNode`) instead of ratatui's
// `Direction` and Herdr's `Node`; a resize finds the edge's split on the
// tree rather than by comparing laid-out rects; a check's message is Hide's;
// the rest of this file (split, close, laying out) is Hide's own.

//! A tab's pane tree and the rules that change it, the one place both the
//! core's own engine and the drawing ahead of the engine read them (PRD
//! engine-bundle-3-core-owns D-4): a split puts the new pane second at its
//! ratio (0.5 unless asked), a close puts the closed pane's sibling in its
//! parent's place, a resize moves the split on the pane's edge toward its
//! direction (the opposite edge at the tab's side), a swap trades two leaves
//! and keeps every split, every ratio is kept between [`RATIO_MIN`] and
//! [`RATIO_MAX`], and a tree is laid out into the rects a session carries
//! with the arithmetic the projection reads them back with
//! (`live::split_areas`). A layout given to be applied holds at
//! most [`MAX_PANES`] panes and [`MAX_DEPTH`] levels.

use crate::live::PaneResizeDirection;
use crate::model::{PaneLayoutDirection, PaneLayoutNodeSnapshot};
use crate::recent_closed::ClosedLayoutNode;
use crate::sidebar::{SessionLayoutPanePayload, SessionLayoutRect, SessionLayoutSplitPayload};

/// Every split is kept between these ratios.
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
/// sibling's first leaf, the pane that takes focus when the closed one had
/// focus.
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

/// The ratio moving `pane`'s edge by `amount` toward `direction` sets on the
/// split [`resize_target`] names, clamped. None when no split of that axis
/// holds the pane: a resize that changes nothing.
pub(crate) fn resized(
    root: &PaneLayoutNodeSnapshot,
    pane: &str,
    direction: PaneResizeDirection,
    amount: f32,
) -> Option<f32> {
    let sign = match direction {
        PaneResizeDirection::Left | PaneResizeDirection::Up => -1.0,
        PaneResizeDirection::Right | PaneResizeDirection::Down => 1.0,
    };
    let current = ratio_at(root, &resize_target(root, pane, direction)?)?;
    Some((current + sign * amount).clamp(RATIO_MIN, RATIO_MAX))
}

/// Sets the ratio of the split [`resize_target`] names. Returns whether it
/// found one.
pub(crate) fn set_resized_ratio(
    root: &mut PaneLayoutNodeSnapshot,
    pane: &str,
    direction: PaneResizeDirection,
    value: f32,
) -> bool {
    let Some(path) = resize_target(root, pane, direction) else {
        return false;
    };
    match ratio_at_mut(root, &path) {
        Some(ratio) => {
            *ratio = value;
            true
        }
        None => false,
    }
}

/// The split a resize of `pane` toward `direction` moves, as the turns from
/// the root (`true` for the second subtree): the one whose boundary is the
/// pane's edge on that side, or its opposite edge when the pane touches the
/// tab's side there, as Herdr's `resize_focused` chooses. So the last pane of
/// a split's first subtree names that split toward right or down, and the
/// first pane of its second subtree names it toward left or up, whatever
/// splits nest inside either subtree.
fn resize_target(
    root: &PaneLayoutNodeSnapshot,
    pane: &str,
    direction: PaneResizeDirection,
) -> Option<Vec<bool>> {
    let (axis, trailing) = match direction {
        PaneResizeDirection::Left => (PaneLayoutDirection::Right, false),
        PaneResizeDirection::Right => (PaneLayoutDirection::Right, true),
        PaneResizeDirection::Up => (PaneLayoutDirection::Down, false),
        PaneResizeDirection::Down => (PaneLayoutDirection::Down, true),
    };
    edge_split(root, pane, axis, trailing).or_else(|| edge_split(root, pane, axis, !trailing))
}

/// The path to the split of `axis` on `pane`'s right or lower edge
/// (`trailing`), or on its left or upper one: the innermost split of that
/// axis holding the pane in its first subtree, or in its second.
fn edge_split(
    node: &PaneLayoutNodeSnapshot,
    pane: &str,
    axis: PaneLayoutDirection,
    trailing: bool,
) -> Option<Vec<bool>> {
    let PaneLayoutNodeSnapshot::Split {
        direction,
        first,
        second,
        ..
    } = node
    else {
        return None;
    };
    let (child, in_second) = if contains(first, pane) {
        (first, false)
    } else if contains(second, pane) {
        (second, true)
    } else {
        return None;
    };
    if let Some(mut path) = edge_split(child, pane, axis, trailing) {
        path.insert(0, in_second);
        return Some(path);
    }
    (*direction == axis && in_second != trailing).then(Vec::new)
}

fn ratio_at(node: &PaneLayoutNodeSnapshot, path: &[bool]) -> Option<f32> {
    let PaneLayoutNodeSnapshot::Split {
        ratio,
        first,
        second,
        ..
    } = node
    else {
        return None;
    };
    match path.split_first() {
        None => Some(*ratio),
        Some((false, rest)) => ratio_at(first, rest),
        Some((true, rest)) => ratio_at(second, rest),
    }
}

fn ratio_at_mut<'a>(node: &'a mut PaneLayoutNodeSnapshot, path: &[bool]) -> Option<&'a mut f32> {
    let PaneLayoutNodeSnapshot::Split {
        ratio,
        first,
        second,
        ..
    } = node
    else {
        return None;
    };
    match path.split_first() {
        None => Some(ratio),
        Some((false, rest)) => ratio_at_mut(first, rest),
        Some((true, rest)) => ratio_at_mut(second, rest),
    }
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

    fn pane(id: &str) -> PaneLayoutNodeSnapshot {
        PaneLayoutNodeSnapshot::Pane {
            pane_id: id.to_owned(),
        }
    }

    fn split(
        direction: PaneLayoutDirection,
        first: PaneLayoutNodeSnapshot,
        second: PaneLayoutNodeSnapshot,
    ) -> PaneLayoutNodeSnapshot {
        PaneLayoutNodeSnapshot::Split {
            direction,
            ratio: 0.5,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    /// Every split of `node` with its path from the root and the panes on
    /// either side of its divider, read off the panes in order.
    fn dividers(
        node: &PaneLayoutNodeSnapshot,
        path: &mut Vec<bool>,
        out: &mut Vec<(Vec<bool>, PaneLayoutDirection, String, String)>,
    ) {
        fn leaves(node: &PaneLayoutNodeSnapshot) -> Vec<String> {
            match node {
                PaneLayoutNodeSnapshot::Pane { pane_id } => vec![pane_id.clone()],
                PaneLayoutNodeSnapshot::Split { first, second, .. } => {
                    [leaves(first), leaves(second)].concat()
                }
            }
        }
        let PaneLayoutNodeSnapshot::Split {
            direction,
            first,
            second,
            ..
        } = node
        else {
            return;
        };
        let before = leaves(first).pop().unwrap();
        let after = leaves(second).remove(0);
        out.push((path.clone(), *direction, before, after));
        path.push(false);
        dividers(first, path, out);
        path.pop();
        path.push(true);
        dividers(second, path, out);
        path.pop();
    }

    /// Issue 2346: the pane before a divider names it toward right or down
    /// and the pane after it toward left or up, in left- and right-nested
    /// tabs and with perpendicular splits on either side of the divider.
    #[test]
    fn every_divider_moves_when_the_pane_on_either_side_of_it_is_resized_across_it() {
        use PaneLayoutDirection::{Down, Right};
        let shapes = [
            split(Right, split(Right, pane("a"), pane("b")), pane("c")),
            split(
                Right,
                split(Right, split(Right, pane("a"), pane("b")), pane("c")),
                pane("d"),
            ),
            split(
                Right,
                pane("a"),
                split(Right, pane("b"), split(Right, pane("c"), pane("d"))),
            ),
            split(
                Right,
                split(Right, pane("a"), split(Down, pane("x"), pane("y"))),
                split(Right, split(Down, pane("p"), pane("q")), pane("c")),
            ),
            split(
                Down,
                split(Down, pane("a"), split(Right, pane("x"), pane("y"))),
                split(Down, split(Right, pane("p"), pane("q")), pane("c")),
            ),
        ];
        for shape in shapes {
            let mut found = Vec::new();
            dividers(&shape, &mut Vec::new(), &mut found);
            for (path, axis, before, after) in found {
                let (toward_after, toward_before) = match axis {
                    Right => (PaneResizeDirection::Right, PaneResizeDirection::Left),
                    Down => (PaneResizeDirection::Down, PaneResizeDirection::Up),
                };
                for (pane, direction, moved) in
                    [(&before, toward_after, 0.6), (&after, toward_before, 0.4)]
                {
                    let mut tree = shape.clone();
                    let ratio = resized(&tree, pane, direction, 0.1).unwrap();
                    assert!(set_resized_ratio(&mut tree, pane, direction, ratio));
                    let mut expected = shape.clone();
                    *ratio_at_mut(&mut expected, &path).unwrap() = moved;
                    assert_eq!(tree, expected, "{pane} {direction:?} in {shape:?}");
                }
            }
        }
    }

    /// A pane at the tab's side moves the split on its other edge, and a pane
    /// with no split of the axis moves nothing.
    #[test]
    fn a_pane_at_the_tab_s_side_moves_the_split_on_its_other_edge() {
        use PaneLayoutDirection::{Down, Right};
        let tree = split(Right, pane("a"), split(Down, pane("b"), pane("c")));
        assert_eq!(
            resized(&tree, "b", PaneResizeDirection::Right, 0.1),
            Some(0.6)
        );
        assert_eq!(
            resized(&tree, "a", PaneResizeDirection::Left, 0.1),
            Some(0.4)
        );
        assert_eq!(resized(&tree, "a", PaneResizeDirection::Up, 0.1), None);
    }

    #[test]
    fn a_split_ratio_is_kept_within_the_limits_and_half_when_it_is_no_number() {
        assert_eq!(valid_ratio(0.05), RATIO_MIN);
        assert_eq!(valid_ratio(0.95), RATIO_MAX);
        assert_eq!(valid_ratio(0.3), 0.3);
        assert_eq!(valid_ratio(f32::INFINITY), 0.5);
    }
}
