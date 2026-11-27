import { describe, expect, it } from "vitest";
import type { AreaNode } from "./areaLayout";
import { areaTreeWidth } from "./paneFloor";
import { laneFrame, type LaneInput, type LaneSizes } from "./workspace";

// docs/UI_BEHAVIOR.md, The pane floor: Agent Views keeps the larger of its
// 480px and the least width the core says the front tab's panes need; a
// standing desk gives way down to its minimum, then floats over the panes.
// The tab's own width is the core's arithmetic (`hide-core`
// `runtime::pane_floor`); here it is a given number.

/** The least width of one pane at the floor: 18 cells of 8 px and an 8 px inset each side. */
const FLOOR_PX = 18 * 8 + 16;

const SIZES: LaneSizes = { agentMin: 480, viewMin: 360, viewIdeal: 520, lanePage: 900, explorerMin: 180, explorerIdeal: 240, panelMin: 360, panelIdeal: 400, divider: 8 };

/** The desk shown beside Agent Views in a body `body` wide, Agent Views keeping what `need` says beside it. */
function desk(body: number, need: number) {
  const input: LaneInput = { open: true, view: "documents", tool: null, documentWidth: null, explorerWidth: null, panelWidth: null, expanded: false, front: "view", body, sizes: { ...SIZES, agentMin: Math.max(SIZES.agentMin, need) } };
  return laneFrame(input);
}

describe("the pane floor", () => {
  it("adds the Agent areas' divider between areas side by side", () => {
    const area = (id: string): AreaNode<{ id: string }> => ({ area: { id, active: null, displays: [] } });
    const areas: AreaNode<{ id: string }> = { split: { id: "s", axis: "row", ratio: 0.5, first: area("a1"), second: area("a2") } };
    expect(areaTreeWidth(areas, () => 300, 2)).toBe(602);
    expect(areaTreeWidth({ split: { id: "s", axis: "column", ratio: 0.5, first: area("a1"), second: area("a2") } }, () => 300, 2)).toBe(300);
  });

  it("keeps a standing desk beside a tab whose panes fit", () => {
    const frame = desk(1600, 2 * FLOOR_PX);
    expect(frame.mode).toBe("docked");
    expect(frame.view?.width).toBe(520);
  });

  it("narrows the standing desk first, then floats it over a seven-pane tab, whose panes keep their width", () => {
    const need = 7 * FLOOR_PX;
    const narrowed = desk(need + 8 + 400, need);
    expect(narrowed.mode).toBe("docked");
    expect(narrowed.view?.width).toBe(400);
    expect(narrowed.agents).toBe(need);
    const floating = desk(need + 8 + 359, need);
    expect(floating.mode).toBe("overlay");
    expect(floating.agents).toBe(need + 8 + 359);
  });
});
