// What the Overview work map draws between sessions (PRD overview-main-map
// D-03, D-04, D-06) and the geometry of its lines and team outlines. Pure, so
// the rules are tested apart from React Flow.

import type { Communication } from "./overviewGraphModel";

type Point = { x: number; y: number };
/** An orb center and radius in flow coordinates, where every line starts and ends. */
export type Orb = Point & { r: number };
/** A drawn session's whole rectangle, orb and words. */
export type Area = { left: number; top: number; right: number; bottom: number };

/** The segment between two orbs, cut back so it starts and ends at their edges (and leaves its arrow room). */
export function between(from: Orb, to: Orb, clearance: number): { x1: number; y1: number; x2: number; y2: number } | null {
  const length = Math.hypot(to.x - from.x, to.y - from.y);
  if (length <= from.r + to.r + clearance * 2) return null;
  const ux = (to.x - from.x) / length;
  const uy = (to.y - from.y) / length;
  return { x1: from.x + ux * (from.r + clearance), y1: from.y + uy * (from.r + clearance), x2: to.x - ux * (to.r + clearance), y2: to.y - uy * (to.r + clearance) };
}

/** The convex hull of the points, counterclockwise in the drawing's own axes (monotone chain). */
export function hull(points: readonly Point[]): Point[] {
  const sorted = [...points].sort((a, b) => a.x - b.x || a.y - b.y);
  if (sorted.length < 3) return sorted;
  const cross = (o: Point, a: Point, b: Point) => (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
  const lower: Point[] = [];
  for (const point of sorted) { while (lower.length >= 2 && cross(lower.at(-2)!, lower.at(-1)!, point) <= 0) lower.pop(); lower.push(point); }
  const upper: Point[] = [];
  for (const point of [...sorted].reverse()) { while (upper.length >= 2 && cross(upper.at(-2)!, upper.at(-1)!, point) <= 0) upper.pop(); upper.push(point); }
  return [...lower.slice(0, -1), ...upper.slice(0, -1)];
}

/** A rounded outline `pad` outside every member's rectangle, as a path relative to its own top left, with that box. */
export function teamOutline(members: readonly Area[], pad: number): { path: string; left: number; top: number; width: number; height: number } {
  const left = Math.min(...members.map((area) => area.left)) - pad;
  const top = Math.min(...members.map((area) => area.top)) - pad;
  const width = Math.max(...members.map((area) => area.right)) + pad - left;
  const height = Math.max(...members.map((area) => area.bottom)) + pad - top;
  const shape = hull(members.flatMap((area) => [{ x: area.left, y: area.top }, { x: area.right, y: area.top }, { x: area.right, y: area.bottom }, { x: area.left, y: area.bottom }])).map((point) => ({ x: point.x - left, y: point.y - top }));
  const normals = shape.map((point, index) => {
    const next = shape[(index + 1) % shape.length]!;
    const length = Math.hypot(next.x - point.x, next.y - point.y) || 1;
    return { x: (next.y - point.y) / length, y: -(next.x - point.x) / length };
  });
  const at = (index: number, normal: Point) => `${shape[index]!.x + normal.x * pad},${shape[index]!.y + normal.y * pad}`;
  let path = `M ${at(0, normals.at(-1)!)}`;
  shape.forEach((_, index) => {
    path += ` A ${pad},${pad} 0 0 1 ${at(index, normals[index]!)}`;
    path += ` L ${at((index + 1) % shape.length, normals[index]!)}`;
  });
  return { path: `${path} Z`, left, top, width, height };
}

export type LineState = {
  /** The chosen session, and the lineage team it belongs to. */
  selection: string | null;
  lineageTeam: string | null;
  /** The session under the pointer or keyboard focus. */
  pointed: string | null;
  /** Each session's lineage team. */
  teams: ReadonlyMap<string, string>;
};

/** Communication lines: across teams always, inside a team only while that team is chosen. */
export function communicationLines(pairs: readonly Communication[], drawn: (id: string) => boolean, state: LineState) {
  return pairs.flatMap((pair) => {
    const team = state.teams.get(pair.source);
    const across = team !== state.teams.get(pair.target);
    if (!drawn(pair.source) || !drawn(pair.target) || !across && team !== state.lineageTeam) return [];
    const touches = (id: string | null) => id !== null && (pair.source === id || pair.target === id);
    const active = touches(state.selection) || touches(state.pointed) || !across;
    const muted = state.selection !== null && !active && team !== state.lineageTeam && state.teams.get(pair.target) !== state.lineageTeam;
    return [{ pair, active, muted }];
  });
}

/** Turn arrows: only a root whose turn it is points at the operator; outside the chosen team it is muted. */
export function turnLines(rows: readonly { id: string; turn: boolean }[], roots: ReadonlySet<string>, state: Pick<LineState, "selection" | "lineageTeam" | "teams">) {
  return rows.flatMap((row) => (roots.has(row.id) && row.turn ? [{ id: row.id, muted: state.selection !== null && state.teams.get(row.id) !== state.lineageTeam }] : []));
}

/** Delegation lines: drawn only inside the chosen team; the dashed outline says the rest. */
export function lineageLines(rows: readonly string[], parents: ReadonlyMap<string, string | null>, teamOf: (id: string) => string, chosenTeam: string | null) {
  if (!chosenTeam) return [];
  return rows.flatMap((id) => { const parent = parents.get(id); return parent && teamOf(id) === chosenTeam ? [{ parent, child: id }] : []; });
}

/** The widest gap between sorted angles around the turn, and the angle that ends it. */
function largestGap(angles: readonly number[]): { size: number; end: number } {
  let gap = { size: angles[0]! + Math.PI * 2 - angles.at(-1)!, end: angles[0]! };
  for (let index = 1; index < angles.length; index += 1) if (angles[index]! - angles[index - 1]! > gap.size) gap = { size: angles[index]! - angles[index - 1]!, end: angles[index]! };
  return gap;
}

/**
 * A project's area: the band of the ring around the operator (at the origin)
 * that its sessions cover, `pad` beyond each of them and never nearer than
 * `inside`. Its outer edge steps out only where a session reaches further, so a
 * team does not widen the whole band. The name's center sits just outside the
 * band's outer edge at the middle of its angles. Coordinates are relative to
 * the returned box, which holds the band and the name.
 */
export function projectArea(members: readonly Area[], pad: number, inside: number, name: { width: number; height: number }) {
  const TURN = Math.PI * 2;
  let near = Number.POSITIVE_INFINITY;
  // Each session covers the turn between its outermost corners, as seen from the operator, out to its farthest corner.
  const spans: { from: number; to: number; far: number }[] = [];
  for (const area of members) {
    near = Math.min(near, Math.hypot(Math.max(area.left, Math.min(0, area.right)), Math.max(area.top, Math.min(0, area.bottom))));
    let far = 0;
    const corners = ([[area.left, area.top], [area.right, area.top], [area.left, area.bottom], [area.right, area.bottom]] as const).map(([x, y]) => { far = Math.max(far, Math.hypot(x, y)); return Math.atan2(y, x); }).sort((a, b) => a - b);
    const open = largestGap(corners);
    spans.push({ from: open.end, to: open.end + TURN - open.size, far: far + pad });
  }
  const inner = Math.max(inside, near - pad);
  const widen = pad / inner;
  // The sessions cover everything but the widest stretch of the turn none of them reaches.
  spans.sort((a, b) => a.from - b.from);
  let empty = 0;
  let start = spans[0]!.from;
  let covered = spans[0]!.to;
  for (const span of [...spans.slice(1), ...spans.map((span) => ({ ...span, from: span.from + TURN, to: span.to + TURN }))]) {
    if (span.from - covered > empty) { empty = span.from - covered; start = span.from; }
    covered = Math.max(covered, span.to);
  }
  const sweep = Math.min(TURN, TURN - empty + 2 * widen);
  start -= widen;
  const end = start + sweep;
  const middle = start + sweep / 2;
  // The outer edge at each angle: the farthest session covering it, or the nearest session's reach.
  const floor = Math.min(...spans.map((span) => span.far));
  const covering = spans.flatMap((span) => [-1, 0, 1].map((turns) => ({ from: span.from + turns * TURN - widen, to: span.to + turns * TURN + widen, far: span.far })));
  const outerAt = (angle: number) => covering.reduce((far, span) => (span.from <= angle && angle <= span.to ? Math.max(far, span.far) : far), floor);
  const breaks = [...new Set([start, end, ...covering.flatMap((span) => [span.from, span.to]).filter((angle) => angle > start && angle < end)])].sort((a, b) => a - b);
  const steps = breaks.slice(0, -1).map((from, index) => ({ from, to: breaks[index + 1]!, far: outerAt((from + breaks[index + 1]!) / 2) }));
  const point = (radius: number, angle: number) => ({ x: Math.cos(angle) * radius, y: Math.sin(angle) * radius });
  // Arcs are drawn a quarter turn at most, so a whole ring and a long band need no large-arc flag.
  const arc = (radius: number, from: number, to: number) => {
    const pieces = Math.max(1, Math.ceil(Math.abs(to - from) / (Math.PI / 2)));
    return Array.from({ length: pieces }, (_, index) => ({ radius, at: point(radius, from + ((to - from) * (index + 1)) / pieces), sweep: to > from ? 1 : 0 }));
  };
  const outline: { radius: number; at: { x: number; y: number }; sweep: number }[] = [];
  for (const step of steps) outline.push({ radius: 0, at: point(step.far, step.from), sweep: 0 }, ...arc(step.far, step.from, step.to));
  outline.push({ radius: 0, at: point(inner, end), sweep: 0 }, ...arc(inner, end, start));
  const direction = { x: Math.cos(middle), y: Math.sin(middle) };
  const reach = Math.min(name.width / 2 / Math.max(Math.abs(direction.x), Number.EPSILON), name.height / 2 / Math.max(Math.abs(direction.y), Number.EPSILON));
  const label = point(outerAt(middle) + reach + pad / 2, middle);
  // Sample every arc finely enough that the box holds the band, then add the name.
  const samples = steps.flatMap((step) => Array.from({ length: 17 }, (_, index) => point(step.far, step.from + ((step.to - step.from) * index) / 16)));
  const points = [point(inner, start), point(inner, end), ...Array.from({ length: 33 }, (_, index) => point(inner, start + (sweep * index) / 32)), ...samples, { x: label.x - name.width / 2, y: label.y - name.height / 2 }, { x: label.x + name.width / 2, y: label.y + name.height / 2 }];
  const left = Math.min(...points.map((p) => p.x));
  const top = Math.min(...points.map((p) => p.y));
  const width = Math.max(...points.map((p) => p.x)) - left;
  const height = Math.max(...points.map((p) => p.y)) - top;
  const at = (p: { x: number; y: number }) => `${p.x - left},${p.y - top}`;
  const first = point(inner, start);
  const path = `M ${at(first)} ${outline.map((step) => (step.radius ? `A ${step.radius},${step.radius} 0 0 ${step.sweep} ${at(step.at)}` : `L ${at(step.at)}`)).join(" ")} Z`;
  return { path, left, top, width, height, label: { x: label.x - left, y: label.y - top } };
}
