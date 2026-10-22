// Places the shared dependency graph (the Issues view's Dependencies mode
// and the Factory graph, PRD software-factory-ui D-08) and a mission's map
// (PRD software-factory-conductor B15) with elkjs's layered
// algorithm: a long edge is routed through the gaps between cards instead of
// across them, and crossings are minimised. elkjs is used unmodified under
// EPL-2.0. It loads only when a graph opens and runs in its own Web Worker,
// so neither the main thread nor the terminal path takes the layout's work.
// A layout that fails, runs too long, or is asked for too many nodes leaves
// the column layout in place and says so in the diagnostic log.

import type { ELK, ElkExtendedEdge, ElkNode, ElkPoint } from "elkjs/lib/elk-api";

/** More cards than this are left in columns: the layout would outgrow a frame's worth of waiting. */
export const LAYOUT_NODE_LIMIT = 400;
/** How long the worker may take before the column layout stays. */
export const LAYOUT_TIMEOUT_MS = 3_000;

export type NodeBox = { id: string; width: number; height: number };
export type GraphEdge = { from: string; to: string };
/** The space kept between columns and between cards in a column, as the column layout draws it. */
export type Spacing = { betweenLayers: number; betweenNodes: number };
export type PlacedNode = { x: number; y: number; layer: number };
export type Placement = { width: number; height: number; nodes: Map<string, PlacedNode>; edges: { id: string; d: string }[] };
/**
 * Which way the graph runs: the dependency graph left to right with curved
 * edges, a mission's map top down with square edges whose corners are
 * rounded (PRD software-factory-conductor B15).
 */
export type Flow = "across" | "down";

/**
 * A routed edge as a path through its points: each step leaves and enters
 * horizontally, so a straight run stays straight and a step between two rows
 * bends in the gap between columns, where no card is.
 */
export function edgePath(points: readonly ElkPoint[]): string {
  const [first, ...rest] = points;
  if (!first) return "";
  let d = `M ${first.x} ${first.y}`;
  let at = first;
  for (const next of rest) {
    const bend = (next.x - at.x) / 2;
    d += ` C ${at.x + bend} ${at.y}, ${next.x - bend} ${next.y}, ${next.x} ${next.y}`;
    at = next;
  }
  return d;
}

/**
 * A square-routed edge as a path through its points, each corner rounded by
 * at most `radius` and never past half of either leg, so a short jog stays
 * a jog.
 */
export function squarePath(points: readonly ElkPoint[], radius: number): string {
  const [first, ...rest] = points;
  if (!first) return "";
  let d = `M ${first.x} ${first.y}`;
  for (let index = 0; index < rest.length; index += 1) {
    const corner = rest[index]!;
    const after = rest[index + 1];
    if (!after) {
      d += ` L ${corner.x} ${corner.y}`;
      break;
    }
    const before = index === 0 ? first : rest[index - 1]!;
    const into = Math.hypot(corner.x - before.x, corner.y - before.y);
    const out = Math.hypot(after.x - corner.x, after.y - corner.y);
    const r = Math.min(radius, into / 2, out / 2);
    if (r <= 0) {
      d += ` L ${corner.x} ${corner.y}`;
      continue;
    }
    const start = { x: corner.x - ((corner.x - before.x) / into) * r, y: corner.y - ((corner.y - before.y) / into) * r };
    const end = { x: corner.x + ((after.x - corner.x) / out) * r, y: corner.y + ((after.y - corner.y) / out) * r };
    d += ` L ${start.x} ${start.y} Q ${corner.x} ${corner.y}, ${end.x} ${end.y}`;
  }
  return d;
}

/** The radius a square edge's corners are rounded by. */
const CORNER = 8;

