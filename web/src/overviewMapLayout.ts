// Places the Overview work map (PRD overview-main-map D-01, D-02, D-13, D-18,
// D-20) with d3-force, the way Obsidian's graph view does: the operator is fixed
// at the origin, each project gathers around its own anchor on a circle around
// the operator, a root is held loosely toward the operator, and a delegated
// session is tied short and tight to its parent. The simulation settles before
// the first paint, so nothing wobbles on open, and every start position comes
// from the session's id, so the same sessions open in the same shape. When
// sessions come or go, the ones already placed hold still while only the new
// ones find a place. Drags move bodies and, unless motion is reduced, let the
// physics pull a team along and settle on release.

import { forceCollide, forceLink, forceManyBody, forceSimulation, forceX, forceY, type ForceLink, type ForceX, type ForceY, type Simulation, type SimulationLinkDatum, type SimulationNodeDatum } from "d3-force";
import { LAYOUT_NODE_LIMIT } from "./dependencyLayout";

/** `parent` names a placed session, or null for a root; `radius` is how much room the session's circle and words need. */
export type MapSession = { id: string; projectId: string; parent: string | null; radius: number };
export type MapInput = {
  /** Projects in drawing order; a session's project must be listed. */
  projects: readonly string[];
  sessions: readonly MapSession[];
  /** Room the operator's circle needs. */
  operator: number;
};
type Point = { x: number; y: number };
/** An orb center, and the session whose team it belongs to (a root names its own). */
export type MapPoint = Point & { team: string };

type Body = SimulationNodeDatum & { id: string; projectId: string; parent: string | null; radius: number; team: string; root: boolean };
type Tie = SimulationLinkDatum<Body> & { kind: "team" | "operator"; length: number };

const OPERATOR = "\u0000operator";
/** Ticks for alpha to fall from 1 to d3's alphaMin; a quicker decay than d3's 300 ticks keeps four hundred nodes within a frame budget the operator does not notice. */
const SETTLE = 120;
const DECAY = 1 - Math.pow(0.001, 1 / SETTLE);

/** FNV-1a of an id: the map's starts and a project's accent depend on identity alone, never on order or chance. */
export function idHash(id: string): number {
  let hash = 2166136261;
  for (let index = 0; index < id.length; index += 1) hash = Math.imul(hash ^ id.charCodeAt(index), 16777619);
  return hash >>> 0;
}

const angleOf = (id: string) => (idHash(id) / 4294967296) * Math.PI * 2;

export class ForceMap {
  private readonly bodies = new Map<string, Body>();
  private readonly anchors = new Map<string, Point>();
  private readonly simulation: Simulation<Body, Tie>;
  private ties: Tie[] = [];
  private readonly operator: Body;
  private reduced = false;
  private listener: (() => void) | null = null;

  constructor(input: MapInput, previous?: ForceMap) {
    this.operator = { id: OPERATOR, projectId: "", parent: null, radius: input.operator, team: OPERATOR, root: false, x: 0, y: 0, fx: 0, fy: 0 };
    this.simulation = forceSimulation<Body, Tie>().alphaDecay(DECAY).stop();
    this.place(input, previous);
  }

  /** Every session's orb center. */
  positions(): Map<string, MapPoint> {
    const out = new Map<string, MapPoint>();
    for (const body of this.bodies.values()) out.set(body.id, { x: body.x!, y: body.y!, team: body.team });
    return out;
  }

  /** Called after each animated step and each settled drop, so the view can redraw. */
  listen(listener: (() => void) | null) { this.listener = listener; }

  /** With motion reduced, a drag moves only what the hand holds and the rest settles at once on release. */
  reduceMotion(reduced: boolean) { this.reduced = reduced; }

  /** The operator holds a session: it follows the hand, and unless motion is reduced its team is pulled along. */
  grab(id: string) {
    const body = this.bodies.get(id);
    if (!body) return;
    body.fx = body.x;
    body.fy = body.y;
    if (!this.reduced) this.animate(0.25);
  }

  drag(id: string, to: Point) {
    const body = this.bodies.get(id);
    if (!body) return;
    body.fx = to.x;
    body.fy = to.y;
    if (this.reduced) { body.x = to.x; body.y = to.y; }
  }

  /** The session stays where it was put; the rest settles around it. */
  drop(id: string) {
    if (this.bodies.has(id)) this.settle();
  }

  /** Where a project gathers; a project drag reads it at the start and moves it by the hand's travel. */
  anchorOf(projectId: string): Point | undefined {
    const anchor = this.anchors.get(projectId);
    return anchor ? { ...anchor } : undefined;
  }

  /** Every session of a project moves together, with the place the project gathers around. */
  moveProject(projectId: string, by: Point) {
    const anchor = this.anchors.get(projectId);
    if (!anchor) return;
    this.simulation.stop();
    anchor.x += by.x;
    anchor.y += by.y;
    for (const body of this.bodies.values()) {
      if (body.projectId !== projectId) continue;
      body.x! += by.x;
      body.y! += by.y;
      if (body.fx != null) body.fx += by.x;
      if (body.fy != null) body.fy += by.y;
      body.vx = 0;
      body.vy = 0;
    }
  }

  /** The project stays where it was put: its roots' ties to the operator take their new length, so no later settling pulls it back. */
  private readonly anchorX = (body: Body) => this.anchors.get(body.projectId)?.x ?? 0;
  private readonly anchorY = (body: Body) => this.anchors.get(body.projectId)?.y ?? 0;

  private operatorTie(id: string): number | undefined {
    return this.ties.find((tie) => tie.kind === "operator" && (tie.target as Body).id === id)?.length;
  }

