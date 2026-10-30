import { GitPullRequestIcon } from "lucide-react";
import type { HTMLAttributes } from "react";
import type { PrState } from "../snapshot";
import { cn } from "../lib/utils";
import { PrMark } from "./pr-mark";

// A mission's pull request wherever its page names one (PRD mission-tool
// B18): the one PR mark, its state's icon and colour, never a bare `#n`. A
// pull request whose state Hide never read (a mission recorded before it
// kept one, an outside pull request) keeps the pull request's icon in the
// muted colour, so no state is made up for it.

export function MissionPrMark({ number, state, stale = false, className, ...rest }: { number: number; state: PrState | null | undefined; stale?: boolean } & HTMLAttributes<HTMLSpanElement>) {
  if (state) return <PrMark state={state} number={number} stale={stale} className={className} {...rest} />;
  return (
    <span data-pr-mark="unread" className={cn("inline-flex shrink-0 items-center gap-xxs font-mono text-caption", className)} {...rest}>
      <GitPullRequestIcon aria-hidden="true" className="size-(--size-pr-icon) shrink-0 text-muted-foreground" />
      <span className="text-subtle-foreground">#{number}</span>
    </span>
  );
}
