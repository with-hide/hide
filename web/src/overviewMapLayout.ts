// Places the Overview work map (PRD overview-main-map D-01, D-02, D-13): the
// operator at the center, every session without a placed parent on the first
// ring, grouped by project, and each delegated descendant one ring further out
// inside its parent's angle, a radial tree. The same sessions in the same order
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
  projects: readonly { id: string; label: { width: number; height: number } }[];
  /** Sessions in drawing order within their project and parent. */
  sessions: readonly MapSession[];
  operator: number;
  gap: number;
  projectGap: number;
};
/** An orb center, its ring and angle, and the first-ring session whose team it belongs to. */
export type MapPoint = { x: number; y: number; level: number; angle: number; team: string };
export type MapPlacement = { nodes: Map<string, MapPoint>; labels: Map<string, { x: number; y: number }> };

type Tree = { session: MapSession; level: number; children: Tree[]; span: number; angle: number };

const TURN = Math.PI * 2;
const TOP = -Math.PI / 2;

/** Half extents of a box measured from its orb center, symmetric so a ring can be solved for every angle. */
const half = (box: MapBox) => ({ w: box.width / 2, h: Math.max(box.orb / 2, box.height - box.orb / 2) });

/** The distance along (dx, dy) at which two half-extent boxes plus a gap stop overlapping. */
function clearance(dx: number, dy: number, a: { w: number; h: number }, b: { w: number; h: number }, gap: number): number {
  return Math.min((a.w + b.w + gap) / Math.max(Math.abs(dx), Number.EPSILON), (a.h + b.h + gap) / Math.max(Math.abs(dy), Number.EPSILON));
}

