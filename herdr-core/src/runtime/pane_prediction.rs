//! What a tab's layout will be once Herdr applies the geometry changes Hide
//! has asked for (PRD instant-pane-topology D-07).
//!
//! Each prediction is absolute: it names the state Herdr will reach, not a
//! step from wherever the tree is. Folding one over a layout that already
//! shows it changes nothing, so a prediction can stay applied until its
//! operation settles without counting twice once Herdr's layout carries it.
//!
//! The rules follow what the pinned Herdr does (0.9.1, measured on a private
//! server): a split puts the new pane second at ratio 0.5 and focuses it; a
//! close collapses the parent split into the sibling, focuses the sibling and
//! ends a zoom on the closed pane; a zoom focuses its pane; a resize moves the
//! nearest enclosing split of its axis by the amount, clamped to 0.1..0.9.
//! Where the prediction and Herdr still differ (a focus Herdr places
//! elsewhere), Herdr's layout replaces it when it arrives.
//!
//! The tree rules themselves live in `engine::own::layout`, the ones the
//! core's own engine applies (PRD engine-bundle-3-core-owns D-4): a drawing
//! ahead of that engine is exactly what it then confirms.

use crate::engine::own::layout::{
    contains, lay_out, remove_leaf, resized, set_nearest_split_ratio, split_leaf,
};
use crate::live::PaneResizeDirection;
use crate::model::{PaneLayoutDirection, PaneLayoutNodeSnapshot, PaneLayoutSnapshot};
use crate::sidebar::SessionLayoutPayload;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Prediction {
    /// `target` split along `direction`, `created` second and focused.
    Split {
        target: String,
        direction: PaneLayoutDirection,
        created: String,
    },
    /// `pane` gone, its sibling in its place.
    Close { pane: String },
    /// The tab zoomed onto `pane` or not zoomed, with `pane` focused.
    Zoom { pane: String, zoomed: bool },
    /// The split of `axis` nearest `pane` at `ratio`.
    Resize {
        pane: String,
        axis: PaneLayoutDirection,
        ratio: f32,
    },
}

/// `layout` with every prediction applied in order.
pub(super) fn predict(
    layout: &PaneLayoutSnapshot,
    predictions: &[Prediction],
) -> PaneLayoutSnapshot {
    let mut predicted = layout.clone();
    for prediction in predictions {
        apply(&mut predicted, prediction);
    }
    predicted
}

/// The resize prediction for moving `pane`'s divider by `amount` toward
/// `direction`, read off `layout` as it will stand once the earlier
/// predictions land. None when no split of that axis encloses the pane, which
/// is what Herdr answers as a resize that changed nothing.
pub(super) fn resize_prediction(
    layout: &PaneLayoutSnapshot,
    pane: &str,
    direction: PaneResizeDirection,
    amount: f32,
) -> Option<Prediction> {
    let (axis, ratio) = resized(&layout.root, pane, direction, amount)?;
    Some(Prediction::Resize {
        pane: pane.to_owned(),
        axis,
        ratio,
    })
}

/// `raw`, a tab's layout as Herdr sent it, with `predictions` applied, in the
/// rect form a session carries.
///
/// The rects are laid out with the same arithmetic the projection reads them
/// back with (`live::split_areas`), so projecting the result gives exactly the
/// predicted tree, and every reader of the session (the tab's pane rows, the
/// terminal projection, the attaches) sees one consistent layout. None when
/// the predictions change nothing, or when the layout cannot be read or a
/// predicted split has no room; the tab is then drawn as Herdr confirmed it.
pub(super) fn overlay_session_layout(
    raw: &SessionLayoutPayload,
    predictions: &[Prediction],
) -> Option<SessionLayoutPayload> {
    let confirmed = crate::live::project_layout(raw).ok()?;
    let predicted = predict(&confirmed, predictions);
    if predicted == confirmed {
        return None;
    }
    let mut panes = Vec::new();
    let mut splits = Vec::new();
    lay_out(&predicted.root, raw.area, &mut panes, &mut splits).ok()?;
    Some(SessionLayoutPayload {
        workspace_id: raw.workspace_id.clone(),
        tab_id: raw.tab_id.clone(),
        zoomed: predicted.zoomed,
        area: raw.area,
        focused_pane_id: predicted.focused_pane_id,
        panes,
        splits,
    })
}

