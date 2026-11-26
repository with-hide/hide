// Where the operator left each conversation (Desk's and a session's beside it), so leaving it for another
// screen and coming back shows the lines they were reading (mission-106 answer, picture section 5). The web's
// memory alone, while the app runs: the core keeps no scroll, and the app opened again starts at the foot.

/** A conversation left above its foot: the item at its top edge, by its `data-scroll-anchor`, and how far below that edge its top stood. */
export type ScrollPlace = { anchor: string; offset: number };

/** How many conversations keep their place; the one left longest ago goes first. */
export const KEPT_SCROLLS = 32;

const kept = new Map<string, ScrollPlace>();

/** Remembers where `key`'s conversation was left: `null` at its foot, which is where one never left starts too. */
export function keepScroll(key: string, place: ScrollPlace | null): void {
  kept.delete(key);
  if (place === null) return;
  kept.set(key, place);
  while (kept.size > KEPT_SCROLLS) kept.delete(kept.keys().next().value!);
}

/** Where `key`'s conversation was left, `null` at its foot or never left. */
export function keptScroll(key: string): ScrollPlace | null {
  return kept.get(key) ?? null;
}

/** Forgets every place, for tests. */
export function forgetScrolls(): void {
  kept.clear();
}

function anchors(box: HTMLElement): HTMLElement[] {
  return [...box.querySelectorAll<HTMLElement>("[data-scroll-anchor]")];
}

/** The item at `box`'s top edge, the first whose bottom is below it, and where its top stands from the edge. */
export function scrollPlace(box: HTMLElement): ScrollPlace | null {
  const top = box.getBoundingClientRect().top;
  for (const item of anchors(box)) {
    const rect = item.getBoundingClientRect();
    if (rect.bottom > top) return { anchor: item.dataset.scrollAnchor!, offset: rect.top - top };
  }
  return null;
}

function anchorOf(box: HTMLElement, place: ScrollPlace): HTMLElement | null {
  return anchors(box).find((item) => item.dataset.scrollAnchor === place.anchor) ?? null;
}

/** How far `place`'s item stands from where it was left, `null` while the conversation does not hold it. */
function drift(box: HTMLElement, place: ScrollPlace): number | null {
  const item = anchorOf(box, place);
  if (!item) return null;
  return item.getBoundingClientRect().top - box.getBoundingClientRect().top - place.offset;
}

/** Scrolls `box` so `place`'s item stands where it was left; false while the conversation does not hold it. */
export function placeAt(box: HTMLElement, place: ScrollPlace): boolean {
  const off = drift(box, place);
  if (off === null) return false;
  if (Math.abs(off) >= 1) box.scrollTop += off;
  return true;
}

/** Whether `place`'s item still stands where it was left: a scroll that moved it was the operator's. */
export function stillAt(box: HTMLElement, place: ScrollPlace): boolean {
  const off = drift(box, place);
  return off !== null && Math.abs(off) < 2;
}
