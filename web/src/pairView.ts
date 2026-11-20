// Pair view's ◧ (PRD lane-desk-pair-view B19, B20, B22, D-31): what the
// pane head, the pane menu and the terminal menu offer for a pane, read once
// from the core's facts, so the three never disagree.

import type { PairView, WorkspaceView } from "./workspace";

/** `on` on the pane in its tab's pair view; `off` on a pane the core lists with something to show beside the desk; `none`, off with why, on every other (B19). */
export type PairOffer = "off" | "on" | "none";

export function pairOffer(view: WorkspaceView | null | undefined, paneId: string): PairOffer {
  if (drawnPair(view)?.pane_id === paneId) return "on";
  return view?.pairable?.includes(paneId) ? "off" : "none";
}

/** The words ◧ says for its offer, on the button's hint and as the menu item. */
export function pairLabelKey(offer: PairOffer): "panes.menu.pair" | "panes.menu.pairOff" | "panes.menu.pairNone" {
  return offer === "on" ? "panes.menu.pairOff" : offer === "off" ? "panes.menu.pair" : "panes.menu.pairNone";
}

/**
 * The pair view drawn now (B17, D-45): the core publishes it while the
 * window's mode is on and the pane the operator is on in the front tab has
 * something to show. The canvas that gives the pane up and the layer that
 * draws it read this one value, in one render, so the terminal parks and is
 * shown again in the same commit (D-26).
 */
export function drawnPair(view: WorkspaceView | null | undefined): PairView | null {
  return view?.tray.pair ?? null;
}

/** Whether the window's pair view mode is on (D-45), drawn or not: a shell pane draws its tab's own layout and the mode stays. */
export function pairModeOn(view: WorkspaceView | null | undefined): boolean {
  return view?.pair_mode?.on === true;
}

/** The tab and pane the tab row and the sidebar mark (D-17): the one pair view drawn, since the mode is the window's. */
export function markedPairs(view: WorkspaceView | null | undefined): PairView[] {
  const drawn = drawnPair(view);
  return drawn ? [drawn] : [];
}
