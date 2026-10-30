import type { TFunction } from "i18next";
import { ChevronRightIcon, HistoryIcon, XIcon } from "lucide-react";
import { useMemo, useRef, useState } from "react";
import { agentProductName } from "./agentAdapters";
import { AgentMark } from "./AgentMark";
import { markWords } from "./agentRow";
import { DependencyGraphView, type EdgeLook } from "./DependencyGraph";
import type { GraphEdge } from "./dependencyLayout";
import { StatusMark } from "./components/status-mark";
import { Button } from "./components/ui/button";
import { Popover, PopoverAnchor, PopoverContent } from "./components/ui/popover";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { usePaneAgents } from "./PaneHeaderBand";
import { DelegationWord, PlanChecklist } from "./PlanChecklist";
import { childNow, childPlanOf, evidenceWords, planCount, planGraph, stepDelegationWord, stepMark, waitingOn } from "./plans";
import type { PlanDelegation, PlanStep, PlanView } from "./snapshot";
import { useShellStore } from "./store";

// A plan's steps top down by what they wait on (PRD hide-plan D-10, B12):
// the mission map's graph with a two-line card per step, all one height
// (PRD hide-plan-delegate D-33), drawn only in the 보기 slot, where a card
// opens its step's detail. Everywhere else a plan is
// read as its checklist (PRD mission-plan-visibility D-07, `PlanChecklist`).

export function PlanGraph({ plan, openPane }: { plan: PlanView; openPane?: (paneId: string) => void }) {
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
  return (
    <div data-plan-graph="large">
      <DependencyGraphView
        graph={graph}
        idOf={(step) => step.key}
        flow="down"
        gaps={{ layers: "--size-mission-layer-gap", nodes: "--size-mission-node-gap" }}
        edgeLook={edgeLook}
        draw={(step) => <StepCard key={step.key} plan={plan} step={step} open={selected === step.key} onOpen={(open) => setSelected(open ? step.key : null)} openPane={openPane} />}
      />
    </div>
  );
}

function StepFace({ plan, step, open }: { plan: PlanView; step: PlanStep; open: boolean }) {
  const { t } = useInterfaceTranslation();
  const current = plan.progress.current === step.key && !plan.progress.finished;
  const word = stepDelegationWord(step);
  return (
    <span
      data-dependency-node={step.key}
      data-plan-step={step.key}
      data-step-status={step.status}
      data-step-current={current || undefined}
      className={cn(
        "flex w-(--size-mission-node) min-w-0 flex-col gap-xxs rounded-md border border-border bg-card px-sm py-xs text-left",
        step.waiting && "border-dashed border-muted-foreground bg-transparent",
        current && "border-agent-working",
        step.status === "dropped" && "opacity-(--opacity-dimmed)",
        open && "ring-1 ring-primary",
      )}
    >
      <span className="flex min-w-0 items-center gap-xs">
        <StatusMark mark={stepMark(step)} />
        <span className={cn("min-w-0 flex-1 truncate text-body", step.status === "dropped" && "line-through")} title={step.title}>
          {step.title}
        </span>
        {step.changed ? (
          <Hint label={step.change ?? t("plan.changed")}>
            <HistoryIcon aria-label={t("plan.changed")} className="size-(--size-icon-sm) shrink-0 text-muted-foreground" data-step-changed="true" />
          </Hint>
        ) : null}
        {word && word !== "pending" ? (
          <DelegationWord word={word} />
        ) : current ? (
          <span className="shrink-0 text-micro font-semibold text-agent-working">{t("plan.current")}</span>
        ) : step.output && step.output !== "none" ? (
          <span className="shrink-0 font-mono text-micro text-muted-foreground" data-step-output={step.output}>{t(step.output === "pr" ? "plan.output.pr" : "plan.output.file")}</span>
        ) : null}
      </span>
      <CardLine plan={plan} step={step} />
    </span>
  );
}

/**
 * A card's second line, `누가 · 지금 뭐` (PRD hide-plan-delegate B25, D-33):
 * `나` or the child's mark and name, then the main's line or `예정`, the
 * child's current step and its count, what a done step left, or, for a step
 * still to be handed, the steps it waits on. A child with no plan read yet
 * shows its row's state, and a child with neither shows its name alone (B32).
 */
function CardLine({ plan, step }: { plan: PlanView; step: PlanStep }) {
  const { t } = useInterfaceTranslation();
  const child = useShellStore((s) => childPlanOf(s.rest, step));
  const agents = usePaneAgents();
  const delegation = step.delegation;
  const pending = stepDelegationWord(step) === "pending";
  const what = (() => {
    if (pending) {
      const waits = waitingOn(plan, step).map((other) => other.title);
      return waits.length > 0 ? t("plan.delegation.pendingWaits", { steps: waits.join("·") }) : t("plan.delegation.pending");
    }
    if (step.status === "dropped") return null;
    if (step.status === "done") {
      if (step.evidence) return evidenceWords(step.evidence);
      return delegation && (!step.output || step.output === "none") ? t("plan.delegation.report") : null;
    }
    if (delegation) {
      const now = childNow(child);
      if (now) return `${now.step} ${now.progress}`;
      const row = delegation.pane_id ? agents.find((agent) => agent.pane_id === delegation.pane_id) : undefined;
      return row ? markWords(t, row) : null;
    }
    return step.status === "working" ? step.line : t("plan.status.planned");
  })();
  return (
    <span className="flex min-w-0 items-center gap-xxs pl-(--size-agent-mark) text-micro text-muted-foreground" data-step-line={step.key}>
      <span className="w-xs shrink-0" aria-hidden="true" />
      {pending ? null : delegation ? (
        <span className="flex min-w-0 max-w-1/2 shrink-0 items-center gap-xxs text-foreground" data-step-who="child">
          <AgentMark kind={delegation.agent} />
          <span className="min-w-0 truncate">{delegation.name}</span>
        </span>
      ) : (
        <span className="shrink-0 text-subtle-foreground" data-step-who="me">{t("plan.me")}</span>
      )}
      {what && !pending ? <span aria-hidden="true">·</span> : null}
      {what ? (
        <span className={cn("min-w-0 truncate", step.evidence && step.status === "done" && "font-mono text-mission-done")} title={what} data-step-what="true">
          {what}
        </span>
      ) : null}
    </span>
  );
}

