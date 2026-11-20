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
 * The front tab's pair view while the screen draws it (B17, B26): the core
 * keeps it as the tab's state (D-15), and it is drawn while the lane shows
 * that tab's desk with something on it. The canvas that gives the pane up and
 * the layer that draws it read this one value, in one render, so the
 * terminal parks and is shown again in the same commit (D-26).
 */
export function drawnPair(view: WorkspaceView | null | undefined): PairView | null {
  const pair = view?.tray.pair;
  if (!pair || !view.open || !view.tray.shown || (view.layout?.display_count ?? 0) === 0) return null;
  return pair;
}

/**
 * The tabs the tab row and the sidebar mark as in pair view (D-17): every
 * paired tab while the lane is open, and the front one only while it is
 * drawn, so a lane closed with ⌘E leaves no mark on a screen that shows the
 * tab's own layout.
 */
export function markedPairs(view: WorkspaceView | null | undefined): PairView[] {
  if (!view?.open) return [];
  const drawn = drawnPair(view);
  return (view.pairs ?? []).filter((pair) => pair.tab_id !== view.tray.tab_id || drawn !== null);
}
