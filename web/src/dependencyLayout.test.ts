import ELK from "elkjs/lib/elk.bundled.js";
import { describe, expect, it, vi } from "vitest";
import { LAYOUT_NODE_LIMIT, downPathAround, edgePath, placeGraph, placeWithin, squarePath, type Box, type NodeBox } from "./dependencyLayout";

const SPACING = { betweenLayers: 96, betweenNodes: 24 };
const card = (id: string): NodeBox => ({ id, width: 288, height: 120 });

/** Points along a path drawn by `edgePath`: its move, then each cubic step sampled. */
function samples(d: string): { x: number; y: number }[] {
  const numbers = d.match(/-?\d+(\.\d+)?/g)!.map(Number);
  let at = { x: numbers[0]!, y: numbers[1]! };
  const points = [at];
  for (let index = 2; index + 5 < numbers.length + 0.5; index += 6) {
    const [c1x, c1y, c2x, c2y, x, y] = numbers.slice(index, index + 6) as [number, number, number, number, number, number];
    for (let step = 1; step <= 40; step += 1) {
      const t = step / 40;
      const u = 1 - t;
      points.push({ x: u ** 3 * at.x + 3 * u ** 2 * t * c1x + 3 * u * t ** 2 * c2x + t ** 3 * x, y: u ** 3 * at.y + 3 * u ** 2 * t * c1y + 3 * u * t ** 2 * c2y + t ** 3 * y });
    }
    at = { x, y };
  }
  return points;
}

describe("a mission map's top-down flow", () => {
  it("puts a node below what it waits on and routes every edge in square legs that never cross a card", async () => {
    const nodes = ["cause", "fix", "api", "screen", "report"].map(card);
    const edges = [
      { from: "cause", to: "fix" },
      { from: "api", to: "screen" },
      { from: "fix", to: "report" },
      { from: "screen", to: "report" },
      { from: "api", to: "report" },
    ];
    const placement = await placeGraph(new ELK(), nodes, edges, SPACING, "down");
    const at = (id: string) => placement.nodes.get(id)!;
    expect(at("fix").y).toBeGreaterThan(at("cause").y);
    expect(at("report").y).toBeGreaterThan(at("screen").y);
    expect(at("report").layer).toBe(2);
    for (const edge of placement.edges) {
      expect(edge.d).not.toContain(" C ");
      const [from, to] = edge.id.split(">") as [string, string];
      const points = edge.d.match(/-?\d+(\.\d+)?/g)!.map(Number);
      for (const node of nodes) {
        if (node.id === from || node.id === to) continue;
        const place = at(node.id);
        for (let index = 0; index + 1 < points.length; index += 2) {
          const [x, y] = [points[index]!, points[index + 1]!];
          expect(x > place.x && x < place.x + node.width && y > place.y && y < place.y + node.height, `${edge.id} passes through ${node.id}`).toBe(false);
        }
      }
    }
  });

  it("centres a node over the row it fans out to and the card that gathers that row", async () => {
    const parts = ["a", "b", "c", "d", "e", "f"];
    const nodes = ["base", ...parts, "report"].map(card);
    const edges = [...parts.map((part) => ({ from: "base", to: part })), ...parts.map((part) => ({ from: part, to: "report" }))];
    const placement = await placeGraph(new ELK(), nodes, edges, SPACING, "down");
    const middle = (id: string) => placement.nodes.get(id)!.x + card(id).width / 2;
    const row = parts.map(middle);
    const centre = (Math.min(...row) + Math.max(...row)) / 2;
    expect(middle("base")).toBeCloseTo(centre, 0);
    expect(middle("report")).toBeCloseTo(centre, 0);
  });

  it("rounds a corner by the radius, and by half the shorter leg when a jog is shorter than that", () => {
    expect(squarePath([{ x: 0, y: 0 }, { x: 0, y: 40 }, { x: 40, y: 40 }], 8)).toBe("M 0 0 L 0 32 Q 0 40, 8 40 L 40 40");
    expect(squarePath([{ x: 0, y: 0 }, { x: 0, y: 6 }, { x: 40, y: 6 }], 8)).toBe("M 0 0 L 0 3 Q 0 6, 3 6 L 40 6");
    expect(squarePath([{ x: 0, y: 0 }, { x: 0, y: 40 }], 8)).toBe("M 0 0 L 0 40");
  });
});

