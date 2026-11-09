// Where a practice card floats: beside or in front of what it talks about, in
// free space, never over a target. A pure function of boxes, so the placement
// is the same on every screen size and a test can state it.

export type Box = { x: number; y: number; w: number; h: number };

/** The gap kept between a card and a target, and between a card and the screen's edge. */
const GAP = 16;

const overlap = (a: Box, b: Box) => Math.max(0, Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x)) * Math.max(0, Math.min(a.y + a.h, b.y + b.h) - Math.max(a.y, b.y));
const grown = (box: Box, by: number): Box => ({ x: box.x - by, y: box.y - by, w: box.w + 2 * by, h: box.h + 2 * by });
const distance = (a: Box, b: Box) => Math.hypot(Math.max(0, Math.max(a.x - (b.x + b.w), b.x - (a.x + a.w))), Math.max(0, Math.max(a.y - (b.y + b.h), b.y - (a.y + a.h))));

/**
 * The top-left corner for a card of `size`: beside each target (right, left,
 * below, above, each aligned to its start, middle or end) or in a corner or the
 * middle of the screen, whichever is free of every target and of `avoid` (the
 * parts of the practice layer itself) and nearest the targets. With no free
 * place, the one that covers the least of them.
 */
export function placeCard(size: { w: number; h: number }, targets: Box[], avoid: Box[], view: { w: number; h: number }): { x: number; y: number } {
  const { w, h } = size;
  const candidates: Box[] = [];
  const add = (x: number, y: number) => candidates.push({ x: Math.min(Math.max(GAP, x), Math.max(GAP, view.w - w - GAP)), y: Math.min(Math.max(GAP, y), Math.max(GAP, view.h - h - GAP)), w, h });
  for (const t of targets) {
    for (const y of [t.y, t.y + (t.h - h) / 2, t.y + t.h - h]) {
      add(t.x + t.w + GAP, y);
      add(t.x - w - GAP, y);
    }
    for (const x of [t.x, t.x + (t.w - w) / 2, t.x + t.w - w]) {
      add(x, t.y + t.h + GAP);
      add(x, t.y - h - GAP);
    }
  }
  for (const x of [GAP, (view.w - w) / 2, view.w - w - GAP]) for (const y of [GAP, (view.h - h) / 2, view.h - h - GAP]) add(x, y);
  const cost = (card: Box) => {
    const covered = targets.reduce((sum, t) => sum + overlap(card, grown(t, GAP / 2)), 0) + avoid.reduce((sum, a) => sum + overlap(card, a), 0);
    const near = targets.length ? Math.min(...targets.map((t) => distance(card, t))) : Math.hypot(card.x + w / 2 - view.w / 2, card.y + h / 2 - view.h / 2);
    return covered * 1000 + near;
  };
  const best = candidates.reduce((one, other) => (cost(other) < cost(one) ? other : one));
  return { x: Math.round(best.x), y: Math.round(best.y) };
}
