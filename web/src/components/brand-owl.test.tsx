// @vitest-environment jsdom
// PRD B10/B11/B15: browser visibility and preference events determine whether
// a decorative image moves; leaving its scene releases those subscriptions.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { BrandOwl } from "./brand-owl";
import { useShellStore } from "../store";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
afterEach(() => { vi.unstubAllGlobals(); vi.restoreAllMocks(); document.body.replaceChildren(); });

it("keeps a static owl for reduced motion, hidden or offscreen scenes and releases the scene", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const media = Object.assign(new EventTarget(), { matches: true });
  const removed = vi.spyOn(media, "removeEventListener");
  vi.stubGlobal("matchMedia", () => media);
  let intersect: (entries: Partial<IntersectionObserverEntry>[]) => void = () => {};
  const disconnect = vi.fn();
  vi.stubGlobal("IntersectionObserver", class {
    constructor(callback: typeof intersect) { intersect = callback; }
    observe() {} disconnect = disconnect;
  });
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  const hidden = vi.spyOn(document, "hidden", "get").mockReturnValue(false);
  try {
    await act(async () => root.render(<BrandOwl pose="ai" />));
    await act(async () => intersect([{ isIntersecting: true }]));
    const image = host.querySelector("img")!;
    const still = image.src;
    expect(image.dataset.owlMotion).toBe("paused");
    expect(image.alt).toBe("");
    expect(image.getAttribute("aria-hidden")).toBe("true");
    await act(async () => { media.matches = false; media.dispatchEvent(new Event("change")); });
    expect(image.src).not.toBe(still);
    await act(async () => { hidden.mockReturnValue(true); document.dispatchEvent(new Event("visibilitychange")); });
    expect(image.src).toBe(still);
    await act(async () => { hidden.mockReturnValue(false); document.dispatchEvent(new Event("visibilitychange")); });
    expect(image.dataset.owlMotion).toBe("playing");
    await act(async () => intersect([{ isIntersecting: false }]));
    expect(image.src).toBe(still);
    await act(async () => intersect([{ isIntersecting: true }]));
    await act(async () => image.dispatchEvent(new Event("error")));
    expect(image.src).toBe(still);
    expect(useShellStore.getState().diagnostics.at(-1)).toContain("brand_owl.unavailable");
  } finally { await act(async () => root.unmount()); }
  expect(disconnect).toHaveBeenCalledOnce();
  expect(removed).toHaveBeenCalledWith("change", expect.any(Function));
});
