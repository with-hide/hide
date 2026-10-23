// Shared geometry, tree queries and drop decisions. Column adapters supply
// vocabulary and item equivalence; this module has no document or terminal policy.
import { translate } from "./i18n/client";
import type { MessageKey } from "./i18n/catalogs";
export type AreaItem = { id: string };
export type Area<I extends AreaItem> = { id: string; active: string | null; displays: I[] };
export type AreaSplit<I extends AreaItem> = { id: string; axis: "row" | "column"; ratio: number; first: AreaNode<I>; second: AreaNode<I> };
export type AreaNode<I extends AreaItem> = { area: Area<I> } | { split: AreaSplit<I> };
export type AreaLayout<I extends AreaItem> = { root: AreaNode<I>; active_area: string; limits: { areas: number; depth: number; displays: number }; display_count: number };
/** Which column an area tree draws, which picks the wording of every sentence about it. */
export type AreaWords = { kind: "agent" | "view" };

export type Rect = { x: number; y: number; width: number; height: number };
export type Point = { x: number; y: number };
export type Edge = "left" | "right" | "up" | "down";

/** The pixel sizes the rules read, each from its design token. */
export type LayoutSizes = {
  /** `--size-workspace-area-min`: the narrowest a View area may be. */
  areaMinWidth: number;
  /** `--size-view-area-min-height`: the shortest a View area may be, its tab bar included. */
  areaMinHeight: number;
  /** `--size-resize-handle`: a divider's thickness. */
  divider: number;
  /** `--size-tab-strip`: an area's tab bar height. */
  tabStrip: number;
};

/** The core clamps a split's ratio to this range (contract 1). */
export const RATIO_MIN = 0.15;
export const RATIO_MAX = 0.85;
/** One arrow key or Grow/Shrink moves a divider by this share. */
export const RESIZE_STEP = 0.05;
/** How far into an area's content, as a share of its size, an edge's drop zone reaches. */
export const EDGE_ZONE = 0.3;

const SPLIT_LABEL = { right: "panes.area.splitRight", left: "panes.area.splitLeft", up: "panes.area.splitUp", down: "panes.area.splitDown" } as const satisfies Record<Edge, MessageKey>;
const MOVE_LABEL = { right: "panes.area.moveRight", left: "panes.area.moveLeft", up: "panes.area.moveUp", down: "panes.area.moveDown" } as const satisfies Record<Edge, MessageKey>;

/** The menu item and the drop preview that split toward `edge`. */
export function splitLabel(edge: Edge): string {
  return translate(SPLIT_LABEL[edge]);
}

/** The menu item that moves a tab into the neighbouring area toward `edge`. */
export function moveLabel(edge: Edge): string {
  return translate(MOVE_LABEL[edge]);
}

type AreaSentence = "itemGone" | "areaGone" | "onlyItem" | "tooSmall" | "notOnScreen" | "tooNarrow" | "tooShort" | "dropHint" | "onlyOne" | "cannotGrow" | "cannotShrink";

/** A sentence about one column's areas, worded for that column. */
export function areaSentence(words: AreaWords, name: AreaSentence): string {
  return translate(`panes.area.${name}.${words.kind}` as const);
}
// --- tree queries ------------------------------------------------------------

/** Every area, in tree order: first child before second, depth first. */
export function areasOf<I extends AreaItem>(node: AreaNode<I>): Area<I>[] {
  return "area" in node ? [node.area] : [...areasOf(node.split.first), ...areasOf(node.split.second)];
}

/** The area drawn at the tree's top-right corner: the right of a row, the top of a column. */
export function topRightArea<I extends AreaItem>(node: AreaNode<I>): Area<I> {
  if ("area" in node) return node.area;
  return topRightArea(node.split.axis === "row" ? node.split.second : node.split.first);
}

export function findArea<I extends AreaItem>(root: AreaNode<I>, areaId: string): Area<I> | null {
  return areasOf(root).find((area) => area.id === areaId) ?? null;
}

export type LocatedItem<I extends AreaItem> = { area: Area<I>; display: I; index: number };

export function locateDisplay<I extends AreaItem>(root: AreaNode<I>, displayId: string): LocatedItem<I> | null {
  for (const area of areasOf(root)) {
    const index = area.displays.findIndex((display) => display.id === displayId);
    const display = area.displays[index];
    if (display) return { area, display, index };
  }
  return null;
}

