import ELK from "elkjs/lib/elk.bundled.js";
import { describe, expect, it } from "vitest";
import { placeCompoundGraph, type CompoundGroup } from "./dependencyLayout";

const spacing = { betweenLayers: 24, betweenNodes: 24, padding: 24, header: 110, groups: 90 };
const group = (id: string, children: string[] = []): CompoundGroup => ({ id, width: 360, height: 170, children: children.map((node) => ({ id: node, width: 188, height: 184 })), edges: [] });

describe("project compounds", () => {
  it("packs multiple independent roots inside a project and keeps collapsed projects clear of it", async () => {
    const open = group("open", ["root-a", "root-b", "child"]);
    open.edges.push({ from: "root-a", to: "child" });
    const placed = await placeCompoundGraph(new ELK(), [open, group("closed-a"), group("closed-b")], spacing);
    expect(placed.groups.size).toBe(3);
    expect(placed.nodes.size).toBe(3);
    const box = placed.groups.get("open")!;
    for (const point of placed.nodes.values()) {
      expect(point.x).toBeGreaterThanOrEqual(spacing.padding);
      expect(point.y).toBeGreaterThanOrEqual(spacing.header);
      expect(point.x + 188).toBeLessThanOrEqual(box.width - spacing.padding);
      expect(point.y + 184).toBeLessThanOrEqual(box.height - spacing.padding);
    }
    const boxes = [...placed.groups.values()];
    for (const a of boxes) for (const b of boxes) if (a.id !== b.id) expect(a.x + a.width <= b.x || b.x + b.width <= a.x || a.y + a.height <= b.y || b.y + b.height <= a.y).toBe(true);
    expect(placed.nodes.get("child")!.x).toBeGreaterThan(placed.nodes.get("root-a")!.x);
  });

  it("rejects a compound beyond 400 including its parent group", async () => {
    await expect(placeCompoundGraph(new ELK(), [group("too-large", Array.from({ length: 400 }, (_, i) => `p${i}`))], spacing)).rejects.toThrow(/over the layout limit/);
  });
});
