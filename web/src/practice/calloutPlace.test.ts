import { describe, expect, it } from "vitest";
import type { Box } from "./cardPlace";
import { placeTags } from "./calloutPlace";

const view: Box = { x: 0, y: 40, w: 1280, h: 760 };
const tag = { w: 120, h: 26 };
const touches = (a: Box, b: Box) => a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
const boxOf = (place: { x: number; y: number }): Box => ({ x: place.x, y: place.y, ...tag });

describe("placeTags", () => {
  it("stands a tag beside its box, to the right, when that is free", () => {
    const [place] = placeTags([{ x: 100, y: 200, w: 40, h: 20 }], [tag], [], view);
    expect(place!.side).toBe("right");
    expect(place!.x).toBeGreaterThanOrEqual(140);
    expect(place!.x).toBeLessThanOrEqual(150);
  });

  it("puts a tag on another side when the right would cover the next box", () => {
    const boxes: Box[] = [{ x: 100, y: 200, w: 40, h: 20 }, { x: 150, y: 196, w: 40, h: 28 }];
    const [first] = placeTags(boxes, [tag, tag], [], view);
    expect(first!.side).not.toBe("right");
    expect(touches(boxOf(first!), boxes[1]!)).toBe(false);
  });

  it("never lets two tags or a tag and a box touch when there is room", () => {
    const boxes: Box[] = [{ x: 290, y: 540, w: 46, h: 18 }, { x: 290, y: 568, w: 46, h: 18 }, { x: 250, y: 596, w: 46, h: 18 }, { x: 300, y: 330, w: 40, h: 18 }];
    const places = placeTags(boxes, boxes.map(() => tag), [], view).map(boxOf);
    places.forEach((place, at) => {
      boxes.forEach((box) => expect(touches(place, box)).toBe(false));
      places.forEach((another, other) => { if (other !== at) expect(touches(place, another)).toBe(false); });
    });
  });

  it("keeps a tag inside the view and off the avoided bar", () => {
    const bar: Box = { x: 400, y: 0, w: 480, h: 40 };
    const [place] = placeTags([{ x: 600, y: 44, w: 40, h: 18 }], [tag], [bar], view);
    expect(touches(boxOf(place!), bar)).toBe(false);
    expect(place!.y).toBeGreaterThanOrEqual(view.y);
  });
});
