// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import { followVisualViewport, VIEWPORT_HEIGHT } from "./viewport";

// A browser keeps its page as tall when the keyboard shows and shrinks only the visual viewport; the page takes its height.

function fakeViewport(height: number) {
  const target = new EventTarget() as EventTarget & { height: number };
  target.height = height;
  return target as unknown as VisualViewport & { height: number };
}

describe("the phone page's height", () => {
  it("follows the visual viewport while the keyboard comes and goes, and lets go when stopped", () => {
    const viewport = fakeViewport(844);
    const root = document.createElement("html");
    const scrollTo = vi.fn();
    const stop = followVisualViewport({ visualViewport: viewport, scrollTo }, root);
    expect(root.style.getPropertyValue(VIEWPORT_HEIGHT)).toBe("844px");
    viewport.height = 508;
    viewport.dispatchEvent(new Event("resize"));
    expect(root.style.getPropertyValue(VIEWPORT_HEIGHT)).toBe("508px");
    expect(scrollTo).toHaveBeenLastCalledWith(0, 0);
    stop();
    expect(root.style.getPropertyValue(VIEWPORT_HEIGHT)).toBe("");
    viewport.height = 300;
    viewport.dispatchEvent(new Event("resize"));
    expect(root.style.getPropertyValue(VIEWPORT_HEIGHT)).toBe("");
  });

  it("leaves the page alone where the browser has no visual viewport", () => {
    const root = document.createElement("html");
    followVisualViewport({ visualViewport: null, scrollTo: vi.fn() }, root)();
    expect(root.style.getPropertyValue(VIEWPORT_HEIGHT)).toBe("");
  });
});
