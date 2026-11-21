import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import type { PlanStepStatus } from "../snapshot";

// A plan step's number drawn as its state (hide-private tasks-and-pr-list, A
// plan's map): a green disc for a finished step, a ring in the working colour
// for the one at work, a dashed ring for one still ahead, a plain faint ring
// for one dropped. Where a mission's card leads with a state mark and its
// worker, a plan's card leads with this, since one agent carries the steps in
// order. The number is read from the shape alone; the step's state word is the
// hover and the accessible name of the card it stands on.

const SHAPE: Record<PlanStepStatus, string> = {
  done: "bg-mission-done text-status-foreground",
  working: "border-2 border-mission-working text-mission-working",
  planned: "border border-dashed border-muted-foreground text-muted-foreground",
  dropped: "border border-muted-foreground text-muted-foreground opacity-(--opacity-dimmed)",
};

export function StepNumber({ number, status, className }: { number: number; status: PlanStepStatus; className?: string }) {
  const { t } = useInterfaceTranslation();
  return (
    <span
      role="img"
      aria-label={t("plan.step.number", { number })}
      data-step-number={number}
      data-step-shape={status}
      className={cn("inline-flex h-(--size-icon) min-w-(--size-icon) shrink-0 items-center justify-center rounded-full px-xxs font-mono text-micro font-semibold tabular-nums leading-none", SHAPE[status], className)}
    >
      {number}
    </span>
  );
}
