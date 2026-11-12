import type { NodeLook } from "../missions";
import { cn } from "../lib/utils";
import { Hint } from "./ui/tooltip";

// A mission card's state as a shape (PRD mission-map-git-criteria D-02,
// B26; PRD mission-node-simple B1): waiting, in progress and done each have
// their own form, readable without its colour; what someone has to look at
// is the card's `!` tag, not a shape. Finished work is a green disc with a
// check. An outside node still watching is a dotted ring, its `↗` drawn
// beside its number on the card in every state. The hover and the
// accessible name say the same state word. A node whose worker Hide put to
// sleep keeps the in-progress ring; its label names the sleep (B1). The ring
// turns only while the node is worked: a node in progress whose worker rests
// or is gone (`still`, hide-private issue 680) keeps the ring and stops it.

/** `hint` false draws the mark alone, where its word stands beside it already (the card's hover, D-58). */
export function MissionNodeMark({ look, label, still = false, hint = true, className }: { look: NodeLook; label: string; still?: boolean; hint?: boolean; className?: string }) {
  const mark = (
    <span role="img" aria-label={label} data-node-mark={look} data-node-mark-still={still || undefined} className={cn("inline-flex size-(--size-agent-mark) shrink-0 items-center justify-center", className)}>
      <svg viewBox="0 0 12 12" className={cn("size-full", look === "working" && !still && "motion-safe:animate-spin")} aria-hidden="true">
        <Shape look={look} />
      </svg>
    </span>
  );
  return hint ? <Hint label={label}>{mark}</Hint> : mark;
}

function Shape({ look }: { look: NodeLook }) {
  switch (look) {
    case "waiting":
      return <circle cx="6" cy="6" r="4.6" fill="none" strokeWidth="1.3" strokeDasharray="1.8 1.6" className="stroke-muted-foreground" />;
    case "working":
      // An open ring that turns while a worker is at the node; a reduced-motion setting holds it still.
      return <path d="M10.6 6A4.6 4.6 0 1 1 6 1.4" fill="none" strokeWidth="1.9" strokeLinecap="round" className="stroke-mission-working" />;
    case "done":
      return (
        <>
          <circle cx="6" cy="6" r="6" className="fill-mission-done" />
          <path d="M3.4 6.2l1.8 1.8 3.4-3.6" fill="none" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" className="stroke-status-foreground" />
        </>
      );
    case "external":
      return <circle cx="6" cy="6" r="4.6" fill="none" strokeWidth="1.3" strokeDasharray="0.1 2.1" strokeLinecap="round" className="stroke-muted-foreground" />;
    case "cancelled":
      return <circle cx="6" cy="6" r="4.6" fill="none" strokeWidth="1.3" strokeDasharray="1.8 1.6" className="stroke-mission-cancelled" />;
    case "dropped":
      return <circle cx="6" cy="6" r="3.4" fill="none" strokeWidth="1.3" className="stroke-muted-foreground opacity-(--opacity-dimmed)" />;
  }
}