/**
 * A step on the large map, which opens its detail: title, state, line, what it
 * waits on, why it changed (B12); a handed step adds its child, the condition
 * that ends it, the child's own plan and a way to its pane (PRD
 * hide-plan-delegate B27).
 */
function StepCard({ plan, step, open, onOpen, openPane }: { plan: PlanView; step: PlanStep; open: boolean; onOpen: (open: boolean) => void; openPane?: (paneId: string) => void }) {
  const { t } = useInterfaceTranslation();
  const card = useRef<HTMLButtonElement>(null);
  const delegation = step.delegation;
  const word = stepDelegationWord(step);
  const status = step.status !== "done" && delegation ? t(`plan.delegation.state.${delegation.state}`) : word === "pending" ? t("plan.delegation.pending") : t(`plan.status.${step.status}`);
  const rows: [string, string][] = [
    ...(delegation ? [[t("plan.field.agent"), [delegation.name, agentProductName(delegation.agent)].join(" · ")] as [string, string]] : []),
    [t("plan.field.status"), status],
    ...(delegation ? [[t("plan.field.ends"), t(step.output === "pr" ? "plan.delegation.endsPr" : "plan.delegation.ends", { name: delegation.name })] as [string, string]] : []),
    ...(step.line ? [[t("plan.field.line"), step.line] as [string, string]] : []),
    ...(step.depends_on.length > 0 ? [[t("plan.field.depends"), dependsWords(plan, step, t)] as [string, string]] : []),
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
          aria-label={`${step.title} · ${status}`}
          className="rounded-md text-left outline-none hover:[&>span]:bg-accent focus-visible:ring-1 focus-visible:ring-ring"
        >
          <StepFace plan={plan} step={step} open={open} />
        </button>
      </PopoverAnchor>
      <PopoverContent
        align="start"
        side="bottom"
        className="max-h-(--radix-popover-content-available-height) w-(--size-mission-popover) overflow-y-auto"
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
          {delegation ? (
            <ChildDetail
              step={step}
              delegation={delegation}
              openPane={
                openPane
                  ? (paneId) => {
                      onOpen(false);
                      openPane(paneId);
                    }
                  : undefined
              }
            />
          ) : null}
        </div>
      </PopoverContent>
    </Popover>
  );
}

/**
 * The child's own plan under a handed step's detail (B27): `<이름>의 계획 n/m`
 * over the same checklist the pane chip opens, or one line when the child
 * works without one, and the way to its pane while it has one. Nothing is
 * said of a plan while the child is still being started (B32).
 */
function ChildDetail({ step, delegation, openPane }: { step: PlanStep; delegation: PlanDelegation; openPane?: (paneId: string) => void }) {
  const { t } = useInterfaceTranslation();
  const child = useShellStore((s) => childPlanOf(s.rest, step));
  const paneId = delegation.pane_id;
  return (
    <>
      {child ? (
        <div className="flex min-w-0 flex-col gap-xxs border-t border-border pt-sm text-caption" data-plan-child-plan={delegation.name}>
          <p className="flex items-baseline gap-xs text-muted-foreground">
            {t("plan.delegation.childPlan", { name: delegation.name })}
            <span className="font-semibold tabular-nums text-foreground">{planCount(child)}</span>
          </p>
          <PlanChecklist plan={child} />
        </div>
      ) : delegation.session ? (
        <p className="border-t border-border pt-sm text-caption text-muted-foreground" data-plan-child-plan="none">
          {t("plan.delegation.noPlan")}
        </p>
      ) : null}
      {paneId && openPane ? (
        <Button variant="secondary" size="sm" className="self-start" aria-label={`${delegation.name}, ${t("record.toPane")}`} onClick={() => openPane(paneId)} data-plan-child-go={paneId}>
          {t("record.toPane")}
          <ChevronRightIcon aria-hidden="true" />
        </Button>
      ) : null}
    </>
  );
}

/** What a step waits on with each one's state, steps alike in state joined: `코어 이벤트, 웹 화면 (끝남)` (B27). */
function dependsWords(plan: PlanView, step: PlanStep, t: TFunction<"translation">): string {
  const groups: { status: PlanStep["status"] | null; titles: string[] }[] = [];
  for (const key of step.depends_on) {
    const other = plan.steps.find((one) => one.key === key);
    const status = other?.status ?? null;
    const group = groups.find((one) => one.status === status);
    if (group) group.titles.push(other?.title ?? key);
    else groups.push({ status, titles: [other?.title ?? key] });
  }
  return groups.map(({ status, titles }) => (status ? `${titles.join(", ")} (${t(`plan.status.${status}`)})` : titles.join(", "))).join(", ");
}
