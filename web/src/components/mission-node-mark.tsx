import type { NodeLook } from "../missions";
import { cn } from "../lib/utils";
import { Hint } from "./ui/tooltip";

// A mission card's state as a shape (PRD mission-map-git-criteria D-02,
// B26): each look has its own form, readable without its colour, and colour
// is kept for what wants a look (working, review, a decision, a failure).
// Finished work is grey; work in main is a faint check with no fill. The
// hover and the accessible name say the same state word.

export function MissionNodeMark({ look, label, className }: { look: NodeLook; label: string; className?: string }) {
  return (
    <Hint label={label}>
      <span role="img" aria-label={label} data-node-mark={look} className={cn("inline-flex size-(--size-agent-mark) shrink-0 items-center justify-center", className)}>
        <svg viewBox="0 0 12 12" className={cn("size-full", look === "working" && "motion-safe:animate-spin")} aria-hidden="true">
          <Shape look={look} />
        </svg>
      </span>
    </Hint>
  );
}

function Shape({ look }: { look: NodeLook }) {
  switch (look) {
    case "waiting":
      return <circle cx="6" cy="6" r="4.6" fill="none" strokeWidth="1.3" strokeDasharray="1.8 1.6" className="stroke-muted-foreground" />;
    case "working":
      // An open ring that turns while a worker is at the node; a reduced-motion setting holds it still.
      return <path d="M10.6 6A4.6 4.6 0 1 1 6 1.4" fill="none" strokeWidth="1.9" strokeLinecap="round" className="stroke-mission-working" />;
    case "review":
      return (
        <>
          <circle cx="6" cy="6" r="6" className="fill-mission-review" />
          <circle cx="6" cy="6" r="2.4" fill="none" strokeWidth="1.3" className="stroke-status-foreground" />
        </>
      );
    case "hold":
      return (
        <>
          <circle cx="6" cy="6" r="6" className="fill-mission-hold" />
          <path d="M5.25 2.6h1.5l-.25 4.6h-1zM5.25 8.2h1.5v1.4h-1.5z" className="fill-status-foreground" />
        </>
      );
    case "failed":
      return (
        <>
          <circle cx="6" cy="6" r="6" className="fill-mission-failed" />
          <path d="M4 4l4 4M8 4l-4 4" fill="none" strokeWidth="1.5" strokeLinecap="round" className="stroke-status-foreground" />
        </>
      );
    case "done":
      return (
        <>
          <circle cx="6" cy="6" r="6" className="fill-muted" />
          <path d="M3.4 6.2l1.8 1.8 3.4-3.6" fill="none" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" className="stroke-muted-foreground" />
        </>
      );
    case "landed":
      return <path d="M3.2 6.2l1.9 1.9 3.7-3.9" fill="none" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" className="stroke-muted-foreground opacity-(--opacity-dimmed)" />;
    case "external":
      return (
        <>
          <circle cx="6" cy="6" r="4.9" fill="none" strokeWidth="1.1" className="stroke-muted-foreground" />
          <path d="M4.4 7.6l3.2-3.2M5 4.4h2.6V7" fill="none" strokeWidth="1.1" strokeLinecap="round" strokeLinejoin="round" className="stroke-muted-foreground" />
        </>
      );
    case "cancelled":
      return <circle cx="6" cy="6" r="4.6" fill="none" strokeWidth="1.3" strokeDasharray="1.8 1.6" className="stroke-mission-cancelled" />;
    case "dropped":
      return <circle cx="6" cy="6" r="3.4" fill="none" strokeWidth="1.3" className="stroke-muted-foreground opacity-(--opacity-dimmed)" />;
  }
}