fn apply(layout: &mut PaneLayoutSnapshot, prediction: &Prediction) {
    match prediction {
        Prediction::Split {
            target,
            direction,
            created,
        } => {
            if contains(&layout.root, created) || !contains(&layout.root, target) {
                return;
            }
            split_leaf(&mut layout.root, target, *direction, created);
            layout.focused_pane_id = created.clone();
            // Herdr shows the new pane, so a zoom does not survive the split.
            layout.zoomed = false;
        }
        Prediction::Close { pane } => {
            // The last pane of a tab is a tab close, which `leaving_tab` owns.
            if matches!(&layout.root, PaneLayoutNodeSnapshot::Pane { .. }) {
                return;
            }
            let Some(sibling_first_leaf) = remove_leaf(&mut layout.root, pane) else {
                return;
            };
            if layout.focused_pane_id == *pane {
                layout.focused_pane_id = sibling_first_leaf;
                layout.zoomed = false;
            }
        }
        Prediction::Zoom { pane, zoomed } => {
            if !contains(&layout.root, pane) {
                return;
            }
            layout.zoomed = *zoomed;
            layout.focused_pane_id = pane.clone();
        }
        Prediction::Resize { pane, axis, ratio } => {
            set_nearest_split_ratio(&mut layout.root, pane, *axis, *ratio);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidebar::{SessionLayoutPanePayload, SessionLayoutRect, SessionLayoutSplitPayload};

    fn pane(id: &str) -> PaneLayoutNodeSnapshot {
        PaneLayoutNodeSnapshot::Pane {
            pane_id: id.to_owned(),
        }
    }

    fn split(
        direction: PaneLayoutDirection,
        ratio: f32,
        first: PaneLayoutNodeSnapshot,
        second: PaneLayoutNodeSnapshot,
    ) -> PaneLayoutNodeSnapshot {
        PaneLayoutNodeSnapshot::Split {
            direction,
            ratio,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    fn layout(root: PaneLayoutNodeSnapshot, focused: &str) -> PaneLayoutSnapshot {
        PaneLayoutSnapshot {
            workspace_id: "w1".to_owned(),
            tab_id: "w1:t1".to_owned(),
            focused_pane_id: focused.to_owned(),
            zoomed: false,
            root,
        }
    }

    use PaneLayoutDirection::{Down, Right};

    fn rect(x: u16, y: u16, width: u16, height: u16) -> SessionLayoutRect {
        SessionLayoutRect {
            x,
            y,
            width,
            height,
        }
    }

    /// Two panes side by side, as Herdr sends them: rects with a border
    /// column the projection does not need to reproduce.
    fn raw_two_panes() -> SessionLayoutPayload {
        SessionLayoutPayload {
            workspace_id: "w1".to_owned(),
            tab_id: "w1:t1".to_owned(),
            zoomed: false,
            area: rect(0, 0, 120, 40),
            focused_pane_id: "w1:p1".to_owned(),
            panes: vec![
                SessionLayoutPanePayload {
                    pane_id: "w1:p1".to_owned(),
                    rect: rect(0, 0, 59, 40),
                },
                SessionLayoutPanePayload {
                    pane_id: "w1:p2".to_owned(),
                    rect: rect(60, 0, 60, 40),
                },
            ],
            splits: vec![SessionLayoutSplitPayload {
                direction: Right,
                ratio: 0.5,
                rect: rect(0, 0, 120, 40),
            }],
        }
    }

    /// The overlaid session layout reads back as exactly the predicted tree,
    /// so every reader of the session agrees with what the canvas draws.
    #[test]
    fn an_overlaid_session_layout_projects_to_the_predicted_tree() {
        let raw = raw_two_panes();
        let predictions = [Prediction::Split {
            target: "w1:p2".to_owned(),
            direction: Down,
            created: "w1:p3".to_owned(),
        }];
        let overlaid = overlay_session_layout(&raw, &predictions).expect("the split changes it");
        let expected = predict(&crate::live::project_layout(&raw).unwrap(), &predictions);
        assert_eq!(crate::live::project_layout(&overlaid).unwrap(), expected);
        assert_eq!(overlaid.focused_pane_id, "w1:p3");
        assert_eq!(overlaid.area, raw.area);
    }

    /// A prediction Herdr's layout already shows leaves the tab as sent.
    #[test]
    fn a_prediction_the_layout_already_shows_overlays_nothing() {
        let raw = raw_two_panes();
        let predictions = [Prediction::Split {
            target: "w1:p1".to_owned(),
            direction: Right,
            created: "w1:p2".to_owned(),
        }];
        assert_eq!(overlay_session_layout(&raw, &predictions), None);
    }

    /// A split with no room to divide is not drawn ahead of Herdr.
    #[test]
    fn a_split_with_no_room_overlays_nothing() {
        let mut raw = raw_two_panes();
        raw.area = rect(0, 0, 2, 40);
        raw.panes[0].rect = rect(0, 0, 1, 40);
        raw.panes[1].rect = rect(1, 0, 1, 40);
        raw.splits[0].rect = raw.area;
        let predictions = [Prediction::Split {
            target: "w1:p1".to_owned(),
            direction: Right,
            created: "w1:p3".to_owned(),
        }];
        assert_eq!(overlay_session_layout(&raw, &predictions), None);
    }

    #[test]
    fn a_split_puts_the_new_pane_second_at_half_and_focuses_it() {
        let before = layout(pane("w1:p1"), "w1:p1");
        let after = predict(
            &before,
            &[Prediction::Split {
                target: "w1:p1".to_owned(),
                direction: Right,
                created: "w1:p2".to_owned(),
            }],
        );
        assert_eq!(after.root, split(Right, 0.5, pane("w1:p1"), pane("w1:p2")));
        assert_eq!(after.focused_pane_id, "w1:p2");
    }

    #[test]
    fn a_split_nests_where_its_target_stands() {
        let before = layout(split(Right, 0.5, pane("w1:p1"), pane("w1:p2")), "w1:p2");
        let after = predict(
            &before,
            &[Prediction::Split {
                target: "w1:p2".to_owned(),
                direction: Down,
                created: "w1:p3".to_owned(),
            }],
        );
        assert_eq!(
            after.root,
            split(
                Right,
                0.5,
                pane("w1:p1"),
                split(Down, 0.5, pane("w1:p2"), pane("w1:p3"))
            )
        );
    }

    #[test]
    fn a_split_herdr_already_shows_changes_nothing() {
        let shown = layout(split(Right, 0.5, pane("w1:p1"), pane("w1:p2")), "w1:p2");
        let again = predict(
            &shown,
            &[Prediction::Split {
                target: "w1:p1".to_owned(),
                direction: Right,
                created: "w1:p2".to_owned(),
            }],
        );
        assert_eq!(again, shown);
    }

    #[test]
    fn a_close_collapses_the_split_and_focuses_the_sibling() {
        let before = layout(
            split(
                Right,
                0.5,
                pane("w1:p1"),
                split(Down, 0.5, pane("w1:p2"), pane("w1:p3")),
            ),
            "w1:p1",
        );
        let after = predict(
            &before,
            &[Prediction::Close {
                pane: "w1:p1".to_owned(),
            }],
        );
        assert_eq!(after.root, split(Down, 0.5, pane("w1:p2"), pane("w1:p3")));
        assert_eq!(after.focused_pane_id, "w1:p2");
        assert_eq!(
            predict(
                &after,
                &[Prediction::Close {
                    pane: "w1:p1".to_owned()
                }]
            ),
            after
        );
    }

    #[test]
    fn closing_the_zoomed_pane_ends_the_zoom_and_another_close_keeps_it() {
        let mut zoomed = layout(split(Right, 0.5, pane("w1:p1"), pane("w1:p2")), "w1:p1");
        zoomed.zoomed = true;
        let closed_zoomed = predict(
            &zoomed,
            &[Prediction::Close {
                pane: "w1:p1".to_owned(),
            }],
        );
        assert!(!closed_zoomed.zoomed);
        assert_eq!(closed_zoomed.root, pane("w1:p2"));
        let mut three = layout(
            split(
                Right,
                0.5,
                pane("w1:p1"),
                split(Down, 0.5, pane("w1:p2"), pane("w1:p3")),
            ),
            "w1:p1",
        );
        three.zoomed = true;
        let closed_other = predict(
            &three,
            &[Prediction::Close {
                pane: "w1:p3".to_owned(),
            }],
        );
        assert!(closed_other.zoomed);
        assert_eq!(closed_other.focused_pane_id, "w1:p1");
    }

    #[test]
    fn the_last_pane_of_a_tab_is_not_removed_by_a_pane_close() {
        let single = layout(pane("w1:p1"), "w1:p1");
        assert_eq!(
            predict(
                &single,
                &[Prediction::Close {
                    pane: "w1:p1".to_owned()
                }]
            ),
            single
        );
    }

    #[test]
    fn a_zoom_sets_the_state_and_focus_whatever_it_started_from() {
        let before = layout(split(Right, 0.5, pane("w1:p1"), pane("w1:p2")), "w1:p1");
        let zoom = Prediction::Zoom {
            pane: "w1:p2".to_owned(),
            zoomed: true,
        };
        let once = predict(&before, std::slice::from_ref(&zoom));
        assert!(once.zoomed);
        assert_eq!(once.focused_pane_id, "w1:p2");
        assert_eq!(predict(&once, &[zoom]), once);
    }

    #[test]
    fn a_resize_moves_the_nearest_split_of_its_axis_from_either_side() {
        // Herdr 0.9.1: +0.1 right on p2 (first child) and on p4 (inside the
        // second child) both move the root right split the same way.
        let tree = layout(
            split(
                Right,
                0.5,
                pane("w1:p2"),
                split(Down, 0.5, pane("w1:p4"), pane("w1:p5")),
            ),
            "w1:p2",
        );
        let from_first =
            resize_prediction(&tree, "w1:p2", PaneResizeDirection::Right, 0.1).unwrap();
        let from_second =
            resize_prediction(&tree, "w1:p4", PaneResizeDirection::Right, 0.1).unwrap();
        for prediction in [from_first, from_second] {
            let after = predict(&tree, std::slice::from_ref(&prediction));
            let PaneLayoutNodeSnapshot::Split { ratio, second, .. } = &after.root else {
                panic!("root stays a split");
            };
            assert!((ratio - 0.6).abs() < 1e-6);
            assert_eq!(**second, split(Down, 0.5, pane("w1:p4"), pane("w1:p5")));
            assert_eq!(predict(&after, &[prediction]), after);
        }
        let down = resize_prediction(&tree, "w1:p5", PaneResizeDirection::Up, 0.1).unwrap();
        let after = predict(&tree, &[down]);
        let PaneLayoutNodeSnapshot::Split { second, .. } = &after.root else {
            panic!("root stays a split");
        };
        assert_eq!(**second, split(Down, 0.4, pane("w1:p4"), pane("w1:p5")));
    }

    #[test]
    fn a_resize_is_clamped_like_herdr_and_needs_a_split_of_its_axis() {
        let tree = layout(split(Right, 0.6, pane("w1:p1"), pane("w1:p2")), "w1:p1");
        let Some(Prediction::Resize { ratio, .. }) =
            resize_prediction(&tree, "w1:p1", PaneResizeDirection::Right, 0.5)
        else {
            panic!("a right split encloses the pane");
        };
        assert!((ratio - 0.9).abs() < 1e-6);
        assert_eq!(
            resize_prediction(&tree, "w1:p1", PaneResizeDirection::Up, 0.1),
            None
        );
        assert_eq!(
            resize_prediction(
                &layout(pane("w1:p1"), "w1:p1"),
                "w1:p1",
                PaneResizeDirection::Left,
                0.1
            ),
            None
        );
    }

    #[test]
    fn predictions_fold_in_the_order_they_were_asked() {
        let before = layout(pane("w1:p1"), "w1:p1");
        let after = predict(
            &before,
            &[
                Prediction::Split {
                    target: "w1:p1".to_owned(),
                    direction: Right,
                    created: "w1:p2".to_owned(),
                },
                Prediction::Split {
                    target: "w1:p2".to_owned(),
                    direction: Right,
                    created: "w1:p3".to_owned(),
                },
                Prediction::Close {
                    pane: "w1:p1".to_owned(),
                },
            ],
        );
        assert_eq!(after.root, split(Right, 0.5, pane("w1:p2"), pane("w1:p3")));
        assert_eq!(after.focused_pane_id, "w1:p3");
    }
}