describe("a top-down graph before the layout places it (PRD mission-pane-chip B7)", () => {
  // The small map's first plan as the columns draw it: two cards, one below the first, the report under both.
  const box = (left: number, top: number): Box => ({ left, right: left + 144, top, bottom: top + 30 });
  const first = box(0, 0);
  const second = box(152, 0);
  const middle = box(0, 60);
  const report = box(76, 120);
  /** The corners of a square path: every coordinate pair it names, each on the route. */
  const corners = (d: string) => {
    const numbers = d.match(/-?\d+(\.\d+)?/g)!.map(Number);
    return Array.from({ length: numbers.length / 2 }, (_, index) => ({ x: numbers[index * 2]!, y: numbers[index * 2 + 1]! }));
  };
  const inside = (point: { x: number; y: number }, card: Box) => point.x > card.left && point.x < card.right && point.y > card.top && point.y < card.bottom;

  it("goes around the cards of a row it skips, on the side nearer the card it leaves", () => {
    const d = downPathAround(second, report, [first, second, middle, report], 4);
    expect(d).not.toContain(" C ");
    const route = corners(d);
    for (const point of route) expect(inside(point, middle), `${JSON.stringify(point)} is inside the middle card`).toBe(false);
    // Its run down the side passes the middle row clear of the card, by the gap.
    expect(route.some((point) => point.x === middle.right + 4 && point.y > middle.top)).toBe(true);
    // From the left card it takes the left side.
    expect(corners(downPathAround(first, report, [first, middle, report], 4)).some((point) => point.x === middle.left - 4)).toBe(true);
  });

  it("stays a curve between neighbouring rows, where only the gap lies between", () => {
    expect(downPathAround(first, middle, [first, second, middle, report], 4)).toBe("M 72 30 C 72 45, 72 45, 72 60");
  });
});

describe("placeGraph", () => {
  it("keeps each layer's cards in the order given when asked, as a row's first card and then its +N", async () => {
    // Folded roots: the kept card, then the +N card a deeper node hangs from; free crossing minimization may swap them.
    const nodes = ["kept", "more", "screen", "report"].map(card);
    const edges = [
      { from: "more", to: "screen" },
      { from: "kept", to: "report" },
      { from: "screen", to: "report" },
    ];
    const placement = await placeGraph(new ELK(), nodes, edges, SPACING, "down", "given");
    expect(placement.nodes.get("kept")!.x).toBeLessThan(placement.nodes.get("more")!.x);
  });

  it("routes a long edge around the cards of the columns it passes, not across them", async () => {
    // L-1 waits on nothing; L-2 on L-1; L-3 on L-2; L-4 on L-3; L-5 on L-1 and L-4, so L-1 to L-5 spans three columns.
    const nodes = ["L-1", "L-2", "L-3", "L-4", "L-5", "L-6"].map(card);
    const edges = [
      { from: "L-1", to: "L-2" },
      { from: "L-2", to: "L-3" },
      { from: "L-3", to: "L-4" },
      { from: "L-4", to: "L-5" },
      { from: "L-1", to: "L-5" },
      { from: "L-2", to: "L-6" },
    ];
    const placement = await placeGraph(new ELK(), nodes, edges, SPACING);
    const long = placement.edges.find((edge) => edge.id === "L-1>L-5")!;
    expect(long.d).toMatch(/^M \S+ \S+ C /);
    for (const node of nodes) {
      if (node.id === "L-1" || node.id === "L-5") continue;
      const place = placement.nodes.get(node.id)!;
      const inside = samples(long.d).filter((point) => point.x > place.x && point.x < place.x + node.width && point.y > place.y && point.y < place.y + node.height);
      expect(inside, `the L-1 to L-5 edge crosses ${node.id}`).toEqual([]);
    }
    expect(new Set([...placement.nodes.values()].map((node) => node.layer)).size).toBe(5);
  });

  it("refuses more cards than the layout limit, so the columns stay", async () => {
    const nodes = Array.from({ length: LAYOUT_NODE_LIMIT + 1 }, (_, index) => card(`T-${index}`));
    await expect(placeGraph(new ELK(), nodes, [], SPACING)).rejects.toThrow(/over the layout limit/);
  });
});

describe("placeWithin", () => {
  it("gives up on a layout that does not answer in time and ends its worker", async () => {
    vi.useFakeTimers();
    try {
      const elk = { layout: () => new Promise<never>(() => {}), terminateWorker: vi.fn() };
      const placed = placeWithin(elk, [card("A")], [], SPACING, 50);
      const refused = expect(placed).rejects.toThrow(/longer than 50 ms/);
      await vi.advanceTimersByTimeAsync(51);
      await refused;
      expect(elk.terminateWorker).toHaveBeenCalledOnce();
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("edgePath", () => {
  it("passes through every routed point and leaves and enters each one level", () => {
    expect(edgePath([{ x: 0, y: 0 }, { x: 100, y: 0 }, { x: 200, y: 50 }])).toBe("M 0 0 C 50 0, 50 0, 100 0 C 150 0, 150 50, 200 50");
  });
});
