import type { AreaNode, AreaItem } from "./areaLayout";

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
