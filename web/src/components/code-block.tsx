// Code as every Markdown renderer draws it (Doop N 1 나): the Desk answer and
// session text (`MarkdownText`), the phone, a pull request's body and a read
// turn (`mobile/markdown.tsx`), and the file editor's Live view, which draws
// `CodeBlockHead` on a fence's first line and boxes the code lines in
// `editor/theme.ts` (`.cm-md-code*`) with the same tokens.

import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";

/** Code inside a line: mono on its own ground inside a hairline, with room and a radius. The editor's Live view and the phone both draw it with these classes. */
export const INLINE_CODE = "rounded-xs border border-border bg-secondary px-xs py-xxs font-mono text-[0.9em]";

// A long line's scrollbar stays thin and in view, under the code, as the frame draws it.
const THIN_SCROLLBAR = "[&::-webkit-scrollbar]:h-(--spacing-xxs) [&::-webkit-scrollbar-thumb]:rounded-full [&::-webkit-scrollbar-thumb]:bg-muted-foreground/50 [&::-webkit-scrollbar-track]:bg-transparent";
// The code fades out at the scroller's own right edge while more of it is hidden: a mask on the code, never a fill laid over it.
const FADE = "[mask-image:linear-gradient(to_left,transparent,currentColor_var(--spacing-lg))]";

/**
 * A code block's head row: the fence's language, or `코드` when it names
 * none, and `action` (복사) at its right, above a hairline. The code is below
 * it, so the button never stands on the code.
 */
export function CodeBlockHead({ language, action }: { language: string | null; action?: ReactNode }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex h-(--size-control-compact) min-w-0 items-center justify-between gap-xs border-b border-border px-md font-sans text-caption text-muted-foreground" data-code-block-head="true">
      <span className="min-w-0 truncate" data-code-block-language={language ?? ""}>{language ?? t("common.code")}</span>
      {action ? <span className="flex shrink-0 items-center" data-code-block-action="true">{action}</span> : null}
    </div>
  );
}

/**
 * A fenced or indented block: its head row, then the code on the block's
 * ground with room around it. A long line scrolls inside the code's own box,
 * with a thin scrollbar and the code fading at that box's right edge while
 * more of it is hidden; it never wraps.
 */
export function CodeBlock({ code, language = null, action }: { code: string; language?: string | null; action?: ReactNode }) {
  const scroller = useRef<HTMLPreElement>(null);
  const [hidden, setHidden] = useState(false);
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
    <div className="min-w-0 overflow-hidden rounded-sm border border-border bg-secondary text-foreground [contain:inline-size]" data-code-block="true">
      <CodeBlockHead language={language} action={action} />
      <pre ref={scroller} className={cn("m-none overflow-x-auto whitespace-pre px-md py-sm font-mono text-[0.9em] leading-relaxed", THIN_SCROLLBAR, hidden && FADE)} data-code-block-fade={hidden ? "true" : undefined}>
        {code}
      </pre>
    </div>
  );
}
