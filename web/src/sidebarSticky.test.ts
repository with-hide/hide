// @vitest-environment jsdom
import { expect, it, vi } from "vitest";
import { observeStuckHeaders } from "./sidebarSticky";

it("coalesces scroll and resize into one geometry pass and cancels on disposal", () => {
  const pending = new Map<number, FrameRequestCallback>();
  let sequence = 0;
  let resize = () => {};
  const disconnect = vi.fn();
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    pending.set(++sequence, callback);
    return sequence;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => pending.delete(id));
  vi.stubGlobal("ResizeObserver", class {
    constructor(callback: () => void) { resize = callback; }
    observe() {}
    disconnect = disconnect;
  });
  const list = document.createElement("ul");
  list.innerHTML = '<li><div data-section-sticky></div></li><li><div data-section-sticky></div></li>';
  const reads: string[] = [];
  const layout = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    // No marker may be written until all geometry has been read.
    expect(list.querySelector('[data-stuck]')).toBeNull();
    reads.push(this === list ? "list" : "section");
    return { top: this === list ? 0 : -10, bottom: 100 } as DOMRect;
  });
  let dispose: (() => void) | undefined;
  try {
    dispose = observeStuckHeaders(list);
    for (let n = 0; n < 10; n++) {
      list.dispatchEvent(new Event("scroll"));
      resize();
    }
    expect(reads).toEqual([]);
    expect(pending.size).toBe(1);
    const [id, callback] = [...pending.entries()][0]!;
    pending.delete(id);
    callback(0);
    expect(reads).toEqual(["list", "section", "section"]);
    expect([...list.querySelectorAll<HTMLElement>("[data-section-sticky]")].map((header) => header.dataset.stuck)).toEqual(["true", "true"]);
    list.dispatchEvent(new Event("scroll"));
    expect(pending.size).toBe(1);
    dispose();
    dispose = undefined;
    expect(pending.size).toBe(0);
    list.dispatchEvent(new Event("scroll"));
    expect(pending.size).toBe(0);
    expect(disconnect).toHaveBeenCalled();
  } finally {
    dispose?.();
    layout.mockRestore();
    vi.unstubAllGlobals();
  }
});
