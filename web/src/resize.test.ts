import { describe, expect, it } from "vitest";
import { dividerPaneId, resizeStep } from "./resize";
import type { LayoutNode } from "./snapshot";

describe("resizeStep", () => {
  it("names the direction and the fraction of the split span", () => {
    expect(resizeStep(100, 1000, true)).toEqual({ direction: "right", amount: 0.1 });
    expect(resizeStep(-250, 1000, true)).toEqual({ direction: "left", amount: 0.25 });
    expect(resizeStep(50, 500, false)).toEqual({ direction: "down", amount: 0.1 });
    expect(resizeStep(-5, 500, false)).toEqual({ direction: "up", amount: 0.01 });
  });

  it("sends nothing outside the core's accepted range", () => {
    expect(resizeStep(0, 1000, true)).toBeNull();
    expect(resizeStep(0.5, 1000, true)).toBeNull();
    expect(resizeStep(600, 1000, true)).toBeNull();
    expect(resizeStep(-600, 1000, false)).toBeNull();
    expect(resizeStep(500, 1000, true)).toEqual({ direction: "right", amount: 0.5 });
  });
});

type Split = Extract<LayoutNode, { type: "split" }>;
const pane = (id: string): LayoutNode => ({ type: "pane", pane_id: id });
const right = (first: LayoutNode, second: LayoutNode): Split => ({ type: "split", direction: "right", ratio: 0.5, first, second });
const down = (first: LayoutNode, second: LayoutNode): Split => ({ type: "split", direction: "down", ratio: 0.5, first, second });

describe("dividerPaneId", () => {
  it("names the pane just before the divider toward right or down and the one just after it toward left or up", () => {
    // ((A|B)|C), issue 2346: the divider between B and C is B's right edge and C's left edge.
    const leftNested = right(right(pane("A"), pane("B")), pane("C"));
    expect(dividerPaneId(leftNested, "right")).toBe("B");
    expect(dividerPaneId(leftNested, "left")).toBe("C");
    // (A|(B|C)): the divider between A and B.
    const rightNested = right(pane("A"), right(pane("B"), pane("C")));
    expect(dividerPaneId(rightNested, "right")).toBe("A");
    expect(dividerPaneId(rightNested, "left")).toBe("B");
  });

  it("walks through perpendicular splits on either side to the pane that touches the divider", () => {
    // (A|(X/Y)) | ((P/Q)|C): the root divider is Y's right edge and P's left edge.
    const mixed = right(right(pane("A"), down(pane("X"), pane("Y"))), right(down(pane("P"), pane("Q")), pane("C")));
    expect(dividerPaneId(mixed, "right")).toBe("Y");
    expect(dividerPaneId(mixed, "left")).toBe("P");
    const stacked = down(down(pane("A"), pane("B")), right(pane("C"), pane("D")));
    expect(dividerPaneId(stacked, "down")).toBe("B");
    expect(dividerPaneId(stacked, "up")).toBe("C");
  });
});
