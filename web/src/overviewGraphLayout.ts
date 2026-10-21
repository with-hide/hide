import { useCallback, useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react";
import type { OverviewGraphModel } from "./overviewGraphModel";
import { ForceMap, type MapPoint } from "./overviewMapLayout";
import { useShellStore } from "./store";

/** A box with its orb at the top center: `orb` is the orb's diameter. */
export type GraphBox = { width: number; height: number; orb: number };
type Dimensions = { root: GraphBox; child: GraphBox; operator: number; label: { width: number; height: number }; gap: number };
export type GraphLayout = {
  phase: "loading" | "ready" | "failed";
  size: Dimensions | null;
  /** Each placed session's orb center, with the operator's at the origin, and its team. */
  points: ReadonlyMap<string, MapPoint>;
  /** The live map that drags move; null until placed or after a failure. */
  map: ForceMap | null;
  /** Places every session afresh, dropping every drag. */
  reset: () => void;
};

/** Geometry comes from the same numeric authority as the rest of Overview. */
function dimensions(element: HTMLElement, scale: number): Dimensions {
  const style = getComputedStyle(element);
  const read = (name: string) => {
    const value = Number.parseFloat(style.getPropertyValue(`--size-overview-graph-${name}`));
    if (!Number.isFinite(value) || value <= 0) throw new Error(`missing overview graph token: ${name}`);
    return value * scale;
  };
  const gap = read("gap");
  return {
    root: { width: read("root-width"), height: read("root-height"), orb: read("root-orb") },
    child: { width: read("child-width"), height: read("child-height"), orb: read("child-orb") },
    operator: read("root-orb"), label: { width: read("root-width") * 2, height: gap * 2 }, gap,
  };
}

/** The room a session's circle and words need around its orb center, for the physics to keep apart. */
const room = (box: GraphBox, gap: number) => Math.max(box.width / 2, box.height - box.orb / 2) * 0.85 + gap / 2;

/** Appearance writes the interface scale on the root; release its observer with the graph. */
function useGraphScale(): number {
  const read = () => Math.max(1, Number.parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--interface-scale")) || 1);
  const [scale, setScale] = useState(read);
  useEffect(() => {
    const observer = new MutationObserver(() => setScale(read()));
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["style"] });
    setScale(read());
    return () => observer.disconnect();
  }, []);
  return scale;
}

/** The operator's reduced-motion setting, followed while the graph is open. */
function useReducedMotion(): boolean {
  const query = "(prefers-reduced-motion: reduce)";
  const [reduced, setReduced] = useState(() => window.matchMedia?.(query).matches ?? false);
  useEffect(() => {
    const media = window.matchMedia?.(query);
    if (!media) return;
    const change = () => setReduced(media.matches);
    media.addEventListener("change", change);
    return () => media.removeEventListener("change", change);
  }, []);
  return reduced;
}

/**
 * Topology and text geometry own placement. Status, filters, hiding, folding and
 * correspondence only redraw existing places. A changed set of sessions keeps
 * every placed session where it is and places only the new ones; a changed
 * interface scale, Retry and Reset layout place everything afresh.
 */
export function useOverviewGraphLayout(model: OverviewGraphModel, host: RefObject<HTMLDivElement | null>, retry: number): GraphLayout {
  const latest = useRef(model);
  latest.current = model;
  const scale = useGraphScale();
  const reduced = useReducedMotion();
  const key = `${model.structure}:${scale}`;
  const [state, setState] = useState<{ key: string; phase: GraphLayout["phase"]; size: Dimensions | null; map: ForceMap | null; points: ReadonlyMap<string, MapPoint> }>({ key: "", phase: "loading", size: null, map: null, points: new Map() });
  // Each animated step only asks for a redraw; the view reads the live map itself.
  const [, setSteps] = useState(0);
  const reducedNow = useRef(reduced);
  reducedNow.current = reduced;
  const current = useRef<{ map: ForceMap; scale: number } | null>(null);
  // Reset layout counts up; a placement for a count not yet placed starts afresh.
  const [resets, setResets] = useState(0);
  const placedResets = useRef(0);
  // Placement is synchronous, so it lands before paint and nothing wobbles or flashes on open.
  useLayoutEffect(() => {
    try {
      if (!host.current) throw new Error("overview graph has no layout host");
      const size = dimensions(host.current, scale);
      const input = latest.current;
      const keep = current.current && current.current.scale === scale && placedResets.current === resets ? current.current.map : undefined;
      placedResets.current = resets;
      const map = new ForceMap({
        projects: input.projects.map((group) => group.id),
        sessions: input.projects.flatMap((group) => [
          ...group.shown.map((row) => { const parent = input.layoutParents.get(row.id) ?? null; return { id: row.id, projectId: group.id, parent, radius: room(parent === null ? size.root : size.child, size.gap) }; }),
          ...(group.revealSlot ? [{ id: group.revealSlot, projectId: group.id, parent: null, radius: room(size.root, size.gap) }] : []),
        ]),
        operator: size.operator + size.gap * 2,
      }, keep);
      current.current?.map.stop();
      current.current = { map, scale };
      map.reduceMotion(reducedNow.current);
      map.listen(() => { if (current.current?.map === map) setSteps((value) => value + 1); });
      setState({ key, phase: "ready", size, map, points: map.positions() });
    } catch (error: unknown) {
      useShellStore.getState().noteDiagnostic(`overview_graph.layout_failed projects=${latest.current.projects.length} visible=${latest.current.visible.length} cause=${error instanceof Error ? error.message : String(error)}`);
      current.current?.map.stop();
      current.current = null;
      setState({ key, phase: "failed", size: null, map: null, points: new Map() });
    }
  }, [key, host, retry, scale, resets]);
  // A change of the motion setting reaches the live map; it never places the map again.
  useEffect(() => { state.map?.reduceMotion(reduced); }, [state.map, reduced]);
  useEffect(() => () => { current.current?.map.stop(); current.current = null; }, []);
  const reset = useCallback(() => setResets((value) => value + 1), []);
  // Until a new structure is placed, the placed map stays on screen, so React Flow keeps its nodes.
  if (state.key !== key && state.phase !== "ready") return { phase: "loading", size: state.size, points: state.points, map: null, reset };
  return { phase: state.phase, size: state.size, points: state.points, map: state.map, reset };
}

/** React Flow's theme class must follow the shell rather than reset inherited tokens. */
export function useGraphTheme(): "light" | "dark" {
  const read = () => document.documentElement.classList.contains("light") ? "light" as const : "dark" as const;
  const [theme, setTheme] = useState(read);
  useEffect(() => {
    const observer = new MutationObserver(() => setTheme(read()));
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });
    setTheme(read());
    return () => observer.disconnect();
  }, []);
  return theme;
}
