// The one card the core reads (PRD lane-desk-pair-view D-13): the card in
// front of the active area of the desk the lane draws. A record card and a
// pull request's card ask this one question before they read, and one out of
// front keeps what it last drew, so two areas never take the read in turns.

import { sameDetailItem, type DetailItem, type SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { frontCard, workspaceViewOf } from "./workspace";

/** Whether `item` is the card the core reads now; a folded desk's card is not, since its read would open the desk again. */
export function isDeskFront(rest: SnapshotRest | null, item: DetailItem): boolean {
  const view = workspaceViewOf(rest);
  return Boolean(view?.open && view.tray.shown) && sameDetailItem(frontCard(view), item);
}

export function useDeskFront(item: DetailItem): boolean {
  return useShellStore((state) => isDeskFront(state.rest, item));
}

/**
 * What cards out of front last drew, by card, so one keeps its content rather
 * than a read in flight for another; the oldest goes past `cap`.
 */
export class KeptDrawings<T> {
  private readonly rows = new Map<string, T>();
  constructor(private readonly cap: number) {}
  keep(key: string, value: T): void {
    this.rows.delete(key);
    this.rows.set(key, value);
    if (this.rows.size > this.cap) this.rows.delete(this.rows.keys().next().value!);
  }
  get(key: string): T | null {
    return this.rows.get(key) ?? null;
  }
}
