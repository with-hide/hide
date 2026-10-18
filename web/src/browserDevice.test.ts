import { describe, expect, it } from "vitest";
import { deviceLabel, devicePlacement } from "./browserDevice";

describe("a browser display's device size (PRD browser-width-zoom)", () => {
  it("lays a page out at a phone's, a tablet's or a desktop's width, the first two as phones lay out (B11)", () => {
    expect(devicePlacement("fit")).toBeNull();
    expect(devicePlacement("mobile")).toEqual({ width: 390, mobile: true });
    expect(devicePlacement("tablet")).toEqual({ width: 768, mobile: true });
    expect(devicePlacement("desktop")).toEqual({ width: 1280, mobile: false });
  });

  it("names nothing while the page fits, the width at its own size, and the scale when the area is narrower (B10, B12)", () => {
    expect(deviceLabel("fit", 500)).toBeNull();
    expect(deviceLabel("mobile", 390)).toBe("390");
    expect(deviceLabel("tablet", 0)).toBe("768");
    expect(deviceLabel("desktop", 496)).toBe("1280 · 39%");
    expect(deviceLabel("tablet", 700)).toBe("768 · 91%");
  });
});
