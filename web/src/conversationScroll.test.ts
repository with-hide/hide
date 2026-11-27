// @vitest-environment jsdom
import { afterEach, expect, it } from "vitest";
import { forgetScrolls, keepScroll, keptScroll, KEPT_SCROLLS, stillAt } from "./conversationScroll";

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

// Mission-109 review: a session beside Desk drawn again after ⌃⇥ away and back first empties its box while its lines
// are read again; the scroll that emptying makes is no one's, so the turn the operator was reading stays held.
it("holds a place while the conversation draws no item, and lets it go once other items are drawn without it", () => {
  const box = document.createElement("div");
  const place = { anchor: "u30:16", offset: 0 };
  expect(stillAt(box, place)).toBe(true);
  const other = document.createElement("div");
  other.dataset.scrollAnchor = "u40:21";
  box.append(other);
  expect(stillAt(box, place)).toBe(false);
});