/** Lays the graph out with `elk` and reads back each card's place, its column (or row, down), and each edge's route. */
export async function placeGraph(elk: Pick<ELK, "layout">, nodes: readonly NodeBox[], edges: readonly GraphEdge[], spacing: Spacing, flow: Flow = "across"): Promise<Placement> {
  if (nodes.length > LAYOUT_NODE_LIMIT) throw new Error(`dependency graph has ${nodes.length} nodes, over the layout limit of ${LAYOUT_NODE_LIMIT}`);
  const known = new Set(nodes.map((node) => node.id));
  const input: ElkNode = {
    id: "root",
    layoutOptions: {
      "elk.algorithm": "layered",
      "elk.direction": flow === "down" ? "DOWN" : "RIGHT",
      "elk.edgeRouting": flow === "down" ? "ORTHOGONAL" : "POLYLINE",
      // Down, every node that waits on nothing starts on the top row, as a plan reads: what starts now, then what follows.
      ...(flow === "down" ? { "elk.layered.layering.strategy": "LONGEST_PATH_SOURCE" } : {}),
      "elk.layered.spacing.nodeNodeBetweenLayers": String(spacing.betweenLayers),
      "elk.spacing.nodeNode": String(spacing.betweenNodes),
      "elk.layered.spacing.edgeNodeBetweenLayers": String(spacing.betweenNodes / 2),
      "elk.spacing.edgeNode": String(spacing.betweenNodes / 2),
      "elk.padding": "[top=0,left=0,bottom=0,right=0]",
    },
    children: nodes.map((node) => ({ id: node.id, width: node.width, height: node.height })),
    edges: edges.filter((edge) => known.has(edge.from) && known.has(edge.to)).map((edge) => ({ id: `${edge.from}>${edge.to}`, sources: [edge.from], targets: [edge.to] })),
  };
  const laid = await elk.layout(input);
  const along = (child: ElkNode) => (flow === "down" ? child.y : child.x) ?? 0;
  const layers = [...new Set((laid.children ?? []).map(along))].sort((a, b) => a - b);
  const placed = new Map<string, PlacedNode>();
  for (const child of laid.children ?? []) placed.set(child.id, { x: child.x ?? 0, y: child.y ?? 0, layer: layers.indexOf(along(child)) });
  const routed = (laid.edges ?? []).flatMap((edge: ElkExtendedEdge) =>
    (edge.sections ?? []).map((section) => {
      const points = [section.startPoint, ...(section.bendPoints ?? []), section.endPoint];
      return { id: edge.id, d: flow === "down" ? squarePath(points, CORNER) : edgePath(points) };
    }),
  );
  return { width: laid.width ?? 0, height: laid.height ?? 0, nodes: placed, edges: routed };
}

/** `placeGraph` with a deadline: a layout that does not answer in time is refused, and the worker is ended so it cannot keep working. */
export function placeWithin(elk: Pick<ELK, "layout" | "terminateWorker">, nodes: readonly NodeBox[], edges: readonly GraphEdge[], spacing: Spacing, timeoutMs = LAYOUT_TIMEOUT_MS, flow: Flow = "across"): Promise<Placement> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      elk.terminateWorker();
      dropWorker(elk);
      reject(new Error(`dependency graph layout took longer than ${timeoutMs} ms`));
    }, timeoutMs);
    placeGraph(elk, nodes, edges, spacing, flow).then(
      (placement) => {
        clearTimeout(timer);
        resolve(placement);
      },
      (error: unknown) => {
        clearTimeout(timer);
        reject(error instanceof Error ? error : new Error(String(error)));
      },
    );
  });
}

let shared: Promise<ELK> | null = null;

/** The one layout worker the page keeps, started the first time a graph opens. */
export function workerElk(): Promise<ELK> {
  // Without workers the layout cannot run, so refuse before loading the layout library.
  if (typeof Worker === "undefined") return Promise.reject(new Error("this page cannot start a layout worker"));
  shared ??= Promise.all([import("elkjs/lib/elk-api.js"), import("elkjs/lib/elk-worker.min.js?url")]).then(([api, worker]) => new api.default({ workerUrl: worker.default }));
  // A worker that failed to start is not kept, so the next graph tries again.
  shared.catch(() => {
    shared = null;
  });
  return shared;
}

/** Forgets an ended worker, so the next graph starts a fresh one. */
function dropWorker(elk: unknown) {
  void shared?.then((current) => {
    if (current === elk) shared = null;
  });
}