/** The active area's active display: what the operator last worked in. */
export function activeDisplay<I extends AreaItem>(layout: AreaLayout<I>): LocatedItem<I> | null {
  const area = findArea(layout.root, layout.active_area);
  if (!area?.active) return null;
  return locateDisplay(layout.root, area.active);
}

/** The display each area shows, one per area. */
export function shownDisplays<I extends AreaItem>(root: AreaNode<I>): I[] {
  return areasOf(root).flatMap((area) => area.displays.filter((display) => display.id === area.active));
}

/** How many splits lie between the root and an area (contract 2's depth). */
export function areaDepth<I extends AreaItem>(root: AreaNode<I>, areaId: string): number | null {
  if ("area" in root) return root.area.id === areaId ? 0 : null;
  for (const child of [root.split.first, root.split.second]) {
    const depth = areaDepth(child, areaId);
    if (depth !== null) return depth + 1;
  }
  return null;
}

/** The split an area sits directly in, and on which side of it. */
export function parentSplit<I extends AreaItem>(root: AreaNode<I>, areaId: string): { split: AreaSplit<I>; side: "first" | "second" } | null {
  if ("area" in root) return null;
  const { split } = root;
  for (const side of ["first", "second"] as const) {
    const child = split[side];
    if ("area" in child && child.area.id === areaId) return { split, side };
    const inner = parentSplit(child, areaId);
    if (inner) return inner;
  }
  return null;
}

/** The area after or before `areaId` in tree order, wrapping; null when there is no other. */
export function adjacentInOrder<I extends AreaItem>(root: AreaNode<I>, areaId: string, step: 1 | -1): Area<I> | null {
  const areas = areasOf(root);
  const index = areas.findIndex((area) => area.id === areaId);
  if (index === -1 || areas.length < 2) return null;
  return areas[(index + step + areas.length) % areas.length] ?? null;
}

// --- geometry ----------------------------------------------------------------

/** The smallest a subtree may be drawn: each area at its minimum, a divider between siblings. */
export function minimumSize<I extends AreaItem>(node: AreaNode<I>, sizes: LayoutSizes): { width: number; height: number } {
  if ("area" in node) return { width: sizes.areaMinWidth, height: sizes.areaMinHeight };
  const first = minimumSize(node.split.first, sizes);
  const second = minimumSize(node.split.second, sizes);
  return node.split.axis === "row"
    ? { width: first.width + sizes.divider + second.width, height: Math.max(first.height, second.height) }
    : { width: Math.max(first.width, second.width), height: first.height + sizes.divider + second.height };
}

export type AreaBox = { id: string; rect: Rect; bar: Rect; content: Rect };

export type DividerBox = {
  /** The split's id. */
  id: string;
  axis: "row" | "column";
  /** The divider itself. */
  rect: Rect;
  /** The whole split it divides. */
  span: Rect;
  ratio: number;
  /** The first child's drawn size along the axis, in pixels. */
  first: number;
  /** The first and second subtrees' minimum sizes along the axis. */
  minFirst: number;
  minSecond: number;
};

/**
 * Where everything sits for one body rectangle. `fits` is false when some
 * split cannot give both of its sides their minimum; the page then shows one
 * area at a time (B13, A7). While it fits, each split's first side keeps the
 * stored ratio within both sides' minimums, the way the Agent/View boundary
 * does (S6 B6).
 */
export type Geometry = { areas: AreaBox[]; dividers: DividerBox[]; fits: boolean };

