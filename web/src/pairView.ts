// Pair view's ◧ (PRD lane-desk-pair-view B19, B20, B22, D-31, D-53): what
// the pane head, the pane menu and the terminal menu offer for a pane, read
// once from the core's facts, so the three never disagree.

import type { PairView, WorkspaceView } from "./workspace";

/** `on` on the pane pair view draws, which ◧ takes back to the usual view; `off` on every other pane, which ◧ enters pair view with (D-53). */
export type PairOffer = "off" | "on";

export function pairOffer(view: WorkspaceView | null | undefined, paneId: string): PairOffer {
  return drawnPair(view)?.pane_id === paneId ? "on" : "off";
}

/** The words ◧ says for its offer, on the button's hint and as the menu item. */
export function pairLabelKey(offer: PairOffer): "panes.menu.pair" | "panes.menu.pairOff" {
  return offer === "on" ? "panes.menu.pairOff" : "panes.menu.pair";
}

/**
 * Whether the pane head draws its ◧ cell (D-43, D-53, D-54): on a pane with
 * a View to show beside the desk, one its agent opened or one the operator
 * opened from its terminal's links, and on the pane pair view draws,
 * pressed. Every other pane enters pair view from ⌘. and its menus only, so
 * its head stays quiet.
 */
export function pairCellShown(view: WorkspaceView | null | undefined, paneId: string): boolean {
  return pairOffer(view, paneId) === "on" || view?.panes_with_views?.includes(paneId) === true;
}

/**
 * The pair view drawn now (B17, D-45, D-53): the core publishes it while
 * the window's mode is on, for the pane the operator is on in the front
 * tab, beside the desk or alone. The canvas that gives the pane up and the
 * layer that draws it read this one value, in one render, so the terminal
 * parks and is shown again in the same commit (D-26).
 */
export function drawnPair(view: WorkspaceView | null | undefined): PairView | null {
  return view?.tray.pair ?? null;
}

/** The pair view drawn beside its desk, the lane the right side of the body (B17); null while the pane is drawn alone (D-53) or no pair view is drawn. */
export function pairBesideDesk(view: WorkspaceView | null | undefined): PairView | null {
  const pair = drawnPair(view);
  return pair && pair.alone !== true ? pair : null;
}

/** Whether the window's pair view mode is on (D-45). */
export function pairModeOn(view: WorkspaceView | null | undefined): boolean {
  return view?.pair_mode?.on === true;
}

/** The tab and pane the tab row and the sidebar mark (D-17): the one pair view drawn, since the mode is the window's. */
export function markedPairs(view: WorkspaceView | null | undefined): PairView[] {
  const drawn = drawnPair(view);
  return drawn ? [drawn] : [];
}
