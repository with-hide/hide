// How a mission's map zooms (docs/UI_BEHAVIOR.md, A mission's map): it
// opens at 맞춤, the whole map inside its box and never larger
// than its own size, and a pinch, a wheel with ⌘ or Ctrl held, or the
// box's − and + move it between 40% and 200%, the buttons in Chrome's steps.
// The scale is a view of the moment kept by the page: nothing reaches the
// core, and a reopened map fits again. The wheel's factor, the anchoring
// around the pointer and the gutter are the image viewer's (`viewerZoom.ts`).

import { GUTTER, type Layout, type Size } from "./viewers/viewerZoom";
import { nextZoomFactor } from "./zoomSteps";

/** Below this a card's caption stops being readable even zoomed out on purpose. */
export const MIN_MAP_ZOOM = 0.4;
/** Above this a card is larger than the box it is read in. */
export const MAX_MAP_ZOOM = 2;

export function clampMapZoom(zoom: number): number {
  return Math.min(MAX_MAP_ZOOM, Math.max(MIN_MAP_ZOOM, zoom));
}

/**
 * 맞춤: the whole map inside the room left by the gutter and, below it, the
 * `below` strip the zoom buttons stand in, never enlarged, never below the floor.
 */
export function mapFit(map: Size, room: Size, below: number): number {
  if (map.width <= 0 || map.height <= 0) return 1;
  const fit = Math.min(1, inside(room.width) / map.width, Math.max(1, inside(room.height) - below) / map.height);
  return Math.max(MIN_MAP_ZOOM, fit);
}

/** − and + move one of Chrome's steps from where the map is drawn now, held to the range. */
export function stepMapZoom(zoom: number, step: "in" | "out"): number {
  return clampMapZoom(nextZoomFactor(zoom, step));
}

/**
 * The map drawn at `scale` in a box `viewport` wide: centred across the box
 * while it is narrower, starting at the gutter and scrolling to each edge
 * once it is wider, and as tall as it is drawn with the `below` strip under
 * it, so a box sized by its content never holds the height of a larger zoom
 * and the zoom buttons never cover the map's last row.
 */
export function mapLayout(map: Size, scale: number, viewport: Size, below: number): Layout {
  const drawn = { width: map.width * scale, height: map.height * scale };
  const width = Math.max(viewport.width, drawn.width + 2 * GUTTER);
  return { width, height: drawn.height + 2 * GUTTER + below, boxes: [{ left: (width - drawn.width) / 2, top: GUTTER, ...drawn }] };
}

function inside(length: number): number {
  return Math.max(1, length - 2 * GUTTER);
}
