// A fenced code block as every Markdown renderer draws it (Doop N 1 가): the
// Desk answer and session text (`MarkdownText`), and the phone, a pull
// request's body and a read turn (`mobile/markdown.tsx`).

import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { cn } from "../lib/utils";

/**
 * The code on its own ground with room around it: a long line scrolls inside
 * the block, faded at the edge while more of it is hidden, and never wraps.
 * `action` (the copy button) stands in a gutter of its own at the right, so
 * it never sits on the code: centred on a one-line block, at the top of a
 * longer one, and still while the code scrolls under the fade.
 */
export function CodeBlock({ code, action }: { code: string; action?: ReactNode }) {
  const scroller = useRef<HTMLPreElement>(null);
  const [hidden, setHidden] = useState(false);
  const oneLine = !code.includes("\n");
  useLayoutEffect(() => {
    const box = scroller.current;
    if (!box) return undefined;
    const measure = () => setHidden(box.scrollLeft + box.clientWidth < box.scrollWidth - 1);
    measure();
    box.addEventListener("scroll", measure, { passive: true });
    const sized = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    sized?.observe(box);
    return () => {
      box.removeEventListener("scroll", measure);
      sized?.disconnect();
    };
  }, [code]);
  return (
    // Its width is the column's, never its longest line's: an editor that sizes its content to what it holds would otherwise widen past the column.
    <div className="relative min-w-0 rounded-sm border border-border bg-secondary font-mono text-[0.9em] leading-relaxed text-foreground [contain:inline-size]" data-code-block={oneLine ? "line" : "lines"}>
      <pre ref={scroller} className={cn("m-none overflow-x-auto whitespace-pre py-sm pl-md", action ? "pr-(--size-icon-button-standard)" : "pr-md")}>{code}</pre>
      {hidden ? <span aria-hidden="true" className={cn("pointer-events-none absolute inset-y-0 w-(--spacing-lg) bg-linear-to-l from-secondary to-transparent", action ? "right-(--size-icon-button-standard)" : "right-0")} data-code-block-fade="true" /> : null}
      {action ? (
        // The gutter keeps the block's ground, so code scrolled under it stays hidden behind it.
        <span className={cn("absolute inset-y-0 right-0 flex w-(--size-icon-button-standard) justify-center rounded-r-sm bg-secondary font-sans", oneLine ? "items-center" : "items-start pt-xs")} data-code-block-action="true">
          {action}
        </span>
      ) : null}
    </div>
  );
}
