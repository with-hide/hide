// What the Overview work map draws between sessions (PRD overview-main-map
// D-03, D-04, D-06, D-17) and the geometry of its lines, team outlines and
// project glows. Pure, so the rules are tested apart from React Flow.

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

/** Delegation lines: always drawn faint as each team's skeleton, brightened for the chosen team. */
export function lineageLines(rows: readonly string[], parents: ReadonlyMap<string, string | null>, teamOf: (id: string) => string, chosenTeam: string | null) {
  return rows.flatMap((id) => { const parent = parents.get(id); return parent ? [{ parent, child: id, emphasized: chosenTeam !== null && teamOf(id) === chosenTeam }] : []; });
}

/** The accent choice a project's glow and name take, by a stable hash of its id (PRD D-17); the stylesheet maps it to its token. */
const PROJECT_ACCENTS = ["sky", "violet", "lime", "amber"] as const;
export type ProjectAccent = (typeof PROJECT_ACCENTS)[number];
export function projectAccent(id: string): ProjectAccent {
  let hash = 2166136261;
  for (let index = 0; index < id.length; index += 1) hash = Math.imul(hash ^ id.charCodeAt(index), 16777619);
  return PROJECT_ACCENTS[(hash >>> 0) % PROJECT_ACCENTS.length]!;
}

/** The middle of a project's sessions and how far its glow reaches past the farthest one. */
export function cloud(points: readonly Point[], reach: number): Point & { r: number } {
  const x = points.reduce((total, point) => total + point.x, 0) / points.length;
  const y = points.reduce((total, point) => total + point.y, 0) / points.length;
  return { x, y, r: Math.max(...points.map((point) => Math.hypot(point.x - x, point.y - y))) + reach };
}