export function areaGeometry<I extends AreaItem>(root: AreaNode<I>, body: Rect, sizes: LayoutSizes): Geometry {
  const areas: AreaBox[] = [];
  const dividers: DividerBox[] = [];
  let fits = true;
  const place = (node: AreaNode<I>, rect: Rect) => {
    if ("area" in node) {
      const bar = Math.min(sizes.tabStrip, rect.height);
      areas.push({
        id: node.area.id,
        rect,
        bar: { x: rect.x, y: rect.y, width: rect.width, height: bar },
        content: { x: rect.x, y: rect.y + bar, width: rect.width, height: rect.height - bar },
      });
      return;
    }
    const { split } = node;
    const row = split.axis === "row";
    const available = Math.max(0, (row ? rect.width : rect.height) - sizes.divider);
    const minFirst = along(minimumSize(split.first, sizes), row);
    const minSecond = along(minimumSize(split.second, sizes), row);
    let first: number;
    if (available < minFirst + minSecond) {
      fits = false;
      first = Math.round(available * split.ratio);
    } else {
      first = Math.round(clamp(split.ratio * available, minFirst, available - minSecond));
    }
    const second = available - first;
    const firstRect = row ? { ...rect, width: first } : { ...rect, height: first };
    const dividerRect = row
      ? { x: rect.x + first, y: rect.y, width: sizes.divider, height: rect.height }
      : { x: rect.x, y: rect.y + first, width: rect.width, height: sizes.divider };
    const secondRect = row
      ? { x: rect.x + first + sizes.divider, y: rect.y, width: second, height: rect.height }
      : { x: rect.x, y: rect.y + first + sizes.divider, width: rect.width, height: second };
    dividers.push({ id: split.id, axis: split.axis, rect: dividerRect, span: rect, ratio: split.ratio, first, minFirst, minSecond });
    place(split.first, firstRect);
    place(split.second, secondRect);
  };
  place(root, body);
  return { areas, dividers, fits };
}

/**
 * The ratio a divider lands at for a first side of `first` pixels: within
 * the core's range and both sides' minimums. When the minimums do not fit
 * the core's range alone applies.
 */
export function ratioForFirst(divider: DividerBox, first: number): number {
  const available = along(divider.span, divider.axis === "row") - along(divider.rect, divider.axis === "row");
  if (available <= 0) return divider.ratio;
  const low = Math.max(RATIO_MIN, divider.minFirst / available);
  const high = Math.min(RATIO_MAX, 1 - divider.minSecond / available);
  const ratio = first / available;
  return low > high ? clamp(ratio, RATIO_MIN, RATIO_MAX) : clamp(ratio, low, high);
}

/** The ratio a divider dragged to `offset` pixels from its split's start stands for. */
export function ratioAtOffset(divider: DividerBox, offset: number): number {
  const thickness = along(divider.rect, divider.axis === "row");
  return ratioForFirst(divider, offset - thickness / 2);
}

/** The ratio one keyboard step moves a divider to, or null when it cannot move that way. */
export function steppedRatio(divider: DividerBox, delta: number): number | null {
  const available = along(divider.span, divider.axis === "row") - along(divider.rect, divider.axis === "row");
  return moved(divider.ratio, ratioForFirst(divider, (divider.ratio + delta) * available));
}

/**
 * The geometry of one area filling the body, for a window too narrow for the
 * whole tree (B13, A7): only the active area shows, and no split fits.
 */
export function singleAreaGeometry<I extends AreaItem>(area: Area<I>, body: Rect, sizes: LayoutSizes): Geometry {
  return { ...areaGeometry({ area }, body, sizes), fits: false };
}

// --- directions --------------------------------------------------------------

/** Only the topology matters for a neighbour, so it is measured on a fixed square with no minimums. */
const TOPOLOGY_SIZES: LayoutSizes = { areaMinWidth: 0, areaMinHeight: 0, divider: 0, tabStrip: 0 };
const TOPOLOGY_BODY: Rect = { x: 0, y: 0, width: 1024, height: 1024 };

/**
 * The area that lies directly in `direction` from `areaId`: it shares that
 * edge and overlaps it across; the widest overlap wins, then the nearest
 * centre, then tree order. Null when that edge is the Views' own edge.
 */
export function neighbourArea<I extends AreaItem>(root: AreaNode<I>, areaId: string, direction: Edge): string | null {
  const boxes = areaGeometry(root, TOPOLOGY_BODY, TOPOLOGY_SIZES).areas;
  const from = boxes.find((box) => box.id === areaId)?.rect;
  if (!from) return null;
  const across = direction === "left" || direction === "right";
  let best: { id: string; overlap: number; distance: number } | null = null;
  for (const box of boxes) {
    if (box.id === areaId || !touches(from, box.rect, direction)) continue;
    const overlap = across
      ? Math.min(from.y + from.height, box.rect.y + box.rect.height) - Math.max(from.y, box.rect.y)
      : Math.min(from.x + from.width, box.rect.x + box.rect.width) - Math.max(from.x, box.rect.x);
    if (overlap <= 0) continue;
    const distance = across
      ? Math.abs(from.y + from.height / 2 - (box.rect.y + box.rect.height / 2))
      : Math.abs(from.x + from.width / 2 - (box.rect.x + box.rect.width / 2));
    if (!best || overlap > best.overlap || (overlap === best.overlap && distance < best.distance)) {
      best = { id: box.id, overlap, distance };
    }
  }
  return best?.id ?? null;
}