/** The rectangle a box covers when its orb center is at (x, y). */
const rect = (x: number, y: number, box: MapBox) => ({ left: x - box.width / 2, right: x + box.width / 2, top: y - box.orb / 2, bottom: y - box.orb / 2 + box.height });
type Rect = ReturnType<typeof rect>;
const overlaps = (a: Rect, b: Rect, gap: number) => a.left < b.right + gap && b.left < a.right + gap && a.top < b.bottom + gap && b.top < a.bottom + gap;

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
  // A lineage cycle has no first-ring member; its earliest session starts it so
  // every session is placed exactly once.
  const placed = new Set<string>();
  const grow = (session: MapSession, level: number): Tree => {
    placed.add(session.id);
    const children = (kids.get(session.id) ?? []).filter((child) => !placed.has(child.id)).map((child) => grow(child, level + 1));
    return { session, level, children, span: 0, angle: 0 };
  };
  const teams: Tree[] = [];
  for (const session of input.sessions) if (session.parent === null) teams.push(grow(session, 1));
  for (const session of input.sessions) if (!placed.has(session.id)) teams.push(grow(session, 1));
  teams.sort((a, b) => projectOrder.get(a.session.projectId)! - projectOrder.get(b.session.projectId)!);
  const levels: Tree[][] = [];
  const visit = (tree: Tree) => { (levels[tree.level - 1] ??= []).push(tree); tree.children.forEach(visit); };
  teams.forEach(visit);
  const operator = { w: input.operator / 2, h: input.operator / 2 };
  const widest = levels.map((row) => row.reduce((box, tree) => { const own = half(tree.session.box); return { w: Math.max(box.w, own.w), h: Math.max(box.h, own.h) }; }, { w: 0, h: 0 }));
  // The least a ring can step outward from the one inside it, whatever the angle.
  const steps = widest.map((box, index) => { const inner = index === 0 ? operator : widest[index - 1]!; return Math.min(inner.w + box.w, inner.h + box.h) + input.gap; });
  const radiiFrom = (first: number) => steps.reduce<number[]>((radii, step, index) => [...radii, index === 0 ? first : radii[index - 1]! + step], []);
  const boundary = (index: number) => teams[index]!.session.projectId !== teams[(index + 1) % teams.length]!.session.projectId;
  // Each subtree takes the larger of its own width and its children's angles,
  // measured on the innermost radius it can have, so every later radius only adds room.
  const spans = (radii: readonly number[]) => {
    const span = (tree: Tree): number => {
      const own = (tree.session.box.width + input.gap) / radii[tree.level - 1]!;
      tree.span = Math.max(own, tree.children.reduce((total, child) => total + span(child), 0));
      return tree.span;
    };
    const gaps = teams.length > 1 ? teams.reduce((total, _, index) => total + (boundary(index) ? input.projectGap : input.gap) / radii[0]!, 0) : 0;
    return teams.reduce((total, tree) => total + span(tree), 0) + gaps;
  };
  // Solve the smallest first ring whose teams fit around the operator.
  let low = steps[0] ?? 0;
  let high = low;
  for (let attempt = 0; teams.length > 0 && spans(radiiFrom(high)) > TURN; attempt += 1) {
    if (attempt > 60) throw new Error("overview map ring did not converge");
    low = high;
    high *= 2;
  }
  for (let attempt = 0; teams.length > 0 && high - low > 0.5 && attempt < 60; attempt += 1) {
    const middle = (low + high) / 2;
    if (spans(radiiFrom(middle)) > TURN) low = middle; else high = middle;
  }
  const minimum = radiiFrom(high);
  const used = teams.length ? spans(minimum) : 0;
  // Spread what is left between teams, twice as much where projects meet.
  const weights = teams.map((_, index) => (teams.length > 1 && boundary(index) ? 2 : 1));
  const weight = weights.reduce((total, value) => total + value, 0);
  const assign = (tree: Tree, start: number) => {
    tree.angle = start + tree.span / 2;
    const inner = tree.children.reduce((total, child) => total + child.span, 0);
    let cursor = tree.angle - inner / 2;
    for (const child of tree.children) { assign(child, cursor); cursor += child.span; }
  };
  let cursor = TOP - (teams[0]?.span ?? 0) / 2;
  teams.forEach((tree, index) => {
    assign(tree, cursor);
    const gap = teams.length > 1 ? (boundary(index) ? input.projectGap : input.gap) / minimum[0]! : 0;
    cursor += tree.span + gap + (TURN - used) * weights[index]! / weight;
  });
  // Each ring then clears the ring inside it at its members' actual angles.
  const parentOf = new Map<string, Tree>();
  const team = new Map<string, string>();
  const link = (tree: Tree, root: string) => { team.set(tree.session.id, root); for (const child of tree.children) { parentOf.set(child.session.id, tree); link(child, root); } };
  for (const tree of teams) link(tree, tree.session.id);
  const radii = [...minimum];
  levels.forEach((row, index) => {
    for (const tree of row) {
      const parent = parentOf.get(tree.session.id);
      const inner = parent ? half(parent.session.box) : operator;
      const need = (index === 0 ? 0 : radii[index - 1]!) + clearance(Math.cos(tree.angle), Math.sin(tree.angle), inner, half(tree.session.box), input.gap);
      radii[index] = Math.max(radii[index]!, need);
    }
    if (index + 1 < radii.length) radii[index + 1] = Math.max(radii[index + 1]!, radii[index]! + steps[index + 1]!);
  });
  const all = levels.flat();
  const position = (tree: Tree) => ({ x: Math.cos(tree.angle) * radii[tree.level - 1]!, y: Math.sin(tree.angle) * radii[tree.level - 1]! });
  const operatorBox: MapBox = { width: input.operator, height: input.operator, orb: input.operator };
  // A last check over every pair: a ring that still touches another moves out
  // with every ring beyond it, keeping every angle, until nothing touches.
  for (let attempt = 0; ; attempt += 1) {
    const boxes = [{ level: 0, rect: rect(0, 0, operatorBox) }, ...all.map((tree) => { const point = position(tree); return { level: tree.level, rect: rect(point.x, point.y, tree.session.box) }; })];
    let crowded = 0;
    for (let i = 0; i < boxes.length && !crowded; i += 1) for (let j = i + 1; j < boxes.length; j += 1) {
      if (overlaps(boxes[i]!.rect, boxes[j]!.rect, input.gap / 2)) { crowded = Math.max(boxes[i]!.level, boxes[j]!.level); break; }
    }
    if (!crowded) break;
    if (attempt >= 60) throw new Error("overview map could not separate its sessions");
    const delta = Math.max(input.gap, radii[crowded - 1]! * 0.1);
    for (let index = crowded - 1; index < radii.length; index += 1) radii[index]! += delta;
  }
  const nodes = new Map<string, MapPoint>();
  for (const tree of all) nodes.set(tree.session.id, { ...position(tree), level: tree.level, angle: tree.angle, team: team.get(tree.session.id)! });
  // A project's name sits just outside its own sessions, at the middle of their
  // angles, moving outward only past a session or another name it would touch.
  const labels = new Map<string, { x: number; y: number }>();
  const taken: Rect[] = all.map((tree) => { const point = position(tree); return rect(point.x, point.y, tree.session.box); });
  for (const project of input.projects) {
    const members = all.filter((tree) => tree.session.projectId === project.id);
    if (!members.length) continue;
    const x = members.reduce((total, tree) => total + Math.cos(tree.angle), 0);
    const y = members.reduce((total, tree) => total + Math.sin(tree.angle), 0);
    const angle = Math.hypot(x, y) < Number.EPSILON ? members[0]!.angle : Math.atan2(y, x);
    const box = { width: project.label.width, height: project.label.height, orb: project.label.height };
    const reach = members.reduce((far, tree) => Math.max(far, radii[tree.level - 1]!), 0);
    let radius = reach + input.gap + clearance(Math.cos(angle), Math.sin(angle), { w: 0, h: 0 }, half(box), 0);
    for (let attempt = 0; attempt < 400; attempt += 1) {
      const area = rect(Math.cos(angle) * radius, Math.sin(angle) * radius, box);
      if (!taken.some((other) => overlaps(other, area, input.gap / 2))) { taken.push(area); break; }
      radius += project.label.height + input.gap / 2;
    }
    labels.set(project.id, { x: Math.cos(angle) * radius, y: Math.sin(angle) * radius });
  }
  return { nodes, labels };
}
