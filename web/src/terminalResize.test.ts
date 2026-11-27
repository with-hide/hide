import { expect, it } from "vitest";
import { resizeEvent, type FloorMeasure } from "./terminals";

// docs/UI_BEHAVIOR.md, The pane floor: the core judges a split from what the
// window measured, sent with each grid as `terminal_resize`'s `floor`.

const floor: FloorMeasure = { cell_px: 8, canvas_px: 320, inset_px: 8, base_cell_px: 8, scale: 1, font_size: 14, font_family: "" };

it("sends a device's pane its measure with its grid, as a pane of this machine", () => {
  for (const pane of ["p1", "remote:dev-a:pane:7"]) {
    expect(resizeEvent(pane, 30, 24, floor)).toEqual({ schema_version: 2, kind: "terminal_resize", payload: { pane_id: pane, cols: 30, rows: 24, floor } });
  }
});

it("sends no resize for another machine's pane running here, which its own node sizes", () => {
  expect(resizeEvent("guest:dev-a:pane:7", 30, 24, floor)).toBeNull();
});

it("sends a grid with no measure as one, which the core reads as not measured", () => {
  expect(resizeEvent("p1", 30, 24, null)?.payload).toEqual({ pane_id: "p1", cols: 30, rows: 24 });
});