function touches(from: Rect, to: Rect, direction: Edge): boolean {
  switch (direction) {
    case "right":
      return to.x === from.x + from.width;
    case "left":
      return to.x + to.width === from.x;
    case "down":
      return to.y === from.y + from.height;
    case "up":
      return to.y + to.height === from.y;
  }
}

/**
 * The scroll position of a tab strip that shows the tab at `offset` (its
 * left edge within the strip's content) with `width`: unchanged when the
 * tab is already whole in the `viewport`, else the least move that shows
 * it, its left edge first when it cannot fit whole.
 */
export function revealedScroll(scroll: number, viewport: number, offset: number, width: number): number {
  if (offset < scroll || width >= viewport) return offset;
  if (offset + width > scroll + viewport) return offset + width - viewport;
  return scroll;
}

/**
 * How a tab draws its contents in its slot: `titled` with the title at full
 * padding, `compact` with narrow padding and a truncated title (an unselected
 * one without its close control), `marks` with its marks alone (a selected
 * one adding its close control).
 */
export type TabFit = "titled" | "compact" | "marks";
export type TabSlotFit = { width: number; fit: TabFit };
/** The width and fit of the selected tab and of every other tab in one strip. */
export type TabStripFit = { selected: TabSlotFit; others: TabSlotFit };

/** The pixel sizes the strip's rules read, each from its design token. */
export type TabStripSizes = {
  /** `--size-tab-preferred`: what every tab asks for. */
  preferred: number;
  /** `--size-tab-title-min`: the narrowest a full titled tab is. */
  titleMin: number;
  /** `--size-tab-icon-identity`: a tab's marks alone. */
  icon: number;
  /** `--size-control-sm`: the close control. */
  control: number;
};

/**
 * How an Agent or View tab strip shares `room` among `count` tabs, the way a browser's
 * tabs do: in three continuous stages, so the bar is used to its end and the
 * selected tab keeps its title longest.
 *
 * 1. While an equal share holds the title minimum, every tab takes
 *    `min(preferred, share)` with its title.
 * 2. Below that the selected tab keeps the title minimum and the others split
 *    what is left, shrinking alike to the icon identity: compact while they
 *    still hold the marks and one control's worth of title, marks below.
 * 3. Once every other tab is a mark, the selected tab gives up the rest:
 *    compact with its close control while it holds the marks, two controls
 *    and so one control's worth of title, then marks with its close control,
 *    and the strip scrolls only once even those overflow.
 *
 * The same room and count always give the same answer, whichever way the
 * room changed. With no tab of this strip selected (a delegated canvas shows
 * instead) every tab follows the others' rule on an equal share.
 */
export function tabStripFit(room: number, count: number, hasSelected: boolean, sizes: TabStripSizes): TabStripFit {
  const { preferred, titleMin, icon, control } = sizes;
  const other = (width: number): TabSlotFit => {
    if (width >= titleMin) return { width: Math.min(preferred, width), fit: "titled" };
    if (width >= icon + control) return { width, fit: "compact" };
    return { width: Math.max(icon, width), fit: "marks" };
  };
  const share = count > 0 ? room / count : preferred;
  if (share >= titleMin || !hasSelected) {
    const slot = other(share);
    return { selected: slot, others: slot };
  }
  const rest = count - 1;
  if (room >= titleMin + rest * icon) {
    return { selected: { width: titleMin, fit: "titled" }, others: other((room - titleMin) / rest) };
  }
  const selected = room - rest * icon;
  return {
    selected: selected >= icon + 2 * control ? { width: selected, fit: "compact" } : { width: icon + control, fit: "marks" },
    others: { width: icon, fit: "marks" },
  };
}

export type Eligibility = { ok: true } | { ok: false; reason: string };

const refuse = (reason: string): Eligibility => ({ ok: false, reason });

/**
 * Whether moving `displayId` into a new area at `edge` of `areaId` can land
 * (B9, B19, contract 2 and 6): the area and depth caps the core publishes,
 * the target area holding two minimums along the axis plus a divider, and a
 * split that would change nothing - a display alone in its own area - is
 * refused. The caps are counted before an emptied source area collapses,
 * which is the stricter reading of the core's rule.
 */
