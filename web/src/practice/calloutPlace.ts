// Where a callout's tag goes: stuck to the box drawn around what it names, on the
// side (right, below, above or left) where it covers the least of the other boxes
// and of the other tags, each tag deciding in turn. A pure function of boxes, so
// a test can state it.

import type { Box } from "./cardPlace";

export type TagSide = "right" | "below" | "above" | "left";

/** How far a tag stands off its box. */
const STAND = 3;
/** What a tag may not cover, grown a little so two never touch. */
const MARGIN = 2;

/** Sides in the order a tag prefers them: beside the box first, where the screen is usually freer. */
const SIDES: TagSide[] = ["right", "below", "above", "left"];

const overlap = (a: Box, b: Box) => Math.max(0, Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x)) * Math.max(0, Math.min(a.y + a.h, b.y + b.h) - Math.max(a.y, b.y));
const grown = (box: Box, by: number): Box => ({ x: box.x - by, y: box.y - by, w: box.w + 2 * by, h: box.h + 2 * by });
const inside = (box: Box, view: Box) => box.x >= view.x && box.y >= view.y && box.x + box.w <= view.x + view.w && box.y + box.h <= view.y + view.h;

/** The places a tag of `size` can stand against `box`: one side, aligned to the box's start, middle or end. */
function candidates(box: Box, size: { w: number; h: number }): { side: TagSide; at: Box }[] {
  const { w, h } = size;
  const across = [box.x, box.x + (box.w - w) / 2, box.x + box.w - w];
  const along = [box.y + (box.h - h) / 2, box.y, box.y + box.h - h];
  return SIDES.flatMap((side): { side: TagSide; at: Box }[] => {
    if (side === "right") return along.map((y) => ({ side, at: { x: box.x + box.w + STAND, y, w, h } }));
    if (side === "left") return along.map((y) => ({ side, at: { x: box.x - w - STAND, y, w, h } }));
    return across.map((x) => ({ side, at: { x, y: side === "below" ? box.y + box.h + STAND : box.y - h - STAND, w, h } }));
  });
}

/**
 * The place of each tag, in the order given: `boxes` are what the tags name,
 * `sizes` the tags', `avoid` parts of the screen no tag may cover (the mode bar),
 * `view` where a tag must stay. A place that covers another box, another tag or
 * an avoided part costs by the area it covers, one outside the view costs more,
 * and between equals the first side in `SIDES` and the first alignment wins.
 */
export function placeTags(boxes: Box[], sizes: { w: number; h: number }[], avoid: Box[], view: Box): { x: number; y: number; side: TagSide }[] {
  const placed: Box[] = [];
  return boxes.map((box, at) => {
    const others = boxes.filter((_, other) => other !== at).map((other) => grown(other, MARGIN));
    const cost = (tag: Box) => {
      const covered = [...others, ...placed.map((one) => grown(one, MARGIN)), ...avoid].reduce((sum, part) => sum + overlap(tag, part), 0);
      return covered * 100 + (inside(tag, view) ? 0 : 1e7);
    };
    const all = candidates(box, sizes[at]!);
    const best = all.reduce((one, other) => (cost(other.at) < cost(one.at) ? other : one));
    placed.push(best.at);
    return { x: Math.round(best.at.x), y: Math.round(best.at.y), side: best.side };
  });
}