  dropProject() {
    for (const tie of this.ties) {
      const root = tie.target as Body;
      if (tie.kind === "operator") tie.length = Math.hypot(root.x!, root.y!);
    }
    // d3 reads a force's targets once, so the moved anchor and lengths are read again.
    (this.simulation.force("tie") as ForceLink<Body, Tie>).distance((tie) => tie.length);
    (this.simulation.force("x") as ForceX<Body>).x(this.anchorX);
    (this.simulation.force("y") as ForceY<Body>).y(this.anchorY);
    this.listener?.();
  }

  /** Stop any animation; the owner calls this when the view closes or replaces the map. */
  stop() { this.simulation.stop(); this.simulation.on("tick", null); this.listener = null; }

  private settle() {
    if (this.reduced) {
      this.simulation.stop();
      this.simulation.alpha(0.3).alphaTarget(0);
      while (this.simulation.alpha() > this.simulation.alphaMin()) this.simulation.tick();
      this.listener?.();
      return;
    }
    this.animate(0);
    if (this.simulation.alpha() < 0.1) this.simulation.alpha(0.1);
  }

  private animate(target: number) {
    this.simulation.on("tick", () => this.listener?.()).alphaTarget(target).restart();
  }

  private place(input: MapInput, previous?: ForceMap) {
    const count = input.projects.length + input.sessions.length;
    if (count > LAYOUT_NODE_LIMIT) throw new Error(`overview map has ${count} nodes, over the layout limit of ${LAYOUT_NODE_LIMIT}`);
    const projects = new Set(input.projects);
    const byId = new Map(input.sessions.map((session) => [session.id, session]));
    for (const session of input.sessions) {
      if (!projects.has(session.projectId)) throw new Error(`session ${session.id} names an unlisted project`);
      if (session.parent !== null && !byId.has(session.parent)) throw new Error(`session ${session.id} names an unplaced parent`);
    }
    // A team is named by its root; a lineage cycle is named by its smallest member.
    const teamOf = (session: MapSession): string => {
      const seen = new Set<string>();
      let current = session;
      while (current.parent !== null && !seen.has(current.id)) { seen.add(current.id); current = byId.get(current.parent)!; }
      return current.parent === null ? current.id : [...seen].sort()[0]!;
    };
    // Projects gather on an ellipse around the operator, wider than tall like the screen and wide enough for how many sessions there are.
    const reach = Math.max(380, 72 * Math.sqrt(input.sessions.length + 1));
    input.projects.forEach((id, index) => {
      const kept = previous?.anchors.get(id);
      const angle = -Math.PI / 2 + (index / Math.max(input.projects.length, 1)) * Math.PI * 2;
      this.anchors.set(id, kept ? { ...kept } : input.projects.length === 1 ? { x: 0, y: -reach * 0.6 } : { x: Math.cos(angle) * reach, y: Math.sin(angle) * reach * 0.7 });
    });
    // Parents start before their children, so a new child starts beside its parent.
    const ordered: MapSession[] = [];
    const queued = new Set<string>();
    const visit = (session: MapSession) => {
      if (queued.has(session.id)) return;
      queued.add(session.id);
      if (session.parent !== null) visit(byId.get(session.parent)!);
      ordered.push(session);
    };
    // Identity order, never the caller's order, so the same set of sessions makes the same shape.
    for (const session of [...input.sessions].sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0))) visit(session);
    const held: Body[] = [];
    for (const session of ordered) {
      const old = previous?.bodies.get(session.id);
      const body: Body = { id: session.id, projectId: session.projectId, parent: session.parent, radius: session.radius, team: teamOf(session), root: session.parent === null };
      if (old) {
        // Already placed: it keeps its place, and an operator's pin, while the new ones settle.
        body.x = old.x; body.y = old.y;
        body.fx = old.x; body.fy = old.y;
        if (old.fx == null) held.push(body);
      } else {
        const near: Point = session.parent !== null && this.bodies.has(session.parent) ? { x: this.bodies.get(session.parent)!.x!, y: this.bodies.get(session.parent)!.y! } : this.anchors.get(session.projectId)!;
        const angle = angleOf(session.id);
        const spread = session.parent !== null ? session.radius * 2 : 48;
        body.x = near.x + Math.cos(angle) * spread;
        body.y = near.y + Math.sin(angle) * spread;
      }
      this.bodies.set(session.id, body);
    }
    const ties: Tie[] = [];
    this.ties = ties;
    for (const body of this.bodies.values()) {
      if (body.parent !== null && body.parent !== body.id) ties.push({ source: body.parent, target: body.id, kind: "team", length: this.bodies.get(body.parent)!.radius + body.radius });
      else if (body.parent === null) ties.push({ source: OPERATOR, target: body.id, kind: "operator", length: previous?.operatorTie(body.id) ?? reach * 0.63 });
    }
    this.simulation
      .nodes([this.operator, ...this.bodies.values()])
      .force("tie", forceLink<Body, Tie>(ties).id((body) => body.id).distance((tie) => tie.length).strength((tie) => (tie.kind === "team" ? 0.9 : 0.04)))
      .force("charge", forceManyBody<Body>().strength((body) => (body.id === OPERATOR ? -600 : body.root ? -320 : -140)))
      .force("collide", forceCollide<Body>((body) => body.radius).iterations(3))
      .force("x", forceX<Body>(this.anchorX).strength((body) => (body.id === OPERATOR ? 0 : body.root ? 0.28 : 0.02)))
      .force("y", forceY<Body>(this.anchorY).strength((body) => (body.id === OPERATOR ? 0 : body.root ? 0.28 : 0.02)))
      .alpha(1).alphaTarget(0);
    for (let step = 0; step < SETTLE; step += 1) this.simulation.tick();
    for (const body of held) { body.fx = null; body.fy = null; }
  }
}
