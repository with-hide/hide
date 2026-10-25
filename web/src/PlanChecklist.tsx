import { CheckIcon, HistoryIcon } from "lucide-react";
import { useLayoutEffect, useMemo, useRef } from "react";
import { StatusMark } from "./components/status-mark";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { planChecklist } from "./plans";
import type { PlanStep, PlanView } from "./snapshot";

// A plan read as a list (PRD mission-plan-visibility D-07, B3-B6): the pane
// chip's popover and a mission node's folded plan. The steps run in the order
// they wait on each other, every title whole, the current one marked, a
// parallel step indented once. In the popover a finished plan folds its steps
// into one line and keeps what they left behind; a session record or a node's
// row, where a finished plan is read afterwards, lists every step. The map is
// drawn only in the 보기 slot.

export function PlanChecklist({ plan, popover = false }: { plan: PlanView; popover?: boolean }) {
  const { t } = useInterfaceTranslation();
  const rows = useMemo(() => planChecklist(plan), [plan]);
  const current = useRef<HTMLLIElement>(null);
  // Opening the popover shows where the plan is, however long the list (B5).
  // Its height is bounded only once it is placed, so the step is revealed
  // again whenever the scrolling box settles to a new size, and follows the
  // current step as it moves.
  const currentKey = plan.progress.current;
  useLayoutEffect(() => {
    const row = current.current;
    if (!popover || !row) return;
    const reveal = () => row.scrollIntoView?.({ block: "nearest" });
    reveal();
    const box = scrollingBox(row);
    if (!box || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(reveal);
    observer.observe(box);
    return () => observer.disconnect();
  }, [popover, currentKey]);
  if (popover && plan.progress.finished) {
    const done = plan.steps.filter((step) => step.status === "done").length;
    const outputs = rows.filter(({ step }) => step.status === "done" && step.output && step.output !== "none");
    return (
      <div className="flex min-w-0 flex-col gap-xxs" data-plan-checklist="finished">
        <p className="flex items-center gap-xs text-muted-foreground" data-plan-done-steps={done}>
          <CheckIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-mission-done" />
          {t("plan.doneSteps", { count: done })}
        </p>
        {outputs.length > 0 ? (
          <ul className="flex min-w-0 flex-col gap-xxs">
            {outputs.map(({ step }) => (
              <li key={step.key} className="flex min-w-0 items-baseline gap-xs" data-plan-output-row={step.key}>
                <span className="min-w-0 flex-1 break-words">{step.title}</span>
                <Output step={step} />
              </li>
            ))}
          </ul>
        ) : null}
      </div>
    );
  }
  return (
    <ol aria-label={t("plan.steplist")} className="flex min-w-0 flex-col" data-plan-checklist="running">
      {rows.map(({ step, parallel }) => {
        const now = step.key === plan.progress.current;
        return (
          <li
            key={step.key}
            ref={now ? current : undefined}
            data-plan-row={step.key}
            data-step-status={step.status}
            data-step-current={now || undefined}
            data-step-parallel={parallel || undefined}
            aria-current={now ? "step" : undefined}
            className={cn(
              "flex min-w-0 items-start gap-xs rounded-xs px-xs py-xxs",
              parallel && "pl-lg",
              now && "bg-mission-working/10 text-foreground",
              step.status === "done" && "text-muted-foreground",
              step.status === "dropped" && "text-muted-foreground line-through opacity-(--opacity-dimmed)",
            )}
          >
            <StepMark step={step} current={now} />
            <span className="min-w-0 flex-1 break-words">{step.title}</span>
            {step.changed ? (
              <Hint label={step.change ?? t("plan.changed")}>
                <span tabIndex={0} className="shrink-0 rounded-xs outline-none focus-visible:ring-1 focus-visible:ring-ring" data-step-changed="true">
                  <HistoryIcon aria-label={t("plan.changed")} className="size-(--size-icon-sm) text-muted-foreground" />
                </span>
              </Hint>
            ) : null}
            {now ? <span className="shrink-0 text-micro font-semibold text-mission-working">{t("plan.current")}</span> : null}
            <Output step={step} />
          </li>
        );
      })}
    </ol>
  );
}

function scrollingBox(element: HTMLElement): HTMLElement | null {
  for (let at = element.parentElement; at; at = at.parentElement) {
    if (/(auto|scroll)/.test(getComputedStyle(at).overflowY)) return at;
  }
  return null;
}

function StepMark({ step, current }: { step: PlanStep; current: boolean }) {
  const { t } = useInterfaceTranslation();
  const label = t(`plan.status.${step.status}`);
  if (step.status === "done") return <CheckIcon role="img" aria-label={label} className="mt-xxs size-(--size-icon-sm) shrink-0 text-mission-done" />;
  if (current || step.status === "working") return <StatusMark mark="going" label={label} className="mt-xxs" />;
  return <span role="img" aria-label={label} className="mt-xxs inline-block size-(--size-agent-mark) shrink-0 rounded-full border border-muted-foreground" />;
}

function Output({ step }: { step: PlanStep }) {
  const { t } = useInterfaceTranslation();
  if (!step.output || step.output === "none") return null;
  return <span className="shrink-0 font-mono text-micro text-muted-foreground" data-step-output={step.output}>{t(step.output === "pr" ? "plan.output.pr" : "plan.output.file")}</span>;
}
