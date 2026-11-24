// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { followVisualViewport, VIEWPORT_HEIGHT } from "./viewport";

// A browser keeps its page as tall when the keyboard shows and shrinks only the visual viewport; the page takes its height.
// The visual viewport also shrinks and grows while a list scrolls, as the browser folds its toolbar away and brings it back.
// The page's place is what the reader sees: `scrolled` is how far it is scrolled, by the reader, by the browser, or back by the app.

function fakeViewport(height: number) {
  const target = new EventTarget() as EventTarget & { height: number };
  target.height = height;
  return target as unknown as VisualViewport & { height: number };
}

function fakeView(viewport: VisualViewport | null) {
  const view = new EventTarget() as EventTarget & { visualViewport: VisualViewport | null; scrolled: number; scrollTo: (x: number, y: number) => void };
  view.visualViewport = viewport;
  view.scrolled = 0;
  view.scrollTo = (_x, y) => {
    view.scrolled = y;
  };
  return view as unknown as Window & { scrolled: number };
}

function resize(viewport: VisualViewport & { height: number }, height: number) {
  viewport.height = height;
  viewport.dispatchEvent(new Event("resize"));
}

// Safari brings a field up by panning its visual viewport, which scrolls no element; Chrome scrolls the page itself.
function safariPans(view: Window & { scrolled: number }, to: number) {
  view.scrolled = to;
  view.visualViewport?.dispatchEvent(new Event("scroll"));
}
function pageScrolls(view: Window & { scrolled: number }, to: number) {
  view.scrolled = to;
  view.dispatchEvent(new Event("scroll"));
}

afterEach(() => {
  document.body.replaceChildren();
});

describe("the phone page's height", () => {
  it("follows the visual viewport while the keyboard comes and goes, and lets go when stopped", () => {
    const viewport = fakeViewport(844);
    const root = document.createElement("html");
    const stop = followVisualViewport(fakeView(viewport), root);
    expect(root.style.getPropertyValue(VIEWPORT_HEIGHT)).toBe("844px");
    resize(viewport, 508);
    expect(root.style.getPropertyValue(VIEWPORT_HEIGHT)).toBe("508px");
    stop();
    expect(root.style.getPropertyValue(VIEWPORT_HEIGHT)).toBe("");
    resize(viewport, 300);
    expect(root.style.getPropertyValue(VIEWPORT_HEIGHT)).toBe("");
  });

  it("keeps the page at its top when the browser moved it before the keyboard's resize", () => {
    const viewport = fakeViewport(844);
    const view = fakeView(viewport);
    const reply = document.body.appendChild(document.createElement("textarea"));
    followVisualViewport(view, document.createElement("html"));
    reply.focus();
    view.scrolled = 337;
    resize(viewport, 508);
    expect(view.scrolled).toBe(0);
  });

  it("keeps the page at its top when Safari pans after the keyboard's resize, every time the keyboard comes up", () => {
    const viewport = fakeViewport(714);
    const view = fakeView(viewport);
    const reply = document.body.appendChild(document.createElement("textarea"));
    followVisualViewport(view, document.createElement("html"));
    for (let round = 0; round < 3; round += 1) {
      reply.focus();
      resize(viewport, 377);
      safariPans(view, 337);
      expect(view.scrolled).toBe(0);
      reply.blur();
      resize(viewport, 714);
      expect(view.scrolled).toBe(0);
    }
  });

  it("keeps the page at its top when Chrome scrolls it to bring the field up", () => {
    const viewport = fakeViewport(806);
    const view = fakeView(viewport);
    const reply = document.body.appendChild(document.createElement("input"));
    followVisualViewport(view, document.createElement("html"));
    reply.focus();
    resize(viewport, 450);
    pageScrolls(view, 290);
    expect(view.scrolled).toBe(0);
  });

  it("leaves a list where the reader scrolled it while the toolbar folds away and comes back", () => {
    const viewport = fakeViewport(745);
    const view = fakeView(viewport);
    // A pressed row or tab keeps the focus on a phone; it raises no keyboard.
    const row = document.body.appendChild(document.createElement("button"));
    row.focus();
    followVisualViewport(view, document.createElement("html"));
    pageScrolls(view, 180);
    resize(viewport, 812);
    safariPans(view, 186);
    resize(viewport, 745);
    expect(view.scrolled).toBe(186);
  });

  it("stops holding the page once stopped", () => {
    const viewport = fakeViewport(714);
    const view = fakeView(viewport);
    const reply = document.body.appendChild(document.createElement("textarea"));
    followVisualViewport(view, document.createElement("html"))();
    reply.focus();
    safariPans(view, 337);
    pageScrolls(view, 337);
    expect(view.scrolled).toBe(337);
  });

  it("leaves the page alone where the browser has no visual viewport", () => {
    const root = document.createElement("html");
    followVisualViewport(fakeView(null), root)();
    expect(root.style.getPropertyValue(VIEWPORT_HEIGHT)).toBe("");
  });
});
