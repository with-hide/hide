// Divider drag math.
//
// The web shell sends one `resize_pane` when the drag ends (PRD S2 D-03):
// the guide line follows the pointer, the panes do not, and a release whose
// ratio change falls outside the core's accepted range sends nothing.

import type { LayoutNode } from "./snapshot";

export type ResizeDirection = "left" | "right" | "up" | "down";

/** The core's `resize_pane` accepts `0.001..=0.5`; a smaller move is not worth a round trip. */
export const MIN_AMOUNT = 0.001;
export const MAX_AMOUNT = 0.5;

export function resizeStep(
  travelPx: number,
  splitSpanPx: number,
  vertical: boolean,
): { direction: ResizeDirection; amount: number } | null {
  const delta = travelPx / Math.max(splitSpanPx, 1);
  const amount = Math.abs(delta);
  if (amount < MIN_AMOUNT || amount > MAX_AMOUNT || !Number.isFinite(amount)) return null;
  const direction: ResizeDirection = vertical ? (delta > 0 ? "right" : "left") : delta > 0 ? "down" : "up";
  return { direction, amount };
}

/**
 * The pane a divider's `resize_pane` names so the core moves that divider and
 * no other. The core moves the split on the named pane's edge toward the
 * direction (Herdr's pane resize), so a drag right or down names the pane just
 * before the divider and a drag left or up the pane just after it; any pane
 * further in would name a split nested inside one side (issue 2346).
 */
export function dividerPaneId(split: Extract<LayoutNode, { type: "split" }>, direction: ResizeDirection): string {
  const towardSecond = direction === "right" || direction === "down";
  let node = towardSecond ? split.first : split.second;
  while (node.type === "split") node = towardSecond ? node.second : node.first;
  return node.pane_id;
}
