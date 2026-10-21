import { useEffect, useId, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import { LAYOUT_TIMEOUT_MS, placeWithin, workerElk, type Flow, type GraphEdge, type NodeBox, type Placement } from "./dependencyLayout";
import { cn } from "./lib/utils";
import { useMeasuredPaths } from "./measuredPaths";
import type { LayeredGraph } from "./projectBoard";
import { useShellStore } from "./store";

/** How an edge is drawn: its colour class, and dashed for a stack's edge. */
export type EdgeLook = { tone: string; dashed?: boolean };

/** The gaps a graph keeps, as the token names that hold them: between layers, and between cards in a layer. */
export type GraphGaps = { layers: string; nodes: string };

const ACROSS_GAPS: GraphGaps = { layers: "--home-dependency-gap", nodes: "--spacing-xl" };

/**
 * Nodes that wait on one another, drawn by what they wait on, left to right
 * or (`flow` down) top down: placed by the layout worker once every node is
 * measured, in plain layers until then or when the layout refuses the graph.
 */
export function DependencyGraphView<T>({ graph, draw, idOf, flow = "across", gaps = ACROSS_GAPS, edgeLook }: { graph: LayeredGraph<T>; draw: (value: T) => ReactNode; idOf: (value: T) => string; flow?: Flow; gaps?: GraphGaps; edgeLook?: (edge: GraphEdge) => EdgeLook }) {
  const box = useRef<HTMLDivElement>(null);
  const marker = `dependency-arrow-${useId()}`;
  const values = graph.layers.flat();
  const down = flow === "down";
  // What the layout depends on: the nodes and the edges, not the object a snapshot rebuilt.
  const structure = JSON.stringify([values.map(idOf), graph.edges.map((edge) => [edge.from, edge.to])]);
  const placement = useDependencyPlacement(box, structure, flow, gaps);
  const places = placement ? values.map((value) => placement.nodes.get(idOf(value))) : [];
  const placed = placement !== null && places.every((place) => place !== undefined);
  const arrows = useMeasuredPaths(
    box,
    "[data-dependency-node]",
    (origin, at) =>
      placed
        ? []
        : graph.edges.flatMap((edge) => {
            const from = at("data-dependency-node", edge.from);
            const to = at("data-dependency-node", edge.to);
            if (!from || !to) return [];
            if (down) {
              const x1 = from.left + from.width / 2 - origin.left;
              const y1 = from.bottom - origin.top;
              const x2 = to.left + to.width / 2 - origin.left;
              const y2 = to.top - origin.top;
              const bend = (y2 - y1) / 2;
              return [{ id: `${edge.from}>${edge.to}`, d: `M ${x1} ${y1} C ${x1} ${y1 + bend}, ${x2} ${y2 - bend}, ${x2} ${y2}` }];
            }
            const x1 = from.right - origin.left;
            const y1 = from.top + from.height / 2 - origin.top;
            const x2 = to.left - origin.left;
            const y2 = to.top + to.height / 2 - origin.top;
            const bend = (x2 - x1) / 2;
            return [{ id: `${edge.from}>${edge.to}`, d: `M ${x1} ${y1} C ${x1 + bend} ${y1}, ${x2 - bend} ${y2}, ${x2} ${y2}` }];
          }),
    [structure, placed, down],
  );
  const lines = placed ? placement.edges : arrows;
  const looks = new Map(graph.edges.map((edge) => [`${edge.from}>${edge.to}`, edgeLook?.(edge)]));
  const tones = [...new Set(["", ...[...looks.values()].map((look) => look?.tone ?? "")])];
  const svg = (
    <svg aria-hidden="true" className={cn("pointer-events-none absolute inset-0 size-full overflow-visible", !edgeLook && "text-warning")}>
      <defs>
        {tones.map((tone, index) => (
          // A marker takes its color from where it is defined, not from the edge that uses it, so each tone has its own.
          <marker key={tone} id={`${marker}-${index}`} viewBox="0 0 8 8" refX="7" refY="4" markerWidth="8" markerHeight="8" orient="auto-start-reverse" className={tone || undefined}>
            {down ? <path d="M 1 1.5 L 7 4 L 1 6.5 Z" fill="currentColor" /> : <path d="M 1 1 L 7 4 L 1 7" fill="none" stroke="currentColor" />}
          </marker>
        ))}
      </defs>
      {lines.map((arrow) => {
        const look = looks.get(arrow.id);
        return (
          <path key={arrow.id} d={arrow.d} fill="none" stroke="currentColor" strokeWidth={1.5} strokeDasharray={look?.dashed ? "4 3" : undefined} className={look?.tone} markerEnd={`url(#${marker}-${tones.indexOf(look?.tone ?? "")})`} data-dependency-edge={arrow.id} data-edge-dashed={look?.dashed || undefined} />
        );
      })}
    </svg>
  );
  if (placed) {
    return (
      <div ref={box} className="relative" style={{ width: placement.width, height: placement.height }} data-dependency-graph="true" data-dependency-edges={graph.edges.length} data-dependency-layout="layered">
        {values.map((value, index) => (
          <div key={idOf(value)} className="absolute" style={{ left: places[index]!.x, top: places[index]!.y }} data-dependency-layer={places[index]!.layer}>
            {draw(value)}
          </div>
        ))}
        {svg}
      </div>
    );
  }
  return (
    <div ref={box} className={cn("relative flex w-fit", down ? "flex-col items-center" : "items-start")} style={{ gap: `var(${gaps.layers})` }} data-dependency-graph="true" data-dependency-edges={graph.edges.length} data-dependency-layout="columns">
      {graph.layers.map((layer, index) => (
        <div key={index} className={cn("flex", down ? "flex-row items-start" : "flex-col")} style={{ gap: `var(${gaps.nodes})` }} data-dependency-layer={index}>
          {layer.map(draw)}
        </div>
      ))}
      {svg}
    </div>
  );
}

/**
 * Measures the drawn cards and asks the layout worker where they go, again
 * only when the graph's nodes or edges change or a card changes size: a
 * snapshot that rebuilds the same graph keeps the placement it has. A
 * structure the layout refused stays in columns without asking again, and
 * the reason goes to the diagnostic log once.
 */
function useDependencyPlacement(box: RefObject<HTMLDivElement | null>, structure: string, flow: Flow, gaps: GraphGaps): Placement | null {
  const [placed, setPlaced] = useState<{ key: string; placement: Placement } | null>(null);
  const [measured, setMeasured] = useState<{ key: string; nodes: NodeBox[] } | null>(null);
  // The one structure the layout refused; another structure clears it.
  const refused = useRef<string | null>(null);
  const layout = placed?.key.startsWith(`${structure}|`) ? "layered" : "columns";
  useLayoutEffect(() => {
    const root = box.current;
    if (!root) return;
    const measure = () => {
      const nodes = [...root.querySelectorAll<HTMLElement>("[data-dependency-node]")].map((node) => ({ id: node.dataset.dependencyNode ?? "", width: node.offsetWidth, height: node.offsetHeight }));
      const key = `${structure}|${nodes.map((node) => `${node.width}x${node.height}`).join(",")}`;
      setMeasured((current) => (current?.key === key ? current : { key, nodes }));
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    for (const node of root.querySelectorAll("[data-dependency-node]")) observer.observe(node);
    return () => observer.disconnect();
  }, [structure, layout]);
  useEffect(() => {
    if (!measured || !measured.key.startsWith(`${structure}|`) || measured.nodes.length === 0) return;
    if (placed?.key === measured.key || refused.current === structure) return;
    const [, pairs] = JSON.parse(structure) as [string[], [string, string][]];
    const edges: GraphEdge[] = pairs.map(([from, to]) => ({ from, to }));
    const root = box.current;
    if (!root) return;
    // The same gaps the plain layers keep: between layers, and between cards in one.
    const style = getComputedStyle(root);
    const spacing = { betweenLayers: parseFloat(style.getPropertyValue(gaps.layers)), betweenNodes: parseFloat(style.getPropertyValue(gaps.nodes)) };
    let current = true;
    workerElk()
      .then((elk) => {
        if (!Number.isFinite(spacing.betweenLayers) || !Number.isFinite(spacing.betweenNodes)) throw new Error("the graph's spacing tokens are not set");
        return placeWithin(elk, measured.nodes, edges, spacing, LAYOUT_TIMEOUT_MS, flow);
      })
      .then(
        (placement) => {
          if (current) setPlaced({ key: measured.key, placement });
        },
        (error: unknown) => {
          if (!current) return;
          refused.current = structure;
          useShellStore.getState().noteDiagnostic(`dependency graph stays in columns: ${error instanceof Error ? error.message : String(error)}`);
        },
      );
    return () => {
      current = false;
    };
  }, [measured, structure, flow, gaps.layers, gaps.nodes]);
  return placed?.key.startsWith(`${structure}|`) ? placed.placement : null;
}
