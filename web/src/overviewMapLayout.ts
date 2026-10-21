// Places the Overview work map (PRD overview-main-map D-01, D-02, D-13): the
// operator at the center, every session without a placed parent on one ring
// around the operator, grouped by project, and each team gathered outside its
// root: children on a fan around their parent facing away from it, the way the
// parent faces away from the operator. The same sessions in the same order
// always land in the same places; status, letters and selection are not inputs.
// Each session is a label rectangle whose orb sits at its top center; no two
// rectangles, the operator's included, overlap.

import { LAYOUT_NODE_LIMIT } from "./dependencyLayout";

/** A label rectangle with its orb at the top center: `orb` is the orb's diameter. */
export type MapBox = { width: number; height: number; orb: number };
/** `parent` names a placed session, or null for a session on the first ring. */
export type MapSession = { id: string; projectId: string; parent: string | null; box: MapBox };
export type MapInput = {
  /** Projects in drawing order; a session's project must be listed. */
  projects: readonly { id: string }[];
  /** Sessions in drawing order within their project and parent. */
  sessions: readonly MapSession[];
  operator: number;
  gap: number;
  projectGap: number;
};
/** An orb center, its ring and angle, and the first-ring session whose team it belongs to. */
export type MapPoint = { x: number; y: number; level: number; angle: number; team: string };
export type MapPlacement = { nodes: Map<string, MapPoint> };

type Tree = { session: MapSession; level: number; children: Tree[] };
/** A member of a team, its orb center relative to the team's root. */
type Member = { tree: Tree; x: number; y: number };

const TURN = Math.PI * 2;
const TOP = -Math.PI / 2;
/** Children spread at most a quarter turn to either side of their parent's outward direction. */
const FAN = Math.PI / 2;

/** Half extents of a box measured from its orb center, symmetric so a distance can be solved for every direction. */
const half = (box: MapBox) => ({ w: box.width / 2, h: Math.max(box.orb / 2, box.height - box.orb / 2) });

/** The distance along (dx, dy) at which two half-extent boxes plus a gap stop overlapping. */
function clearance(dx: number, dy: number, a: { w: number; h: number }, b: { w: number; h: number }, gap: number): number {
  return Math.min((a.w + b.w + gap) / Math.max(Math.abs(dx), Number.EPSILON), (a.h + b.h + gap) / Math.max(Math.abs(dy), Number.EPSILON));
}

/** The rectangle a box covers when its orb center is at (x, y). */
const rect = (x: number, y: number, box: MapBox) => ({ left: x - box.width / 2, right: x + box.width / 2, top: y - box.orb / 2, bottom: y - box.orb / 2 + box.height });
type Rect = ReturnType<typeof rect>;
const overlaps = (a: Rect, b: Rect, gap: number) => a.left < b.right + gap && b.left < a.right + gap && a.top < b.bottom + gap && b.top < a.bottom + gap;

/** Whether no member of one group touches a member of another; members of one group are already apart. */
function apart(groups: readonly (readonly Member[])[], gap: number): boolean {
  const boxes = groups.map((group) => group.map((member) => rect(member.x, member.y, member.tree.session.box)));
  for (let i = 0; i < boxes.length; i += 1) for (let j = i + 1; j < boxes.length; j += 1) {
    for (const a of boxes[i]!) for (const b of boxes[j]!) if (overlaps(a, b, gap / 2)) return false;
  }
  return true;
}

const shift = (members: readonly Member[], x: number, y: number) => members.map((member) => ({ tree: member.tree, x: member.x + x, y: member.y + y }));

/**
 * A subtree with its root at the origin facing `direction`: the children sit on
 * one arc around it, as close as the arc can be without any two members touching.
 */
function arrange(tree: Tree, direction: number, gap: number): Member[] {
  const own: Member[] = [{ tree, x: 0, y: 0 }];
  const count = tree.children.length;
  if (!count) return own;
  const widest = tree.children.reduce((box, child) => { const size = half(child.session.box); return { w: Math.max(box.w, size.w), h: Math.max(box.h, size.h) }; }, { w: 0, h: 0 });
  const near = clearance(Math.cos(direction), Math.sin(direction), half(tree.session.box), widest, gap);
  const step = count > 1 ? Math.min((2 * FAN) / (count - 1), (widest.w * 2 + gap) / near) : 0;
  const subtrees = tree.children.map((child, index) => { const angle = direction + (index - (count - 1) / 2) * step; return { angle, members: arrange(child, angle, gap) }; });
  const at = (radius: number) => [own, ...subtrees.map((subtree) => shift(subtree.members, Math.cos(subtree.angle) * radius, Math.sin(subtree.angle) * radius))];
  // Grow the arc until it fits, then close in on the nearest radius that still does.
  let low = near;
  let high = near;
  for (let attempt = 0; !apart(at(high), gap); attempt += 1) {
    if (attempt > 60) throw new Error(`overview map could not fit the team of ${tree.session.id}`);
    low = high;
    high *= 1.25;
  }
  for (let attempt = 0; high - low > gap / 2 && attempt < 30; attempt += 1) {
    const middle = (low + high) / 2;
    if (apart(at(middle), gap)) high = middle; else low = middle;
  }
  return at(high).flat();
}

