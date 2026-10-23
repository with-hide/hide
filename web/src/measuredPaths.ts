import { useLayoutEffect, useState, type RefObject } from "react";

/** One drawn line between two measured nodes. */
export type MeasuredPath = { id: string; d: string };

/**
 * Lines drawn between nodes a board laid out with CSS, measured after layout
 * against `root` and again whenever the root or a node changes size (the
 * Dependencies mode's arrows, the Overview's delegation lines). `selector`
 * finds the nodes to watch; `compute` turns the root's rect and a node lookup
 * into the paths. Measuring is local layout: it publishes nothing.
 */
export function useMeasuredPaths(
  root: RefObject<HTMLElement | null>,
  selector: string,
  compute: (origin: DOMRect, at: (attribute: string, id: string) => DOMRect | null) => MeasuredPath[],
  deps: readonly unknown[],
): MeasuredPath[] {
  const [paths, setPaths] = useState<MeasuredPath[]>([]);
  useLayoutEffect(() => {
    const box = root.current;
    if (!box) return;
    const measure = () => {
      const origin = box.getBoundingClientRect();
      // A board drawn shrunk (the mission map fitted to its slot) measures on screen at that scale; the
      // paths are drawn inside the shrunk board, so each rect is read back at the board's own size.
      const scale = box.offsetWidth > 0 ? origin.width / box.offsetWidth : 1;
      const local = (rect: DOMRect) => (scale === 1 || !Number.isFinite(scale) ? rect : new DOMRect(origin.left + (rect.left - origin.left) / scale, origin.top + (rect.top - origin.top) / scale, rect.width / scale, rect.height / scale));
      const at = (attribute: string, id: string) => {
        const rect = box.querySelector(`[${attribute}="${CSS.escape(id)}"]`)?.getBoundingClientRect();
        return rect ? local(rect) : null;
      };
      const next = compute(origin, at);
      setPaths((current) => (JSON.stringify(current) === JSON.stringify(next) ? current : next));
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(box);
    for (const node of box.querySelectorAll(selector)) observer.observe(node);
    return () => observer.disconnect();
    // `compute` is recreated each render; the caller's deps say when it changes.
  }, deps);
  return paths;
}
