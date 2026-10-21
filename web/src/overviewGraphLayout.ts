import { useEffect, useRef, useState, type RefObject } from "react";
import type { OverviewGraphModel } from "./overviewGraphModel";
import { placeOverviewMap, type MapBox } from "./overviewMapLayout";
import { useShellStore } from "./store";

type Dimensions = { root: MapBox; child: MapBox; operator: number; label: { width: number; height: number }; gap: number; projectGap: number };
/** Label and operator boxes are absolute; a session's box is relative to its project's label, its React Flow parent.
 * `ring` sessions take the root box and stand on the first ring, `team` names the first-ring session they belong to. */
export type GraphPlacement = {
  operator: { x: number; y: number };
  groups: Map<string, { x: number; y: number; width: number; height: number }>;
  nodes: Map<string, { x: number; y: number; ring: boolean; team: string }>;
};
export type GraphLayout = { key: string; phase: "loading" | "ready" | "failed"; placement: GraphPlacement | null; size: Dimensions | null };

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
    operator: read("root-orb"), label: { width: read("root-width"), height: gap * 2 }, gap, projectGap: read("project-gap"),
  };
}

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

/** Topology and text geometry own placement. Status, filters, hiding and correspondence only redraw existing places. */
export function useOverviewGraphLayout(model: OverviewGraphModel, host: RefObject<HTMLDivElement | null>, retry: number) {
  const latest = useRef(model);
  latest.current = model;
  const [layout, setLayout] = useState<GraphLayout>({ key: "", phase: "loading", placement: null, size: null });
  const scale = useGraphScale();
  const key = `${model.structure}:${scale}`;
  useEffect(() => {
    try {
      if (!host.current) throw new Error("overview graph has no layout host");
      const size = dimensions(host.current, scale);
      const input = latest.current;
      const map = placeOverviewMap({
        projects: input.projects.map((group) => ({ id: group.id, label: size.label })),
        sessions: input.projects.flatMap((group) => [
          ...group.shown.map((row) => ({ id: row.id, projectId: group.id, parent: input.layoutParents.get(row.id) ?? null, box: input.layoutParents.get(row.id) == null ? size.root : size.child })),
          ...(group.revealSlot ? [{ id: group.revealSlot, projectId: group.id, parent: null, box: size.root }] : []),
        ]),
        operator: size.operator, gap: size.gap, projectGap: size.projectGap,
      });
      const groups = new Map([...map.labels].map(([id, point]) => [id, { x: point.x - size.label.width / 2, y: point.y - size.label.height / 2, ...size.label }]));
      const nodes: GraphPlacement["nodes"] = new Map();
      for (const group of input.projects) {
        const label = groups.get(group.id);
        for (const id of [...group.shown.map((row) => row.id), ...(group.revealSlot ? [group.revealSlot] : [])]) {
          const point = map.nodes.get(id);
          if (!point || !label) throw new Error("layout omitted a project or session");
          const ring = id === group.revealSlot || input.layoutParents.get(id) == null;
          const box = ring ? size.root : size.child;
          nodes.set(id, { x: point.x - box.width / 2 - label.x, y: point.y - box.orb / 2 - label.y, ring, team: point.team });
        }
      }
      setLayout({ key, phase: "ready", placement: { operator: { x: -size.operator / 2, y: -size.operator / 2 }, groups, nodes }, size });
    } catch (error: unknown) {
      useShellStore.getState().noteDiagnostic(`overview_graph.layout_failed projects=${latest.current.projects.length} visible=${latest.current.visible.length} cause=${error instanceof Error ? error.message : String(error)}`);
      setLayout({ key, phase: "failed", placement: null, size: null });
    }
  }, [key, host, retry, scale]);
  return layout.key === key ? layout : { key, phase: "loading" as const, placement: layout.placement, size: layout.size };
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
