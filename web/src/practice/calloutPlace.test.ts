import { describe, expect, it } from "vitest";
import { placeCallouts } from "./calloutPlace";

describe("placeCallouts", () => {
  const view = { top: 40, bottom: 780 };
  it("keeps each tag level with its target when they are apart", () => {
    expect(placeCallouts([100, 300], 24, 8, view)).toEqual([88, 288]);
  });
  it("pushes tags that would touch down in their targets' order", () => {
    const tops = placeCallouts([300, 304, 310], 24, 8, view);
    expect(tops[1]! - tops[0]!).toBe(32);
    expect(tops[2]! - tops[1]!).toBe(32);
  });
  it("lifts the column back inside the view when it runs past the bottom", () => {
    const tops = placeCallouts([700, 740, 770], 24, 8, view);
    expect(Math.max(...tops) + 24).toBeLessThanOrEqual(780);
    expect(tops[1]! - tops[0]!).toBeGreaterThanOrEqual(32);
  });
});
