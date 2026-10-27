import { describe, expect, it } from "vitest";
import { adjacentDisplay, tabStripFit, type AreaNode, type TabStripSizes } from "./areaLayout";

// The tokens' values: preferred 180, title minimum 104, icon identity 40, control 24.
const SIZES: TabStripSizes = { preferred: 180, titleMin: 104, icon: 40, control: 24 };
const fit = (room: number, count: number, hasSelected = true) => tabStripFit(room, count, hasSelected, SIZES);
const drawn = (room: number, count: number) => {
  const { selected, others } = fit(room, count);
  return `${round(selected.width)} ${selected.fit} / ${round(others.width)} ${others.fit}`;
};
const round = (width: number) => Math.round(width * 100) / 100;

describe("tabStripFit", () => {
  it("shrinks in three stages, the selected tab keeping its title longest", () => {
    expect([
      drawn(1200, 6), // 1. room to spare: every tab asks for the preferred width
      drawn(720, 6), // 1. an equal share of 120 still holds a title
      drawn(485, 6), // 2. the issue's bar: selected keeps 104, five others share 381
      drawn(395, 6), // 2. others at 58.2 no longer hold a control's worth of title
      drawn(304, 6), // 2. others reach the icon identity exactly
      drawn(290, 6), // 3. the selected tab gives up width, still compact with its close
      drawn(287, 6), // 3. 87 is below marks and two controls: marks with close at 64
      drawn(200, 6), // 3. even marks overflow, so the strip scrolls
    ]).toEqual([
      "180 titled / 180 titled",
      "120 titled / 120 titled",
      "104 titled / 76.2 compact",
      "104 titled / 58.2 marks",
      "104 titled / 40 marks",
      "90 compact / 40 marks",
      "64 marks / 40 marks",
      "64 marks / 40 marks",
    ]);
  });

  it("gives a lone tab the same rules", () => {
    expect([drawn(400, 1), drawn(150, 1), drawn(100, 1), drawn(88, 1), drawn(87, 1), drawn(30, 1)]).toEqual([
      "180 titled / 180 titled",
      "150 titled / 150 titled",
      "100 compact / 40 marks",
      "88 compact / 40 marks",
      "64 marks / 40 marks",
      "64 marks / 40 marks",
    ]);
  });

  it("fills the bar to its end until only marks are left, and never jumps in width", () => {
    for (const count of [1, 2, 3, 6, 16]) {
      let previous = fit(count * 200, count);
      for (let room = count * 200; room >= 0; room -= 0.5) {
        const next = fit(room, count);
        const total = next.selected.width + (count - 1) * next.others.width;
        const marksOnly = next.selected.fit === "marks";
        if (room / count <= SIZES.preferred && !marksOnly) expect(total).toBeCloseTo(room, 6);
        if (marksOnly) expect(room - total).toBeLessThan(SIZES.control);
        expect(Math.abs(next.selected.width - previous.selected.width)).toBeLessThanOrEqual(
          // The one step: the selected tab turns to its 64 marks below 88.
          previous.selected.fit === "compact" && next.selected.fit === "marks" ? SIZES.control : 0.5 + 1e-9,
        );
        // A lone tab has no others.
        if (count > 1) expect(Math.abs(next.others.width - previous.others.width)).toBeLessThanOrEqual(0.5 + 1e-9);
        previous = next;
      }
    }
  });

  it("shares the room alike when no tab of the strip is selected", () => {
    expect(fit(485, 6, false)).toEqual({ selected: { width: 485 / 6, fit: "compact" }, others: { width: 485 / 6, fit: "compact" } });
    expect(fit(200, 6, false).others).toEqual({ width: 40, fit: "marks" });
  });
});

describe("adjacentDisplay", () => {
  const strip = (active: string, ids: string[]): AreaNode<{ id: string }> => ({ area: { id: "a1", active, displays: ids.map((id) => ({ id })) } });
  const root: AreaNode<{ id: string }> = { split: { id: "s1", axis: "row", ratio: 0.5, first: strip("b", ["a", "b", "c"]), second: { area: { id: "a2", active: "x", displays: [{ id: "x" }] } } } };

  it("steps along the strip of the area it names, wrapping at either end", () => {
    expect(adjacentDisplay(root, "a1", 1)?.id).toBe("c");
    expect(adjacentDisplay(root, "a1", -1)?.id).toBe("a");
    expect(adjacentDisplay(strip("c", ["a", "b", "c"]), "a1", 1)?.id).toBe("a");
    expect(adjacentDisplay(strip("a", ["a", "b", "c"]), "a1", -1)?.id).toBe("c");
  });

  it("has nowhere to go with one tab, an unknown area or no tab in front", () => {
    expect(adjacentDisplay(root, "a2", 1)).toBeNull();
    expect(adjacentDisplay(root, "a9", 1)).toBeNull();
    expect(adjacentDisplay(strip("gone", ["a", "b"]), "a1", 1)).toBeNull();
  });
});
