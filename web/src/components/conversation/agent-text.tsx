// A piece of the agent's words in a conversation (mockup v3 sections 0, 2): always shown, led by ●, as a terminal
// leads each piece of an agent's answer. The words themselves are the screen's to draw (Markdown on the desktop, the
// phone's own renderer), so this part holds only the mark and its place.

import type { ReactNode } from "react";
import { cn } from "../../lib/utils";

/**
 * The agent's words, led by a ● on their first line. The words keep the chat's line height, so the mark sits on the
 * first line whatever follows; `className` sets another text size with its line height (the phone's).
 */
export function AgentText({ children, className, ...data }: { children: ReactNode; className?: string } & Record<`data-${string}`, string | undefined>) {
  return (
    <div className={cn("flex min-w-0 items-start gap-sm text-subhead leading-[calc(var(--text-subhead)+var(--spacing-md))]", className)} data-conversation-text="true" {...data}>
      <span aria-hidden="true" className="flex h-lh shrink-0 items-center pt-xs">
        <span className="size-(--size-status-mark) rounded-full bg-foreground" />
      </span>
      <div className="min-w-0 flex-1">{children}</div>
    </div>
  );
}
