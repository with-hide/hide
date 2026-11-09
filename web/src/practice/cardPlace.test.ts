import { expect, it } from "vitest";
import { placeCard, type Box } from "./cardPlace";

const VIEW = { w: 1280, h: 800 };
const CARD = { w: 380, h: 220 };
const covers = (at: { x: number; y: number }, box: Box) => at.x < box.x + box.w && at.x + CARD.w > box.x && at.y < box.y + box.h && at.y + CARD.h > box.y;

it("floats beside a target without covering it, wherever the target is", () => {
  for (const target of [
    { x: 40, y: 80, w: 330, h: 600 },
    { x: 880, y: 40, w: 360, h: 700 },
    { x: 400, y: 100, w: 420, h: 280 },
  ]) {
    const at = placeCard(CARD, [target], [], VIEW);
    expect(covers(at, target)).toBe(false);
    expect(at.x).toBeGreaterThanOrEqual(0);
    expect(at.x + CARD.w).toBeLessThanOrEqual(VIEW.w);
    expect(at.y + CARD.h).toBeLessThanOrEqual(VIEW.h);
  }
});

it("keeps clear of every one of several targets and of the layer's own parts", () => {
  const targets = [{ x: 40, y: 300, w: 330, h: 60 }, { x: 880, y: 40, w: 360, h: 700 }];
  const bar = { x: 450, y: 6, w: 380, h: 30 };
  const at = placeCard(CARD, targets, [bar], VIEW);
  for (const box of [...targets, bar]) expect(covers(at, box)).toBe(false);
});

it("sits in the middle of the screen when nothing is pointed at", () => {
  expect(placeCard(CARD, [], [], VIEW)).toEqual({ x: 450, y: 290 });
});
