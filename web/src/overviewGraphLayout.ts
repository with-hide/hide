import { useEffect, useRef, useState, type RefObject } from "react";
import { placeCompoundWithin, type CompoundGroup, type CompoundPlacement, type CompoundSpacing } from "./dependencyLayout";
import type { OverviewGraphModel } from "./overviewGraphModel";
import { useShellStore } from "./store";

type Dimensions = { rootWidth: number; rootHeight: number; childWidth: number; childHeight: number; groupWidth: number; groupHeight: number; spacing: CompoundSpacing };
export type GraphLayout = { key: string; phase: "loading" | "ready" | "failed"; placement: CompoundPlacement | null };

/** Geometry comes from the same numeric authority as the rest of Overview. */
function dimensions(element: HTMLElement): Dimensions {
  const style = getComputedStyle(element);
  const read = (name: string) => {
    const value = Number.parseFloat(style.getPropertyValue(`--size-overview-graph-${name}`));
    if (!Number.isFinite(value) || value <= 0) throw new Error(`missing overview graph token: ${name}`);
    return value;
  };
  const gap = read("gap");
  return {
    rootWidth: read("root-width"), rootHeight: read("root-height"),
    childWidth: read("child-width"), childHeight: read("child-height"),
    groupWidth: read("group-width"), groupHeight: read("group-height"),
    spacing: { betweenLayers: gap, betweenNodes: gap, padding: read("padding"), header: read("header"), groups: read("project-gap") },
  };
}

/** One topology owns one request. Status and correspondence refreshes only redraw existing places. */
export function useOverviewGraphLayout(model: OverviewGraphModel, host: RefObject<HTMLDivElement | null>, retry: number) {
  const latest = useRef(model);
  latest.current = model;
  const [layout, setLayout] = useState<GraphLayout>({ key: "", phase: "loading", placement: null });
  const key = model.structure;
  useEffect(() => {
    const controller = new AbortController();
    setLayout((previous) => ({ key, phase: "loading", placement: previous.placement }));
    async function run() {
      if (!host.current) throw new Error("overview graph has no layout host");
      const size = dimensions(host.current);
      const input = latest.current;
      const groups: CompoundGroup[] = input.projects.map((group) => {
        const members = new Set(group.agents.map((row) => row.id));
        return {
          id: group.id, width: size.groupWidth, height: size.groupHeight,
          children: group.expanded ? group.agents.map((row) => {
            const root = !input.lineage.some((edge) => edge.to === row.id);
            return { id: row.id, width: root ? size.rootWidth : size.childWidth, height: root ? size.rootHeight : size.childHeight };
          }) : [],
          edges: input.lineage.filter((edge) => members.has(edge.from) && members.has(edge.to)),
        };
      });
      const placement = await placeCompoundWithin(groups, size.spacing, controller.signal);
      if (!controller.signal.aborted) setLayout({ key, phase: "ready", placement });
    }
    void run().catch((error: unknown) => {
      if (controller.signal.aborted) return;
      useShellStore.getState().noteDiagnostic(`overview_graph.layout_failed projects=${latest.current.projects.length} visible=${latest.current.visible.length} cause=${error instanceof Error ? error.message : String(error)}`);
      setLayout({ key, phase: "failed", placement: null });
    });
    return () => controller.abort();
  }, [key, host, retry]);
  return layout.key === key ? layout : { key, phase: "loading" as const, placement: layout.placement };
}

/** No polling clock or retained observer survives the graph's owner. */
export function useGraphReducedMotion(): boolean {
  const [reduced, setReduced] = useState(() => typeof matchMedia !== "undefined" && matchMedia("(prefers-reduced-motion: reduce)").matches);
  useEffect(() => {
    if (typeof matchMedia === "undefined") return;
    const query = matchMedia("(prefers-reduced-motion: reduce)");
    const update = () => setReduced(query.matches);
    query.addEventListener("change", update);
    update();
    return () => query.removeEventListener("change", update);
  }, []);
  return reduced;
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
