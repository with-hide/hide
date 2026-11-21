import type { HTMLAttributes } from "react";
import { badgeParts, type BadgeCounts, type BadgePart } from "../agentRow";
import { cn } from "../lib/utils";
import { StatusMark } from "./status-mark";
import { Badge } from "./ui/badge";

/** A badge's marks, each in the colour its rows draw it, with its count. */
export function BadgeMarks({ parts }: { parts: BadgePart[] }) {
  return parts.map((part) => (
    <span key={part.state} className="mark-with-count inline-flex items-center gap-xs" data-badge-part={part.state}>
      <StatusMark mark={part.mark} />
      {part.count}
    </span>
  ));
}

/**
 * A project's or a closed checkout's agents, one mark and count per mark
 * their rows draw, worst first (docs/status-model.md, The status badge).
 * Nothing is drawn when no agent there draws a counted mark. The row it sits
 * on says it in words in its own accessible name.
 */
export function StatusBadge({ counts, className, ...rest }: { counts: BadgeCounts | undefined } & HTMLAttributes<HTMLSpanElement>) {
  const parts = badgeParts(counts);
  if (parts.length === 0) return null;
  return (
    <Badge aria-hidden="true" variant="secondary" className={cn("shrink-0 gap-xs font-mono", className)} {...rest}>
      <BadgeMarks parts={parts} />
    </Badge>
  );
}
