import { ChevronDownIcon, ListTreeIcon } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "../lib/utils";

/**
 * The children chip's face, the one chip for everything working under an
 * agent (docs/UI_BEHAVIOR.md, Subagents): the tree, the first direct child's
 * status mark (`mark`, null when none can be claimed), how many direct
 * children there are (left out when one marked child already says it), then,
 * after a dot, the subagent mark and how many subagents the agent runs inside
 * its session, and the chevron. The desktop pane header's button and the
 * phone list's toggle draw it at their own sizes (`icon`); the owner names
 * the control, so every part here is hidden from a screen reader.
 */
export function TreeChipFace({ mark, count, subagents, icon }: { mark: ReactNode; count: number; subagents: number; icon: string }) {
  return (
    <>
      <ListTreeIcon aria-hidden="true" className={cn(icon, "shrink-0 text-muted-foreground")} />
      {mark}
      {count > 0 && (count > 1 || !mark) ? <span aria-hidden="true" className="font-mono" data-tree-chip-count={count}>{count}</span> : null}
      {count > 0 && subagents > 0 ? <span aria-hidden="true" className="text-muted-foreground">·</span> : null}
      {subagents > 0 ? (
        <>
          <span aria-hidden="true" className="size-(--size-subagent-dot) shrink-0 rounded-full bg-agent-working opacity-(--opacity-secondary)" />
          <span aria-hidden="true" className="font-mono" data-tree-chip-subagents={subagents}>{subagents}</span>
        </>
      ) : null}
      <ChevronDownIcon aria-hidden="true" className={cn(icon, "shrink-0 text-muted-foreground")} />
    </>
  );
}
