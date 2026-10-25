import type { LegendTone, NodeTone } from "../missions";
import { cn } from "../lib/utils";

// One colour per node state (PRD mission-plan-visibility D-11, B14, B35):
// the card's whole border, the legend's swatch, a chip's or the head's cell
// and the sidebar's mission chip all read the `--mission-*` tokens here.

/** A cell's fill: a state with colour, an empty cell for a node still ahead. */
export const TONE_FILL: Record<LegendTone, string> = {
  done: "bg-mission-done",
  review: "bg-mission-review",
  working: "bg-mission-working",
  hold: "bg-mission-hold",
  failed: "bg-mission-failed",
  waiting: "bg-muted",
  cancelled: "bg-mission-cancelled",
};

/** A card's whole border (B14): a waiting or cancelled card dashed, a card with a worker at it or a decision lit by a faint glow. */
export const TONE_BORDER: Record<NodeTone, string> = {
  done: "border-mission-done/60",
  review: "border-mission-review ring-2 ring-mission-review/20",
  working: "border-mission-working ring-2 ring-mission-working/20",
  hold: "border-mission-hold ring-2 ring-mission-hold/20",
  failed: "border-mission-failed",
  waiting: "border-dashed border-mission-waiting bg-transparent",
  cancelled: "border-dashed border-mission-cancelled bg-transparent",
  dropped: "border-dashed border-border bg-transparent opacity-(--opacity-dimmed)",
};

/** A word in a state's colour: a chip, a result or a legend count. */
export const TONE_TEXT: Record<LegendTone, string> = {
  done: "text-mission-done",
  review: "text-mission-review",
  working: "text-mission-working",
  hold: "text-mission-hold",
  failed: "text-mission-failed",
  waiting: "text-muted-foreground",
  cancelled: "text-mission-cancelled",
};

/** A legend swatch: the card's border in small. */
const SWATCH: Record<LegendTone, string> = {
  done: "border-mission-done bg-mission-done/20",
  review: "border-mission-review",
  working: "border-mission-working",
  hold: "border-mission-hold",
  failed: "border-mission-failed",
  waiting: "border-dashed border-mission-waiting",
  cancelled: "border-dashed border-mission-cancelled",
};

export function ToneSwatch({ tone }: { tone: LegendTone }) {
  return <span aria-hidden="true" className={cn("inline-block size-(--size-status-mark) shrink-0 rounded-xs border", SWATCH[tone])} />;
}

/** Past this many, a bar draws each run of one state as one stretch instead of a cell per item. */
export const CELL_LIMIT = 20;

/**
 * One cell per step or node (B1, B8, B13), in order and in its state's
 * colour; a long list draws each run of one state as one stretch, so the bar
 * keeps its width and its proportions. The words beside it carry the
 * numbers, so the bar itself is hidden from a screen reader.
 */
export function ProgressCells({ tones, size, className }: { tones: readonly LegendTone[]; size: "chip" | "head"; className?: string }) {
  const runs: { tone: LegendTone; count: number }[] = [];
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
        <span key={index} className={cn("rounded-full", TONE_FILL[cell.tone])} style={{ flexGrow: cell.count, flexBasis: 0 }} />
      ))}
    </span>
  );
}
