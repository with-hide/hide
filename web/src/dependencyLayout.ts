// Places the shared dependency graph (the Issues view's Dependencies mode
// and the Factory graph, PRD software-factory-ui D-08) with elkjs's layered
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

/** Lays the graph out with `elk` and reads back each card's place, its column, and each edge's route. */
export async function placeGraph(elk: Pick<ELK, "layout">, nodes: readonly NodeBox[], edges: readonly GraphEdge[], spacing: Spacing): Promise<Placement> {
  if (nodes.length > LAYOUT_NODE_LIMIT) throw new Error(`dependency graph has ${nodes.length} nodes, over the layout limit of ${LAYOUT_NODE_LIMIT}`);
  const known = new Set(nodes.map((node) => node.id));
  const input: ElkNode = {
    id: "root",
    layoutOptions: {
      "elk.algorithm": "layered",
      "elk.direction": "RIGHT",
      "elk.edgeRouting": "POLYLINE",
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
  const columns = [...new Set((laid.children ?? []).map((child) => child.x ?? 0))].sort((a, b) => a - b);
  const placed = new Map<string, PlacedNode>();
  for (const child of laid.children ?? []) placed.set(child.id, { x: child.x ?? 0, y: child.y ?? 0, layer: columns.indexOf(child.x ?? 0) });
  const routed = (laid.edges ?? []).flatMap((edge: ElkExtendedEdge) =>
    (edge.sections ?? []).map((section) => ({ id: edge.id, d: edgePath([section.startPoint, ...(section.bendPoints ?? []), section.endPoint]) })),
  );
  return { width: laid.width ?? 0, height: laid.height ?? 0, nodes: placed, edges: routed };
}

/** `placeGraph` with a deadline: a layout that does not answer in time is refused, and the worker is ended so it cannot keep working. */
export function placeWithin(elk: Pick<ELK, "layout" | "terminateWorker">, nodes: readonly NodeBox[], edges: readonly GraphEdge[], spacing: Spacing, timeoutMs = LAYOUT_TIMEOUT_MS): Promise<Placement> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      elk.terminateWorker();
      dropWorker(elk);
      reject(new Error(`dependency graph layout took longer than ${timeoutMs} ms`));
    }, timeoutMs);
    placeGraph(elk, nodes, edges, spacing).then(
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

/** A caller-owned worker. Ending an Overview must not terminate a Factory's worker. */
export async function ownedWorkerElk(): Promise<ELK> {
  if (typeof Worker === "undefined") throw new Error("this page cannot start a layout worker");
  const [api, worker] = await Promise.all([import("elkjs/lib/elk-api.js"), import("elkjs/lib/elk-worker.min.js?url")]);
  return new api.default({ workerUrl: worker.default });
}

export type CompoundGroup = NodeBox & { children: NodeBox[]; edges: GraphEdge[] };
export type CompoundPlacement = { groups: Map<string, NodeBox & { x: number; y: number }>; nodes: Map<string, PlacedNode> };
export type CompoundSpacing = Spacing & { padding: number; header: number; groups: number };

/** Sessions occupy fixed radial slots; ELK packs their independent project footprints.
 * Fixed ring capacity leaves existing slots still when another session is revealed.
 * A ring's chord exceeds the diagonal of a session plus its gap, preventing overlap.
 */
export async function placeCompoundGraph(elk: Pick<ELK, "layout">, groups: readonly CompoundGroup[], spacing: CompoundSpacing): Promise<CompoundPlacement> {
  const count = groups.reduce((sum, group) => sum + 1 + group.children.length, 0);
  if (count > LAYOUT_NODE_LIMIT) throw new Error(`compound graph has ${count} nodes, over the layout limit of ${LAYOUT_NODE_LIMIT}`);
  const placed: CompoundPlacement = { groups: new Map(), nodes: new Map() };
  const footprints = groups.map((group) => {
    const width = Math.max(0, ...group.children.map((node) => node.width));
    const height = Math.max(0, ...group.children.map((node) => node.height));
    const diagonal = Math.hypot(width, height);
    const step = Math.max(diagonal, (Math.hypot(group.width, group.height) + diagonal) / 2) + spacing.betweenNodes;
    const points = group.children.map((node, index) => {
      // Six on the first ring, twelve on the second, and so on.
      let ring = 1;
      let slot = index;
      while (slot >= 6 * ring) { slot -= 6 * ring; ring += 1; }
      const radius = step * ring;
      const occupants = Math.min(6 * ring, group.children.length - index + slot);
      const angle = -Math.PI / 2 + slot * Math.PI * 2 / occupants;
      return { id: node.id, x: Math.cos(angle) * radius - node.width / 2, y: Math.sin(angle) * radius - node.height / 2, layer: ring };
    });
    const extent = points.reduce((box, point, index) => ({
      left: Math.min(box.left, point.x), top: Math.min(box.top, point.y),
      right: Math.max(box.right, point.x + group.children[index]!.width), bottom: Math.max(box.bottom, point.y + group.children[index]!.height),
    }), { left: -group.width / 2, top: -group.height / 2, right: group.width / 2, bottom: group.height / 2 });
    // Symmetric bounds make the project heading the true cluster center.
    const halfWidth = Math.max(-extent.left, extent.right) + spacing.padding;
    const halfHeight = Math.max(-extent.top, extent.bottom) + spacing.padding;
    for (const point of points) placed.nodes.set(point.id, { x: point.x + halfWidth, y: point.y + halfHeight, layer: point.layer });
    return { id: group.id, width: halfWidth * 2, height: halfHeight * 2 };
  });
  const laid = await elk.layout({ id: "overview", layoutOptions: {
    "elk.algorithm": "rectpacking", "elk.aspectRatio": "1.6", "elk.spacing.nodeNode": String(spacing.groups), "elk.padding": "[top=0,left=0,bottom=0,right=0]",
  }, children: footprints });
  for (const group of laid.children ?? []) {
    if (![group.x, group.y, group.width, group.height].every((value) => value !== undefined && Number.isFinite(value))) throw new Error("layout returned an incomplete project");
    placed.groups.set(group.id, { id: group.id, x: group.x!, y: group.y!, width: group.width!, height: group.height! });
  }
  if (placed.groups.size !== groups.length || placed.nodes.size !== count - groups.length) throw new Error("layout omitted a project or session");
  return placed;
}

/** One request owns its worker, startup deadline and cancellation. No pending request queue. */
export async function placeCompoundWithin(groups: readonly CompoundGroup[], spacing: CompoundSpacing, signal: AbortSignal, timeoutMs = LAYOUT_TIMEOUT_MS): Promise<CompoundPlacement> {
  let elk: ELK | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let cancel: (() => void) | undefined;
  const stopped = new Promise<never>((_, reject) => {
    cancel = () => reject(new DOMException("layout cancelled", "AbortError"));
    signal.addEventListener("abort", cancel, { once: true });
    timer = setTimeout(() => reject(new Error("overview layout deadline exceeded")), timeoutMs);
    if (signal.aborted) cancel();
  });
  let ended = false;
  const work = ownedWorkerElk().then((worker) => {
    elk = worker;
    if (ended || signal.aborted) { worker.terminateWorker(); throw new DOMException("layout cancelled", "AbortError"); }
    return placeCompoundGraph(worker, groups, spacing);
  });
  try { return await Promise.race([work, stopped]); }
  finally {
    ended = true;
    clearTimeout(timer);
    if (cancel) signal.removeEventListener("abort", cancel);
    elk?.terminateWorker();
  }
}

/** Forgets an ended worker, so the next graph starts a fresh one. */
function dropWorker(elk: unknown) {
  void shared?.then((current) => {
    if (current === elk) shared = null;
  });
}
