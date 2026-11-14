import type { NodeLook, NodePace } from "../missions";
import { cn } from "../lib/utils";
import { Hint } from "./ui/tooltip";

// A mission card's state as a shape (PRD mission-map-git-criteria D-02,
// B26; PRD mission-node-simple B1): waiting, in progress and done each have
// their own form, readable without its colour; what someone has to look at
// is the card's `!` tag, not a shape. Finished work is a green disc with a
// check. An outside node still watching is a dotted ring, its `↗` drawn
// beside its number on the card in every state. The hover and the
// accessible name say the same state word. The in-progress ring follows the
// node's pace (`NodePace`, operator decision 2026-11-13): it turns while the
// node is worked, and stands cut into dashes in the working colour while its
// worker rests on someone or sleeps (its label names the sleep, B1). A node
// no one moves because its worker stopped or is gone is a filled grey disc
// with two pause bars, the stopped mark: a full shape, unlike the empty
// dashed ring of a node still waiting to start, so the two never read alike.
// A reduced-motion setting holds the turning ring still.

/** `hint` false draws the mark alone, where its word stands beside it already (the card's hover, D-58). */
/** `failed` draws a done node that ended on a failed verdict red with a cross (PRD node-presets D-08). */
export function MissionNodeMark({ look, label, pace = null, failed = false, hint = true, className }: { look: NodeLook; label: string; pace?: NodePace | null; failed?: boolean; hint?: boolean; className?: string }) {
  const moves = look === "working" ? (pace ?? "working") : null;
  const mark = (
    <span role="img" aria-label={label} data-node-mark={look} data-node-mark-pace={moves ?? undefined} data-node-mark-failed={(failed && look === "done") || undefined} className={cn("inline-flex size-(--size-agent-mark) shrink-0 items-center justify-center", className)}>
      <svg viewBox="0 0 12 12" className={cn("size-full", moves === "working" && "motion-safe:animate-working-turn")} aria-hidden="true">
        {failed && look === "done" ? <Failed /> : moves === "stopped" ? <Paused /> : moves === "waiting" ? <Dashed /> : <Shape look={look} />}
      </svg>
    </span>
  );
  return hint ? <Hint label={label}>{mark}</Hint> : mark;
}

function Failed() {
  return (
    <>
      <circle cx="6" cy="6" r="6" className="fill-mission-failed" />
      <path d="M4 4l4 4M8 4l-4 4" fill="none" strokeWidth="1.5" strokeLinecap="round" className="stroke-status-foreground" />
    </>
  );
}

/** The in-progress ring cut into five dashes in the working colour, as an agent's waiting mark is (`StatusMark`). */
function Dashed() {
  return <path d="M10.6 6A4.6 4.6 0 1 1 6 1.4" fill="none" strokeWidth="1.9" pathLength={19} strokeDasharray="2.2 2" className="stroke-mission-working" />;
}

/** A grey disc with two pause bars: a node no one moves, its worker stopped or gone (operator decision 2026-11-13, card A). */
function Paused() {
  return (
    <>
      <circle cx="6" cy="6" r="6" className="fill-mission-stopped" />
      <rect x="3.9" y="3.5" width="1.5" height="5" rx="0.4" className="fill-status-foreground" />
      <rect x="6.6" y="3.5" width="1.5" height="5" rx="0.4" className="fill-status-foreground" />
    </>
  );
}

function Shape({ look }: { look: NodeLook }) {
  switch (look) {
    case "waiting":
      return <circle cx="6" cy="6" r="4.6" fill="none" strokeWidth="1.3" strokeDasharray="1.8 1.6" className="stroke-muted-foreground" />;
    case "working":
      // An open ring that turns while a worker is at the node.
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
