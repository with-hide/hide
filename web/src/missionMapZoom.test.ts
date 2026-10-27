import { expect, it } from "vitest";
import { mapFit, mapLayout, stepMapZoom } from "./missionMapZoom";

// docs/UI_BEHAVIOR.md, A mission's map: 맞춤 shows the whole map in its box,
// never larger than its own size and never below 40%; − and + move in
// Chrome's steps between 40% and 200%.

it("fits the whole map in both directions, never enlarges a small one and stops at 40%", () => {
  // 16 px of gutter on each side: a 1000 x 400 map in a 532 x 600 box is held by its width (500 / 1000).
  expect(mapFit({ width: 1000, height: 400 }, { width: 532, height: 600 }, 0)).toBe(0.5);
  // A tall map in a short box is held by its height less the buttons' strip ((672 - 72) / 800).
  expect(mapFit({ width: 400, height: 800 }, { width: 1000, height: 672 }, 40)).toBe(0.75);
  expect(mapFit({ width: 200, height: 100 }, { width: 1200, height: 800 }, 40)).toBe(1);
  expect(mapFit({ width: 6000, height: 3000 }, { width: 600, height: 400 }, 40)).toBe(0.4);
});

it("steps in Chrome's steps from wherever the map is, held to 40% and 200%", () => {
  expect(stepMapZoom(0.63, "in")).toBeCloseTo(2 / 3);
  expect(stepMapZoom(0.63, "out")).toBe(0.5);
  expect(stepMapZoom(0.5, "out")).toBe(0.4);
  expect(stepMapZoom(0.4, "out")).toBe(0.4);
  expect(stepMapZoom(1.75, "in")).toBe(2);
  expect(stepMapZoom(2, "in")).toBe(2);
});

it("centres a map narrower than its box, starts a wider one at the gutter, and is as tall as it is drawn with the buttons' strip under it", () => {
  expect(mapLayout({ width: 400, height: 200 }, 0.5, { width: 1000, height: 900 }, 40)).toEqual({ width: 1000, height: 172, boxes: [{ left: 400, top: 16, width: 200, height: 100 }] });
  expect(mapLayout({ width: 400, height: 200 }, 2, { width: 300, height: 100 }, 40)).toEqual({ width: 832, height: 472, boxes: [{ left: 16, top: 16, width: 800, height: 400 }] });
});
