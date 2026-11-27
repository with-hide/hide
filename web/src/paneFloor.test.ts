import { describe, expect, it } from "vitest";
import type { AreaNode } from "./areaLayout";
import { areaTreeWidth, PANE_FLOOR, paneFramesWidth, paneTreeWidth, splitsRight } from "./paneFloor";
import type { LayoutNode } from "./snapshot";
import { laneFrame, type LaneInput, type LaneSizes } from "./workspace";

// docs/UI_BEHAVIOR.md, The pane floor: Agent Views keeps the larger of its
// 480px and what the front tab's panes need to keep the floor; a standing
// desk gives way down to its minimum, then floats over the panes.

const CELL = 8;
const FLOOR_PX = PANE_FLOOR.columns * CELL;
const floor = () => FLOOR_PX;
const pane = (id: string): LayoutNode => ({ type: "pane", pane_id: id });
const right = (ratio: number, first: LayoutNode, second: LayoutNode): LayoutNode => ({ type: "split", direction: "right", ratio, first, second });
const down = (first: LayoutNode, second: LayoutNode): LayoutNode => ({ type: "split", direction: "down", ratio: 0.5, first, second });
const tab = (root: LayoutNode, zoomed = false) => ({ root, zoomed, focused_pane_id: "p1" });

/** `count` panes side by side at equal shares, as splitting each new pane right at its share makes them. */
function row(count: number, from = 1): LayoutNode {
  return count === 1 ? pane(`p${from}`) : right(1 / count, pane(`p${from}`), row(count - 1, from + 1));
}

const SIZES: LaneSizes = { agentMin: 480, viewMin: 360, viewIdeal: 520, lanePage: 900, explorerMin: 180, explorerIdeal: 240, panelMin: 360, panelIdeal: 400, divider: 8 };

/** The desk shown beside Agent Views in a body `body` wide, Agent Views keeping what `need` says beside it. */
function desk(body: number, need: number) {
  const input: LaneInput = { open: true, view: "documents", tool: null, documentWidth: null, explorerWidth: null, panelWidth: null, expanded: false, front: "view", body, sizes: { ...SIZES, agentMin: Math.max(SIZES.agentMin, need) } };
  return laneFrame(input);
}

describe("the pane floor", () => {
  it("lets a pane split right only when both halves keep the floor", () => {
    expect(PANE_FLOOR.split_right_columns).toBe(2 * PANE_FLOOR.columns);
    expect(splitsRight(PANE_FLOOR.split_right_columns - 1)).toBe(false);
    expect(splitsRight(PANE_FLOOR.split_right_columns)).toBe(true);
  });

  it("counts every pane across a tab at its share of the width, and nothing for a split down", () => {
    expect(paneTreeWidth(tab(pane("p1")), floor)).toBe(FLOOR_PX);
    expect(paneTreeWidth(tab(row(7)), floor)).toBe(7 * FLOOR_PX);
    expect(paneTreeWidth(tab(down(row(2), row(3, 3))), floor)).toBe(3 * FLOOR_PX);
    // A pane at a third of the width needs three floors of it.
    expect(paneTreeWidth(tab(right(1 / 3, pane("p1"), pane("p2"))), floor)).toBe(3 * FLOOR_PX);
    // A zoomed tab draws its focused pane alone.
    expect(paneTreeWidth(tab(row(7), true), floor)).toBe(FLOOR_PX);
  });

  it("reads each pane at its own cell, so a pane drawn at a larger text scale needs more", () => {
    const scaled = (id: string) => (id === "p2" ? 2 * FLOOR_PX : FLOOR_PX);
    expect(paneTreeWidth(tab(row(2)), scaled)).toBe(4 * FLOOR_PX);
  });

  it("counts a device's tab from the rectangles its engine reports", () => {
    const frames = [
      { pane_id: "p1", x: 0, y: 0, width: 40, height: 24 },
      { pane_id: "p2", x: 40, y: 0, width: 20, height: 24 },
      { pane_id: "p3", x: 60, y: 0, width: 20, height: 24 },
    ];
    expect(paneFramesWidth({ frames, zoomed: false, focused_pane_id: "p1" }, floor)).toBe(4 * FLOOR_PX);
    expect(paneFramesWidth({ frames, zoomed: true, focused_pane_id: "p1" }, floor)).toBe(FLOOR_PX);
  });

  it("adds the Agent areas' divider between areas side by side", () => {
    const area = (id: string): AreaNode<{ id: string }> => ({ area: { id, active: null, displays: [] } });
    const areas: AreaNode<{ id: string }> = { split: { id: "s", axis: "row", ratio: 0.5, first: area("a1"), second: area("a2") } };
    expect(areaTreeWidth(areas, () => 300, 2)).toBe(602);
    expect(areaTreeWidth({ split: { id: "s", axis: "column", ratio: 0.5, first: area("a1"), second: area("a2") } }, () => 300, 2)).toBe(300);
  });

  it("keeps a standing desk beside a tab whose panes fit", () => {
    const frame = desk(1600, paneTreeWidth(tab(row(2)), floor));
    expect(frame.mode).toBe("docked");
    expect(frame.view?.width).toBe(520);
  });

  it("narrows the standing desk first, then floats it over a seven-pane tab, whose panes keep their width", () => {
    const need = paneTreeWidth(tab(row(7)), floor);
    const narrowed = desk(need + 8 + 400, need);
    expect(narrowed.mode).toBe("docked");
    expect(narrowed.view?.width).toBe(400);
    expect(narrowed.agents).toBe(need);
    const floating = desk(need + 8 + 359, need);
    expect(floating.mode).toBe("overlay");
    expect(floating.agents).toBe(need + 8 + 359);
  });
});
