import { afterEach, expect, it } from "vitest";
import { forgetScrolls, keepScroll, keptScroll, KEPT_SCROLLS } from "./conversationScroll";

afterEach(forgetScrolls);

it("keeps each conversation's place apart, forgets one left at its foot, and lets the one left longest ago go past the cap", () => {
  keepScroll("desk:1", { anchor: "12", offset: -40 });
  keepScroll("thread:w1:p2", { anchor: "u1:hi", offset: 8 });
  expect(keptScroll("desk:1")).toEqual({ anchor: "12", offset: -40 });
  expect(keptScroll("thread:w1:p2")).toEqual({ anchor: "u1:hi", offset: 8 });
  keepScroll("desk:1", null);
  expect(keptScroll("desk:1")).toBeNull();

  for (let n = 0; n < KEPT_SCROLLS; n += 1) keepScroll(`thread:p${n}`, { anchor: String(n), offset: 0 });
  // Leaving the first again makes it the newest, so the cap lets the next oldest go instead.
  keepScroll("thread:p0", { anchor: "0", offset: 4 });
  keepScroll("thread:extra", { anchor: "x", offset: 0 });
  expect(keptScroll("thread:w1:p2")).toBeNull();
  expect(keptScroll("thread:p1")).toBeNull();
  expect(keptScroll("thread:p0")).toEqual({ anchor: "0", offset: 4 });
  expect(keptScroll("thread:extra")).toEqual({ anchor: "x", offset: 0 });
});
