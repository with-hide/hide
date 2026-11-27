import contract from "../../contracts/pane-floor.json";
import type { AreaNode, AreaItem } from "./areaLayout";
import type { LayoutNode, PaneLayout, RemotePaneLayout } from "./snapshot";

/**
 * The pane floor (docs/UI_BEHAVIOR.md, The pane floor): the narrowest
 * terminal pane, in columns, every agent's input box reads in, and the
 * narrowest pane that splits right into two of at least that. Both are the
 * core's (`hide_agent_adapter::composer`), carried by
 * `contracts/pane-floor.json`; the core refuses a split right on the same
 * number, so this page only compares.
 */
export const PANE_FLOOR: { columns: number; split_right_columns: number } = contract;

/** Whether a pane `cols` columns wide splits right with both halves at the floor or wider. */
export function splitsRight(cols: number): boolean {
  return cols >= PANE_FLOOR.split_right_columns;
}

/**
 * The least width a tab's panes need for every pane drawn to keep `floor`
 * pixels for its own pane id: a split right gives each side its share of
 * the width and its divider takes none, a split down gives both the whole
 * width, and a zoomed tab draws its focused pane alone.
 */
export function paneTreeWidth(layout: Pick<PaneLayout, "root" | "zoomed" | "focused_pane_id">, floor: (paneId: string) => number): number {
  if (layout.zoomed) return floor(layout.focused_pane_id);
  const width = (node: LayoutNode): number => {
    if (node.type === "pane") return floor(node.pane_id);
    const first = width(node.first);
    const second = width(node.second);
    return node.direction === "down" ? Math.max(first, second) : Math.max(first / node.ratio, second / (1 - node.ratio));
  };
  return Math.ceil(width(layout.root));
}

/**
 * The same for a device's tab, whose engine reports each pane's rectangle
 * rather than the split tree (`RemotePaneLayout`): each pane draws at its
 * share of the tab's width.
 */
export function paneFramesWidth(layout: Pick<RemotePaneLayout, "frames" | "zoomed" | "focused_pane_id">, floor: (paneId: string) => number): number {
  if (layout.zoomed) return floor(layout.focused_pane_id);
  const whole = Math.max(0, ...layout.frames.map((frame) => frame.x + frame.width));
  const shares = layout.frames.filter((frame) => frame.width > 0).map((frame) => (floor(frame.pane_id) * whole) / frame.width);
  return Math.ceil(Math.max(0, ...shares));
}

/**
 * The least width Agent Views needs for every area to give the tab it shows
 * what `tabWidth` says that tab needs: a row of areas shares the width less
 * its divider at the split's ratio, as `areaGeometry` lays it out, and a
 * column gives each area the whole width.
 */
export function areaTreeWidth<I extends AreaItem>(root: AreaNode<I>, tabWidth: (areaId: string) => number, divider: number): number {
  const width = (node: AreaNode<I>): number => {
    if ("area" in node) return tabWidth(node.area.id);
    const { split } = node;
    const first = width(split.first);
    const second = width(split.second);
    return split.axis === "column" ? Math.max(first, second) : divider + Math.max(first / split.ratio, second / (1 - split.ratio));
  };
  return Math.ceil(width(root));
}
