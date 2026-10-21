import ELK from "elkjs/lib/elk.bundled.js";
import { describe, expect, it } from "vitest";
import { placeCompoundGraph, type CompoundGroup } from "./dependencyLayout";

const spacing = { betweenLayers: 24, betweenNodes: 24, padding: 24, header: 110, groups: 90 };
const group = (id: string, children: string[] = []): CompoundGroup => ({ id, width: 360, height: 170, children: children.map((node) => ({ id: node, width: 188, height: 184 })), edges: [] });

describe("project compounds", () => {
  it("places sessions below the project heading without overlapping labels or another project", async () => {
    const open = group("open", ["root-a", "root-b", "child"]);
    open.edges.push({ from: "root-a", to: "child" });
    const placed = await placeCompoundGraph(new ELK(), [open, group("closed-a"), group("closed-b")], spacing);
    expect(placed.groups.size).toBe(3);
    expect(placed.nodes.size).toBe(3);
    const box = placed.groups.get("open")!;
    for (const point of placed.nodes.values()) {
      expect(point.x).toBeGreaterThanOrEqual(spacing.padding);
      expect(point.y).toBeGreaterThanOrEqual(spacing.padding + open.height + spacing.betweenNodes);
      expect(point.x + 188).toBeLessThanOrEqual(box.width - spacing.padding);
      expect(point.y + 184).toBeLessThanOrEqual(box.height - spacing.padding);
    }
    const boxes = [...placed.groups.values()];
    for (const a of boxes) for (const b of boxes) if (a.id !== b.id) expect(a.x + a.width <= b.x || b.x + b.width <= a.x || a.y + a.height <= b.y || b.y + b.height <= a.y).toBe(true);
    // Approved radial contract B1 replaces the former tree's child-right-of-parent rule.
    const points = [...placed.nodes.values()];
    for (const a of points) for (const b of points) if (a !== b) expect(a.x + 188 <= b.x || b.x + 188 <= a.x || a.y + 184 <= b.y || b.y + 184 <= a.y).toBe(true);
    expect(points.some((point) => point.x < box.width / 2 - 188)).toBe(true);
    expect(points.some((point) => point.x > box.width / 2)).toBe(true);
  });

  it("rejects a compound beyond 400 including its parent group", async () => {
    await expect(placeCompoundGraph(new ELK(), [group("too-large", Array.from({ length: 400 }, (_, i) => `p${i}`))], spacing)).rejects.toThrow(/over the layout limit/);
  });

  it.each([11, 48])("keeps all %i session rectangles clear of each other and the top project heading", async (count) => {
    const input = group("project", Array.from({ length: count }, (_, index) => `session${index}`));
    const placed = await placeCompoundGraph(new ELK(), [input], spacing);
    const box = placed.groups.get(input.id)!;
    const rectangles = [...placed.nodes.values()].map((point) => ({ x: point.x, y: point.y, width: 188, height: 184 }));
    rectangles.push({ x: (box.width - input.width) / 2, y: spacing.padding, width: input.width, height: input.height });
    for (const a of rectangles) for (const b of rectangles) if (a !== b) expect(a.x + a.width <= b.x || b.x + b.width <= a.x || a.y + a.height <= b.y || b.y + b.height <= a.y).toBe(true);
  });
});
