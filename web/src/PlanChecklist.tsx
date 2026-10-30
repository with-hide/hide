import { CheckIcon, HistoryIcon } from "lucide-react";
import { useLayoutEffect, useMemo, useRef } from "react";
import { AgentMark } from "./AgentMark";
import { StatusMark } from "./components/status-mark";
import { Button } from "./components/ui/button";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { childNow, childPlanOf, evidenceWords, planChecklist, stepDelegationWord, type StepMarkWord } from "./plans";
import type { PlanDelegation, PlanStep, PlanView } from "./snapshot";
import { useShellStore } from "./store";

// A plan read as a list (PRD mission-plan-visibility D-07, B3-B5): the pane
// chip's popover, a mission node's folded plan, the session record and a
// handed step's detail on the map. The steps run in the order they wait on
// each other, every title whole, the current one marked, a parallel step
// indented once, and a finished plan still lists every step (PRD
// hide-plan-delegate B30). A done step's tail is what it left, a handed step
// names its child and the child's current step, and the child's name goes to
// its pane where `openPane` is given (B26, B28). The map is drawn only in the
// 보기 slot.

export function PlanChecklist({ plan, popover = false, openPane }: { plan: PlanView; popover?: boolean; openPane?: (paneId: string) => void }) {
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
            <span className="min-w-0 flex-1 break-words">
              {step.title}
              {step.delegation && step.status !== "done" ? <ChildNow step={step} /> : null}
            </span>
            {step.changed ? (
              <Hint label={step.change ?? t("plan.changed")}>
                <span tabIndex={0} className="shrink-0 rounded-xs outline-none focus-visible:ring-1 focus-visible:ring-ring" data-step-changed="true">
                  <HistoryIcon aria-label={t("plan.changed")} className="size-(--size-icon-sm) text-muted-foreground" />
                </span>
              </Hint>
            ) : null}
            <StepTail step={step} now={now} />
            {step.delegation ? <ChildTag delegation={step.delegation} openPane={openPane} /> : null}
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

/**
 * A row's right end (B26, B28): a handed step's short word (보고 옴, 막힘,
 * 닫힘, 맡길 예정), else `지금`, else what a done step left, `#n` or the
 * file's name, else the kind it will leave. A step done before evidence was
 * recorded keeps its kind; a handed step done with nothing to leave says its
 * child's report ended it (B13). A child still at work has no tail: its name
 * and its current step say it.
 */
function StepTail({ step, now }: { step: PlanStep; now: boolean }) {
  const { t } = useInterfaceTranslation();
  const word = stepDelegationWord(step);
  if (word) return <DelegationWord word={word} />;
  if (now) return <span className="shrink-0 text-micro font-semibold text-mission-working">{t("plan.current")}</span>;
  if (step.status === "done" && step.evidence) {
    return <span className="shrink-0 font-mono text-micro text-mission-done" data-step-evidence={step.evidence.kind}>{evidenceWords(step.evidence)}</span>;
  }
  if (step.status === "done" && step.delegation && (!step.output || step.output === "none")) {
    return <span className="shrink-0 text-micro text-muted-foreground" data-step-evidence="report">{t("plan.delegation.report")}</span>;
  }
  if (step.delegation && step.status !== "done") return null;
  return <Output step={step} />;
}

const WORD_TONE: Record<StepMarkWord, string> = {
  reported: "font-semibold text-warning",
  blocked: "font-semibold text-warning",
  closed: "text-muted-foreground",
  pending: "text-muted-foreground",
};

/** `보고 옴`, `막힘`, `닫힘` or `맡길 예정`: one short word, where a step's tail goes. */
export function DelegationWord({ word }: { word: StepMarkWord }) {
  const { t } = useInterfaceTranslation();
  return <span className={cn("shrink-0 text-micro", WORD_TONE[word])} data-step-delegation={word}>{t(`plan.delegation.${word}`)}</span>;
}

/** ` · <the child's current step>` after a handed step's title, from the child's own plan; nothing while the child has no plan (B28, B32). */
function ChildNow({ step }: { step: PlanStep }) {
  const child = useShellStore((s) => childPlanOf(s.rest, step));
  const now = childNow(child);
  if (!now) return null;
  return <span className="text-muted-foreground" data-step-child-now="true"> · {now.step}</span>;
}

/**
 * The child's name tag, its agent's mark and its name (B28). It goes to the
 * child's pane while the pane is shown and the surface can go there; a child
 * asleep, closed or still starting has no pane, and its tag is only a name.
 */
export function ChildTag({ delegation, openPane }: { delegation: PlanDelegation; openPane?: (paneId: string) => void }) {
  const { t } = useInterfaceTranslation();
  const paneId = delegation.pane_id;
  const face = (
    <>
      <AgentMark kind={delegation.agent} />
      <span className="min-w-0 truncate">{delegation.name}</span>
    </>
  );
  if (!paneId || !openPane) {
    return <span className="flex min-w-0 max-w-1/2 shrink-0 items-center gap-xxs text-micro text-subtle-foreground" data-step-child={delegation.name}>{face}</span>;
  }
  return (
    <Button
      variant="text"
      size="inline"
      className="flex min-w-0 max-w-1/2 shrink-0 items-center gap-xxs text-micro text-subtle-foreground hover:text-foreground"
      aria-label={`${delegation.name}, ${t("record.toPane")}`}
      data-step-child={delegation.name}
      data-step-child-pane={paneId}
      onClick={() => openPane(paneId)}
    >
      {face}
    </Button>
  );
}

function Output({ step }: { step: PlanStep }) {
  const { t } = useInterfaceTranslation();
  if (!step.output || step.output === "none") return null;
  return <span className="shrink-0 font-mono text-micro text-muted-foreground" data-step-output={step.output}>{t(step.output === "pr" ? "plan.output.pr" : "plan.output.file")}</span>;
}
