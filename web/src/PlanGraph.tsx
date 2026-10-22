import { HistoryIcon, XIcon } from "lucide-react";
import { useMemo, useRef, useState } from "react";
import { DependencyGraphView, type EdgeLook } from "./DependencyGraph";
import type { GraphEdge } from "./dependencyLayout";
import { StatusMark } from "./components/status-mark";
import { Button } from "./components/ui/button";
import { Popover, PopoverAnchor, PopoverContent } from "./components/ui/popover";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { planGraph, stepMark } from "./plans";
import type { PlanStep, PlanView } from "./snapshot";

// A plan's steps top down by what they wait on (PRD hide-plan D-10, B12):
// the mission map's graph with a one-line card per step. One drawing at two
// sizes: `compact` in the pane chip's popover and folded in a mission node's
// detail, `large` in the 보기 slot, where a card opens its step's detail.

export type PlanGraphSize = "compact" | "large";

export function PlanGraph({ plan, size }: { plan: PlanView; size: PlanGraphSize }) {
  const [selected, setSelected] = useState<string | null>(null);
  const graph = useMemo(() => planGraph(plan), [plan]);
  const byKey = useMemo(() => new Map(plan.steps.map((step) => [step.key, step])), [plan]);
  // A line already walked is faint; one still ahead is bright (D-10).
  const edgeLook = (edge: GraphEdge): EdgeLook => {
    const from = byKey.get(edge.from);
    if (from?.status === "working") return { tone: "text-agent-working" };
    if (from?.status === "done" || from?.status === "dropped") return { tone: "text-border" };
    return { tone: "text-muted-foreground" };
  };
  const large = size === "large";
  return (
    <div data-plan-graph={size}>
      <DependencyGraphView
        graph={graph}
        idOf={(step) => step.key}
        flow="down"
        gaps={large ? { layers: "--size-mission-layer-gap", nodes: "--size-mission-node-gap" } : { layers: "--spacing-lg", nodes: "--spacing-sm" }}
        edgeLook={edgeLook}
        draw={(step) =>
          large ? (
            <StepCard key={step.key} plan={plan} step={step} open={selected === step.key} onOpen={(open) => setSelected(open ? step.key : null)} />
          ) : (
            <StepFace key={step.key} plan={plan} step={step} compact />
          )
        }
      />
    </div>
  );
}

function StepFace({ plan, step, compact, open }: { plan: PlanView; step: PlanStep; compact: boolean; open?: boolean }) {
  const { t } = useInterfaceTranslation();
  const current = plan.progress.current === step.key && !plan.progress.finished;
  return (
    <span
      data-dependency-node={step.key}
      data-plan-step={step.key}
      data-step-status={step.status}
      data-step-current={current || undefined}
      className={cn(
        "flex min-w-0 items-center gap-xs rounded-md border border-border bg-card text-left",
        compact ? "w-(--size-plan-node-compact) px-xs py-xxs" : "w-(--size-mission-node) px-sm py-xs",
        step.waiting && "border-dashed bg-transparent",
        current && "border-agent-working",
        step.status === "dropped" && "opacity-(--opacity-dimmed)",
        open && "ring-1 ring-primary",
      )}
    >
      <StatusMark mark={stepMark(step)} />
      <span className={cn("min-w-0 flex-1 truncate", compact ? "text-caption" : "text-body", step.status === "dropped" && "line-through")} title={step.title}>
        {step.title}
      </span>
      {step.changed ? (
        <Hint label={step.change ?? t("plan.changed")}>
          <HistoryIcon aria-label={t("plan.changed")} className="size-(--size-icon-sm) shrink-0 text-muted-foreground" data-step-changed="true" />
        </Hint>
      ) : null}
      {current ? <span className="shrink-0 text-micro font-semibold text-agent-working">{t("plan.current")}</span> : null}
      {step.output && step.output !== "none" ? <span className="shrink-0 font-mono text-micro text-muted-foreground" data-step-output={step.output}>{t(step.output === "pr" ? "plan.output.pr" : "plan.output.file")}</span> : null}
    </span>
  );
}

/** A step on the large map, which opens its detail: title, state, line, what it waits on, why it changed (B12). */
function StepCard({ plan, step, open, onOpen }: { plan: PlanView; step: PlanStep; open: boolean; onOpen: (open: boolean) => void }) {
  const { t } = useInterfaceTranslation();
  const card = useRef<HTMLButtonElement>(null);
  const depends = step.depends_on.map((key) => plan.steps.find((other) => other.key === key)?.title ?? key);
  const rows: [string, string][] = [
    [t("plan.field.status"), t(`plan.status.${step.status}`)],
    ...(step.line ? [[t("plan.field.line"), step.line] as [string, string]] : []),
    ...(depends.length > 0 ? [[t("plan.field.depends"), depends.join(", ")] as [string, string]] : []),
    ...(step.change ? [[t("plan.field.change"), step.change] as [string, string]] : []),
  ];
  return (
    <Popover open={open} onOpenChange={onOpen}>
      <PopoverAnchor asChild>
        <button
          ref={card}
          type="button"
          onClick={() => onOpen(!open)}
          aria-expanded={open}
          aria-label={`${step.title} · ${t(`plan.status.${step.status}`)}`}
          className="rounded-md text-left outline-none hover:[&>span]:bg-accent focus-visible:ring-1 focus-visible:ring-ring"
        >
          <StepFace plan={plan} step={step} compact={false} open={open} />
        </button>
      </PopoverAnchor>
      <PopoverContent
        align="start"
        side="bottom"
        className="w-(--size-mission-popover)"
        data-plan-step-detail={step.key}
        // The keyboard stays where it was, as everything the 보기 slot opens leaves it.
        onOpenAutoFocus={(event) => event.preventDefault()}
        onCloseAutoFocus={(event) => event.preventDefault()}
        onFocusOutside={(event) => event.preventDefault()}
        onInteractOutside={(event) => {
          if (event.target instanceof Node && card.current?.contains(event.target)) event.preventDefault();
        }}
      >
        <div className="flex flex-col gap-sm">
          <div className="flex min-w-0 items-center gap-xs">
            <StatusMark mark={stepMark(step)} />
            <span className="min-w-0 flex-1 break-words text-body font-semibold">{step.title}</span>
            <Button variant="ghost" size="icon-sm" aria-label={t("plan.closeStep")} onClick={() => onOpen(false)}><XIcon /></Button>
          </div>
          <dl className="grid grid-cols-[auto_1fr] gap-x-md gap-y-xs text-body">
            {rows.map(([label, value]) => (
              <div key={label} className="contents">
                <dt className="text-muted-foreground">{label}</dt>
                <dd className="min-w-0 break-words">{value}</dd>
              </div>
            ))}
          </dl>
        </div>
      </PopoverContent>
    </Popover>
  );
}
