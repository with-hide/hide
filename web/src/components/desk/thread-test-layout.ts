// A layout for tests of a session conversation, which a test window does not lay out: the conversation draws only
// the rows its viewport and their measured heights reach (issue 3101), so a test gives it a viewport and each row a
// height, as a window would. Imported by tests only.

import { act } from "react";
import { vi } from "vitest";

/**
 * Gives the conversation's scroll box a viewport of `viewport` pixels and each row `row` pixels (or the height `row`
 * answers for it), and lets a scroll the
 * conversation asks for move it within its height and say so when it moved, as a browser's does. The default viewport
 * holds every row a test writes; a test of which rows are drawn gives a small one. Undone by `vi.restoreAllMocks()`.
 */
export function layOutConversation({ viewport = 100_000, row = 40 }: { viewport?: number; row?: number | ((row: HTMLElement) => number) } = {}): void {
  const rowHeight = typeof row === "number" ? () => row : row;
  const height = (element: HTMLElement) => (element.hasAttribute("data-desk-thread-conversation") ? viewport : element.hasAttribute("data-thread-row") ? rowHeight(element) : null);
  const offsetHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetHeight")!.get!;
  const clientHeight = Object.getOwnPropertyDescriptor(Element.prototype, "clientHeight")!.get!;
  const scrollHeight = Object.getOwnPropertyDescriptor(Element.prototype, "scrollHeight")!.get!;
  vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockImplementation(function (this: HTMLElement) { return height(this) ?? offsetHeight.call(this); });
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockImplementation(function (this: HTMLElement) { return height(this) ?? clientHeight.call(this); });
  vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockImplementation(function (this: HTMLElement) {
    return this.hasAttribute("data-desk-thread-conversation") ? Number.parseFloat(this.querySelector("ol")?.style.height || "0") : scrollHeight.call(this);
  });
  // A test window has no `scrollTo` on an element at all.
  if (!("scrollTo" in Element.prototype)) Object.defineProperty(Element.prototype, "scrollTo", { configurable: true, writable: true, value: () => undefined });
  vi.spyOn(Element.prototype as HTMLElement, "scrollTo").mockImplementation(function (this: HTMLElement, options?: ScrollToOptions | number) {
    if (typeof options !== "object" || options.top === undefined) return;
    const top = Math.max(0, Math.min(options.top, this.scrollHeight - this.clientHeight));
    if (top === this.scrollTop) return;
    this.scrollTop = top;
    this.dispatchEvent(new Event("scroll"));
  });
}

type Watch = { callback: ResizeObserverCallback; targets: Set<Element> };

/**
 * A `ResizeObserver` that reports when `laidOut` says the page was laid out, as a browser's does in the frame after a
 * change: every watched element reports, and each reads its size from the layout above.
 */
export function watchResizes(): { laidOut: () => Promise<void> } {
  const watches = new Set<Watch>();
  vi.stubGlobal("ResizeObserver", class {
    readonly watch: Watch;
    constructor(callback: ResizeObserverCallback) {
      this.watch = { callback, targets: new Set<Element>() };
      watches.add(this.watch);
    }
    observe(target: Element) { this.watch.targets.add(target); }
    unobserve(target: Element) { this.watch.targets.delete(target); }
    disconnect() { watches.delete(this.watch); }
  });
  return {
    laidOut: () => act(async () => {
      for (const watch of [...watches]) {
        if (watch.targets.size) watch.callback([...watch.targets].map((target) => ({ target }) as ResizeObserverEntry), {} as ResizeObserver);
      }
    }),
  };
}

/** The operator scrolls the conversation inside `container` to `top`, or to its foot, as a wheel would. */
export function scrollConversation(container: ParentNode, top: number | "end"): Promise<void> {
  return act(async () => {
    const box = container.querySelector<HTMLElement>("[data-desk-thread-conversation]")!;
    box.scrollTop = top === "end" ? box.scrollHeight - box.clientHeight : top;
    box.dispatchEvent(new Event("scroll"));
  });
}

/** The rows the conversation inside `container` draws now. */
export function drawnRows(container: ParentNode): HTMLElement[] {
  return Array.from(container.querySelectorAll<HTMLElement>("[data-thread-row]"));
}
