// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { followVisualViewport, VIEWPORT_HEIGHT } from "./viewport";

// A browser keeps its page as tall when the keyboard shows and shrinks only the visual viewport; the page takes its height.
// The visual viewport also shrinks and grows while a list scrolls, as the browser folds its toolbar away and brings it back.

function fakeViewport(height: number) {
  const target = new EventTarget() as EventTarget & { height: number };
  target.height = height;
  return target as unknown as VisualViewport & { height: number };
}

function resize(viewport: VisualViewport & { height: number }, height: number) {
  viewport.height = height;
  viewport.dispatchEvent(new Event("resize"));
}

afterEach(() => {
  document.body.replaceChildren();
});

describe("the phone page's height", () => {
  it("follows the visual viewport while the keyboard comes and goes, and lets go when stopped", () => {
    const viewport = fakeViewport(844);
    const root = document.createElement("html");
    const stop = followVisualViewport({ visualViewport: viewport, scrollTo: vi.fn() }, root);
    expect(root.style.getPropertyValue(VIEWPORT_HEIGHT)).toBe("844px");
    resize(viewport, 508);
    expect(root.style.getPropertyValue(VIEWPORT_HEIGHT)).toBe("508px");
    stop();
    expect(root.style.getPropertyValue(VIEWPORT_HEIGHT)).toBe("");
    resize(viewport, 300);
    expect(root.style.getPropertyValue(VIEWPORT_HEIGHT)).toBe("");
  });

  it("takes back the browser's move of the page when the keyboard comes up for a reply", () => {
    const viewport = fakeViewport(844);
    const scrollTo = vi.fn();
    const reply = document.body.appendChild(document.createElement("textarea"));
    followVisualViewport({ visualViewport: viewport, scrollTo }, document.createElement("html"));
    reply.focus();
    resize(viewport, 508);
    expect(scrollTo).toHaveBeenLastCalledWith(0, 0);
  });

  it("leaves a list where the reader scrolled it while the toolbar folds away and comes back", () => {
    const viewport = fakeViewport(745);
    const scrollTo = vi.fn();
    // A pressed row or tab keeps the focus on a phone; it raises no keyboard.
    const row = document.body.appendChild(document.createElement("button"));
    row.focus();
    followVisualViewport({ visualViewport: viewport, scrollTo }, document.createElement("html"));
    resize(viewport, 812);
    resize(viewport, 745);
    expect(scrollTo).not.toHaveBeenCalled();
  });

  it("leaves the page alone where the browser has no visual viewport", () => {
    const root = document.createElement("html");
    followVisualViewport({ visualViewport: null, scrollTo: vi.fn() }, root)();
    expect(root.style.getPropertyValue(VIEWPORT_HEIGHT)).toBe("");
  });
});
