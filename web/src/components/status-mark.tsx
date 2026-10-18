import type { HTMLAttributes } from "react";
import { MoonIcon } from "lucide-react";
import { cn } from "../lib/utils";
import type { RowMark } from "../snapshot";
import { Hint } from "./ui/tooltip";

/**
 * An agent's status mark: whose move is next, one size on every surface that
 * lists agents (docs/status-model.md, One meaning across surfaces). The core
 * names the mark; this draws it. The shapes differ without their
 * colours: the operator's turn is a `!` in a warning badge, in progress is a
 * static three-quarter ring (dashed while it waits on proven work, D-25), to read is a small dot (green only when the label
 * read the turn as finished), and quiet, an agent at rest, a faint hollow ring
 * smaller and thinner than the in-progress one (sidebar-quiet-mark-indent D-02).
 * An agent Hide put to sleep keeps a grey moon, the one shape outside the four
 * (status-marks D-21). A row of unknown state, and a slot with no live agent
 * behind it (`null`: an ended session, a disconnected device's row), keep the
 * slot empty so titles line up (D-04, D-05). Each shape carries its own colour;
 * a caller's class dims a read turn. A surface with no second line passes
 * `label`, the mark's words, so its hover and accessible name tell a waiting
 * ring from a working one (B36).
 */
export function StatusMark({ mark, label, className, ...rest }: { mark: RowMark | null; label?: string } & HTMLAttributes<HTMLSpanElement>) {
  const shape = (
    <span
      {...(label ? { role: "img", "aria-label": label } : { "aria-hidden": true })}
      data-mark={mark ?? undefined}
      className={cn("inline-flex size-(--size-agent-mark) shrink-0 items-center justify-center", className)}
      {...rest}
    >
      {mark === "turn" ? (
        <svg viewBox="0 0 12 12" className="size-full">
          <circle cx="6" cy="6" r="6" className="fill-warning" />
          <path d="M5.25 2.6h1.5l-.25 4.6h-1zM5.25 8.2h1.5v1.4h-1.5z" className="fill-status-foreground" />
        </svg>
      ) : mark === "going" || mark === "waiting" ? (
        <svg viewBox="0 0 12 12" className="size-full">
          {/* Three quarters from three o'clock round to twelve; it does not turn (D-02). Waiting cuts it into five dashes (D-25). */}
          <path d="M10.05 6A4.05 4.05 0 1 1 6 1.95" fill="none" strokeWidth="1.9" className="stroke-agent-working" {...(mark === "waiting" ? { pathLength: 19, strokeDasharray: "2.2 2" } : {})} />
        </svg>
      ) : mark === "done" || mark === "unread" ? (
        <span className={cn("size-(--size-status-mark) rounded-full", mark === "done" ? "bg-success" : "bg-foreground")} />
      ) : mark === "quiet" ? (
        <svg viewBox="0 0 12 12" className="size-full">
          <circle cx="6" cy="6" r="3.4" fill="none" strokeWidth="1.3" className="stroke-muted-foreground opacity-(--opacity-read-status)" />
        </svg>
      ) : mark === "asleep" ? (
        <MoonIcon className="size-full text-muted-foreground" />
      ) : null}
    </span>
  );
  return label ? <Hint label={label}>{shape}</Hint> : shape;
}