export function splitEligibility<I extends AreaItem>(
  layout: AreaLayout<I>,
  geometry: Geometry,
  sizes: LayoutSizes,
  displayId: string,
  areaId: string,
  edge: Edge,
  words: AreaWords,
): Eligibility {
  const located = locateDisplay(layout.root, displayId);
  if (!located) return refuse(areaSentence(words, "itemGone"));
  const target = findArea(layout.root, areaId);
  if (!target) return refuse(areaSentence(words, "areaGone"));
  if (located.area.id === areaId && located.area.displays.length === 1) return refuse(areaSentence(words, "onlyItem"));
  return roomToSplit(layout, geometry, sizes, areaId, edge, words);
}



/** The caps, the depth and the pixel room a new area at `edge` of `areaId` needs. */
export function roomToSplit<I extends AreaItem>(layout: AreaLayout<I>, geometry: Geometry, sizes: LayoutSizes, areaId: string, edge: Edge, words: AreaWords): Eligibility {
  if (areasOf(layout.root).length >= layout.limits.areas) return refuse(translate(`panes.area.maxAreas.${words.kind}` as const, { count: layout.limits.areas }));
  if ((areaDepth(layout.root, areaId) ?? 0) >= layout.limits.depth) return refuse(translate(`panes.area.maxDepth.${words.kind}` as const, { count: layout.limits.depth }));
  if (!geometry.fits) return refuse(areaSentence(words, "tooSmall"));
  const box = geometry.areas.find((area) => area.id === areaId);
  if (!box) return refuse(areaSentence(words, "notOnScreen"));
  const across = edge === "left" || edge === "right";
  const needed = across ? 2 * sizes.areaMinWidth + sizes.divider : 2 * sizes.areaMinHeight + sizes.divider;
  if ((across ? box.rect.width : box.rect.height) < needed) {
    return refuse(areaSentence(words, across ? "tooNarrow" : "tooShort"));
  }
  return { ok: true };
}

// --- dropping a dragged tab --------------------------------------------------

export type TabSlot = { displayId: string; rect: Rect };

export type DropTarget =
  /** A thin insertion line in a tab bar: reorder there, or move into that area (B6). `index` is the display's final position. */
  | { kind: "bar"; areaId: string; index: number; line: { x: number; y: number; height: number } }
  /** An area's content edge: a new area there takes half of it (B7). */
  | { kind: "edge"; areaId: string; edge: Edge; preview: Rect; label: string }
  /** Nothing lands here; a reason means the place is refused and the cursor says so (B8). */
  | { kind: "none"; reason: string | null };



/**
 * Where a dragged display would land at `point` (B6-B8, D-04, D-05): over a
 * tab bar, a thin insertion line between tabs; over an area's content, the
 * edge nearest the pointer when it is within `EDGE_ZONE` of that edge, and
 * only when that split can land. One target wins at a time. A drop back into
 * its own slot changes nothing and targets nothing. `tabs` are each area's
 * tab rectangles in the same coordinates as `geometry`.
 */
export function dropTarget<I extends AreaItem>(input: {
  layout: AreaLayout<I>;
  geometry: Geometry;
  sizes: LayoutSizes;
  tabs: Record<string, TabSlot[]>;
  displayId: string;
  point: Point;
  words: AreaWords;
  sameContent: (a: I, b: I) => boolean;
}): DropTarget {
  const { layout, geometry, sizes, tabs, displayId, point, words, sameContent } = input;
  const nowhere = areaSentence(words, "dropHint");
  const located = locateDisplay(layout.root, displayId);
  if (!located) return { kind: "none", reason: areaSentence(words, "itemGone") };
  for (const box of geometry.areas) {
    if (contains(box.bar, point)) return barTarget(box, findArea(layout.root, box.id), tabs[box.id] ?? [], located, point, sameContent);
  }
  for (const box of geometry.areas) {
    if (!contains(box.content, point)) continue;
    const edge = nearestEdge(box.content, point);
    if (!edge) return { kind: "none", reason: nowhere };
    const eligibility = splitEligibility(layout, geometry, sizes, displayId, box.id, edge, words);
    if (!eligibility.ok) return { kind: "none", reason: eligibility.reason };
    return { kind: "edge", areaId: box.id, edge, preview: half(box.rect, edge), label: splitLabel(edge) };
  }
  return { kind: "none", reason: nowhere };
}

