import type { CellTone, HeadCell } from "../missions";
import { cn } from "../lib/utils";

// One colour per node state (PRD mission-plan-visibility D-11, B35): a chip's
// cell and the sidebar's mission chip read the `--mission-*` tokens here. The
// mission page's head draws its cells as its cards (`HEAD_FILL`, PRD
// mission-map-git-criteria B26): finished work green, in main or not, so it
// reads apart from what waits or works at a glance.

/** A cell's fill: a state with colour, an empty cell for a node still ahead. */
export const TONE_FILL: Record<CellTone, string> = {
  done: "bg-mission-done",
  review: "bg-mission-review",
  working: "bg-mission-working",
  hold: "bg-mission-hold",
  failed: "bg-mission-failed",
  waiting: "bg-muted",
  cancelled: "bg-mission-cancelled",
};

/** A mission page head cell, as its card looks: an outside node waits as an empty cell. */
export const HEAD_FILL: Record<HeadCell, string> = {
  waiting: "bg-muted",
  external: "bg-muted",
  working: "bg-mission-working",
  review: "bg-mission-review",
  hold: "bg-mission-hold",
  failed: "bg-mission-failed",
  done: "bg-mission-done",
  cancelled: "bg-mission-cancelled",
};

/** A word in a state's colour: a chip or a result. */
export const TONE_TEXT: Record<CellTone, string> = {
  done: "text-mission-done",
  review: "text-mission-review",
  working: "text-mission-working",
  hold: "text-mission-hold",
  failed: "text-mission-failed",
  waiting: "text-muted-foreground",
  cancelled: "text-mission-cancelled",
};

/** Past this many, a bar draws each run of one state as one stretch instead of a cell per item. */
export const CELL_LIMIT = 20;

/**
 * One cell per step or node (B1, B8, B13), in order and in its state's
 * colour; a long list draws each run of one state as one stretch, so the bar
 * keeps its width and its proportions. The words beside it carry the
 * numbers, so the bar itself is hidden from a screen reader.
 */
export function ProgressCells<T extends string = CellTone>({ tones, fill, size, className }: { tones: readonly T[]; fill?: Record<T, string>; size: "chip" | "head"; className?: string }) {
  const fills = fill ?? (TONE_FILL as Record<string, string>);
  const runs: { tone: T; count: number }[] = [];
  if (tones.length > CELL_LIMIT) {
    for (const tone of tones) {
      const last = runs.at(-1);
      if (last?.tone === tone) last.count += 1;
      else runs.push({ tone, count: 1 });
    }
  }
  const cells = tones.length > CELL_LIMIT ? runs : tones.map((tone) => ({ tone, count: 1 }));
  return (
    <span
      aria-hidden="true"
      data-progress-cells={tones.join(" ")}
      className={cn(
        "flex shrink-0 items-stretch",
        size === "chip" ? "h-(--size-mission-step-h) w-(--size-chip-bar) gap-px" : "h-(--size-flow-bar) min-w-0 flex-1 gap-xxs",
        className,
      )}
    >
      {cells.map((cell, index) => (
        <span key={index} className={cn("rounded-full", fills[cell.tone])} style={{ flexGrow: cell.count, flexBasis: 0 }} />
      ))}
    </span>
  );
}
