// What every part shares: the tooltip host the product's components expect and the height the docs page needs.
// The part's box reports its own height to the page that frames it, on load and whenever it changes (a font
// arriving, a state swapping, the frame being resized), so the iframe fits the picture with no fixed height typed
// into a page. A part whose product component fills a viewport instead gets a fixed `height` here.
import { useLayoutEffect, useRef, type ReactNode } from "react";
import { TooltipProvider } from "../components/ui/tooltip";
import { useReducedMotion } from "../lib/useReducedMotion";
import type { PartId } from "./parts";

export const HEIGHT_MESSAGE = "hide-docs-embed-height";

export function PartFrame({ part, height, children }: { part: PartId; /** A part that fills its box asks for one; the rest take the height their content has. */ height?: number; children: ReactNode }) {
  const box = useRef<HTMLDivElement>(null);
  const reduced = useReducedMotion();
  useLayoutEffect(() => {
    const element = box.current;
    if (!element || window.parent === window) return;
    const report = () => window.parent.postMessage({ type: HEIGHT_MESSAGE, part, height: Math.ceil(element.getBoundingClientRect().height) }, "*");
    report();
    const observer = new ResizeObserver(report);
    observer.observe(element);
    return () => observer.disconnect();
  }, [part]);
  return (
    <TooltipProvider>
      <div ref={box} style={height ? { height } : undefined} className="flow-root min-w-0 bg-background p-lg text-body text-foreground" data-docs-part={part} data-docs-motion={reduced ? "still" : "live"}>
        {children}
      </div>
    </TooltipProvider>
  );
}