function barTarget<I extends AreaItem>(box: AreaBox, area: Area<I> | null, slots: TabSlot[], located: LocatedItem<I>, point: Point, sameContent: (a: I, b: I) => boolean): DropTarget {
  const slot = slots.filter((tab) => tab.rect.x + tab.rect.width / 2 < point.x).length;
  const at = slots[slot]?.rect;
  const last = slots.at(-1)?.rect;
  const x = at ? at.x : last ? last.x + last.width : box.bar.x;
  const line = { x, y: box.bar.y, height: box.bar.height };
  if (located.area.id !== box.id) {
    // A view of this document already in that area gives way to the moved
    // one, so one left of the line leaves the final place a slot earlier.
    const twin = area?.displays.find((other) => sameContent(other, located.display));
    const twinSlot = twin ? slots.findIndex((tab) => tab.displayId === twin.id) : -1;
    return { kind: "bar", areaId: box.id, index: twinSlot !== -1 && twinSlot < slot ? slot - 1 : slot, line };
  }
  const from = slots.findIndex((tab) => tab.displayId === located.display.id);
  const index = from !== -1 && slot > from ? slot - 1 : slot;
  if (from === -1 || index === from) return { kind: "none", reason: null };
  return { kind: "bar", areaId: box.id, index, line };
}

/** The edge `point` is nearest to, when it is within the edge zone; ties go right, left, up, down. */
function nearestEdge(rect: Rect, point: Point): Edge | null {
  if (rect.width <= 0 || rect.height <= 0) return null;
  const distances: [Edge, number][] = [
    ["right", (rect.x + rect.width - point.x) / rect.width],
    ["left", (point.x - rect.x) / rect.width],
    ["up", (point.y - rect.y) / rect.height],
    ["down", (rect.y + rect.height - point.y) / rect.height],
  ];
  let best: [Edge, number] | null = null;
  for (const entry of distances) {
    if (entry[1] < EDGE_ZONE && (!best || entry[1] < best[1])) best = entry;
  }
  return best ? best[0] : null;
}

/** The half of `rect` a new area at `edge` would take. */
function half(rect: Rect, edge: Edge): Rect {
  switch (edge) {
    case "right":
      return { x: rect.x + rect.width / 2, y: rect.y, width: rect.width / 2, height: rect.height };
    case "left":
      return { x: rect.x, y: rect.y, width: rect.width / 2, height: rect.height };
    case "down":
      return { x: rect.x, y: rect.y + rect.height / 2, width: rect.width, height: rect.height / 2 };
    case "up":
      return { x: rect.x, y: rect.y, width: rect.width, height: rect.height / 2 };
  }
}

/** Whether two targets are the same place, so a drop lands only where the preview said it would (D-05). */
export function sameTarget(a: DropTarget, b: DropTarget): boolean {
  if (a.kind === "bar" && b.kind === "bar") return a.areaId === b.areaId && a.index === b.index;
  if (a.kind === "edge" && b.kind === "edge") return a.areaId === b.areaId && a.edge === b.edge;
  return false;
}

export function resizeTarget<I extends AreaItem>(layout: AreaLayout<I>, geometry: Geometry | null, grow: boolean, words: AreaWords): { splitId: string; ratio: number } | { reason: string } {
  const parent = parentSplit(layout.root, layout.active_area);
  if (!parent) return { reason: areaSentence(words, "onlyOne") };
  const delta = (grow ? 1 : -1) * (parent.side === "first" ? RESIZE_STEP : -RESIZE_STEP);
  const divider = geometry?.dividers.find((box) => box.id === parent.split.id);
  const ratio = divider ? steppedRatio(divider, delta) : moved(parent.split.ratio, clamp(parent.split.ratio + delta, RATIO_MIN, RATIO_MAX));
  if (ratio === null) return { reason: areaSentence(words, grow ? "cannotGrow" : "cannotShrink") };
  return { splitId: parent.split.id, ratio };
}

function moved(before: number, after: number): number | null {
  return Math.abs(after - before) < 1e-6 ? null : after;
}

function contains(rect: Rect, point: Point): boolean {
  return point.x >= rect.x && point.x < rect.x + rect.width && point.y >= rect.y && point.y < rect.y + rect.height;
}

function along(size: { width: number; height: number }, row: boolean): number {
  return row ? size.width : size.height;
}

function clamp(value: number, low: number, high: number): number {
  return Math.min(Math.max(value, low), high);
}