/** How far a team reaches to either side of its root, across the direction it faces. */
function sideways(members: readonly Member[], direction: number): { lo: number; hi: number } {
  const across = { x: -Math.sin(direction), y: Math.cos(direction) };
  let lo = 0;
  let hi = 0;
  for (const member of members) {
    const box = rect(member.x, member.y, member.tree.session.box);
    for (const [x, y] of [[box.left, box.top], [box.right, box.top], [box.left, box.bottom], [box.right, box.bottom]] as const) {
      const offset = x * across.x + y * across.y;
      lo = Math.min(lo, offset);
      hi = Math.max(hi, offset);
    }
  }
  return { lo, hi };
}

export function placeOverviewMap(input: MapInput): MapPlacement {
  const count = input.projects.length + input.sessions.length;
  if (count > LAYOUT_NODE_LIMIT) throw new Error(`overview map has ${count} nodes, over the layout limit of ${LAYOUT_NODE_LIMIT}`);
  const projectOrder = new Map(input.projects.map((project, index) => [project.id, index]));
  for (const session of input.sessions) if (!projectOrder.has(session.projectId)) throw new Error(`session ${session.id} names an unlisted project`);
  const byId = new Map(input.sessions.map((session) => [session.id, session]));
  const kids = new Map<string, MapSession[]>();
  for (const session of input.sessions) {
    if (session.parent === null) continue;
    if (!byId.has(session.parent)) throw new Error(`session ${session.id} names an unplaced parent`);
    kids.set(session.parent, [...(kids.get(session.parent) ?? []), session]);
  }
  // A lineage cycle has no ring member; its earliest session starts it so
  // every session is placed exactly once.
  const placed = new Set<string>();
  const grow = (session: MapSession, level: number): Tree => {
    placed.add(session.id);
    const children = (kids.get(session.id) ?? []).filter((child) => !placed.has(child.id)).map((child) => grow(child, level + 1));
    return { session, level, children };
  };
  const teams: Tree[] = [];
  for (const session of input.sessions) if (session.parent === null) teams.push(grow(session, 1));
  for (const session of input.sessions) if (!placed.has(session.id)) teams.push(grow(session, 1));
  teams.sort((a, b) => projectOrder.get(a.session.projectId)! - projectOrder.get(b.session.projectId)!);
  const operator: MapBox = { width: input.operator, height: input.operator, orb: input.operator };
  const boundary = (index: number) => teams.length > 1 && teams[index]!.session.projectId !== teams[(index + 1) % teams.length]!.session.projectId;
  const between = (index: number) => (teams.length > 1 ? (boundary(index) ? input.projectGap : input.gap) : 0);
  // A team's shape depends on the direction it faces, and its direction on the
  // room the teams before it take; two rounds settle both.
  let angles = teams.map((_, index) => TOP + (TURN * index) / Math.max(teams.length, 1));
  let shapes = teams.map((tree, index) => arrange(tree, angles[index]!, input.gap));
  let radius = 0;
  for (let round = 0; round < 2; round += 1) {
    const reach = shapes.map((members, index) => sideways(members, angles[index]!));
    const room = reach.reduce((total, side, index) => total + side.hi - side.lo + between(index), 0);
    const clear = teams.reduce((far, tree, index) => Math.max(far, clearance(Math.cos(angles[index]!), Math.sin(angles[index]!), half(operator), half(tree.session.box), input.gap)), 0);
    radius = Math.max(clear, room / TURN);
    // Spread what is left between teams, twice as much where projects meet.
    const weights = teams.map((_, index) => (boundary(index) ? 2 : 1));
    const weight = weights.reduce((total, value) => total + value, 0);
    const spare = Math.max(0, TURN - room / radius);
    let cursor = TOP + (reach[0]?.lo ?? 0) / radius;
    angles = teams.map((_, index) => {
      const angle = cursor - reach[index]!.lo / radius;
      cursor = angle + (reach[index]!.hi + between(index)) / radius + (spare * weights[index]!) / weight;
      return angle;
    });
    shapes = teams.map((tree, index) => arrange(tree, angles[index]!, input.gap));
  }
  // A last check over every team: while one still touches another or the
  // operator, the ring moves out, keeping every angle and every team's shape.
  const at = (ring: number) => [[{ tree: { session: { id: "", projectId: "", parent: null, box: operator }, level: 0, children: [] }, x: 0, y: 0 }], ...shapes.map((members, index) => shift(members, Math.cos(angles[index]!) * ring, Math.sin(angles[index]!) * ring))];
  for (let attempt = 0; teams.length && !apart(at(radius), input.gap); attempt += 1) {
    if (attempt >= 60) throw new Error("overview map could not separate its teams");
    radius += Math.max(input.gap, radius * 0.1);
  }
  const nodes = new Map<string, MapPoint>();
  at(radius).slice(1).forEach((members, index) => {
    for (const member of members) nodes.set(member.tree.session.id, { x: member.x, y: member.y, level: member.tree.level, angle: Math.atan2(member.y, member.x), team: teams[index]!.session.id });
  });
  return { nodes };
}
