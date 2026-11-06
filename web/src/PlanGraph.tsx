import { CircleDashedIcon, HistoryIcon } from "lucide-react";
import { useMemo } from "react";
import { AgentMark } from "./AgentMark";
import { markWords } from "./agentRow";
import { DependencyGraphView, type EdgeLook } from "./DependencyGraph";
import type { GraphEdge } from "./dependencyLayout";
import { StepNumber } from "./components/step-number";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { usePaneAgents } from "./PaneHeaderBand";
import { DelegationWord } from "./components/plan-checklist";
import { MissionDevice, useMissionDeviceName } from "./missionParts";
import { neighbours } from "./missions";
import { childNow, childPlanOf, evidenceWords, planGraph, stepDelegationWord, stepLeft, stepStatusWord, waitingOn } from "./plans";
import type { PlanStep, PlanView } from "./snapshot";
import { useShellStore } from "./store";

// A plan's steps top down by what they wait on (PRD hide-plan D-10, B12):
// the mission map's graph with a two-line card per step, all one height
// (PRD hide-plan-delegate D-33), drawn only in the 보기 slot. A card leads
// with its step's number drawn as its state, where a mission's card leads
// with the state and its worker, since one agent carries the steps in turn;
// only a step handed to another agent names that agent. A card is picked as a
// mission's is (`useMapPick`): the page opens the step's panel beside the map
// (`PlanStepPanel`) and the cards one edge away stay lit. Everywhere else a
// plan is read as its checklist (PRD mission-plan-visibility D-07,
// `PlanChecklist`).

export function PlanGraph({ plan, selected, onSelect }: { plan: PlanView; selected: string | null; onSelect: (key: string) => void }) {
  const graph = useMemo(() => planGraph(plan), [plan]);
  const byKey = useMemo(() => new Map(plan.steps.map((step) => [step.key, step])), [plan]);
  const numbers = useMemo(() => new Map(plan.steps.map((step, index) => [step.key, index + 1])), [plan]);
  // A pick keeps itself and every card one edge away lit (B30).
  const lit = selected ? neighbours(graph.edges, selected) : null;
  const away = (edge: GraphEdge) => selected !== null && edge.from !== selected && edge.to !== selected;
  // A line already walked is faint; one still ahead is bright (D-10).
  const edgeLook = (edge: GraphEdge): EdgeLook => {
    const from = byKey.get(edge.from);
    const dimmed = away(edge);
    if (from?.status === "working") return { tone: "text-mission-working", dimmed };
    if (from?.status === "done" || from?.status === "dropped") return { tone: "text-border", dimmed };
    return { tone: "text-muted-foreground", dimmed };
  };
  return (
    <div data-plan-graph="large">
      <DependencyGraphView
        graph={graph}
        idOf={(step) => step.key}
        flow="down"
        gaps={{ layers: "--size-mission-layer-gap", nodes: "--size-mission-node-gap" }}
        edgeLook={edgeLook}
        draw={(step) => (
          <StepCard key={step.key} plan={plan} step={step} number={numbers.get(step.key)!} selected={selected === step.key} dimmed={lit !== null && !lit.has(step.key)} onSelect={() => onSelect(step.key)} />
        )}
      />
    </div>
  );
}

/**
 * A step on the large map, which opens its panel (B12): its number as its
 * state, its title with the history mark when the last `set` changed it, and
 * `지금`, its output kind or a handed step's `보고 옴`, `막힘` or `닫힘` at the
 * end of the line; the second line is `누가 · 지금 뭐` (`CardLine`).
 */
function StepCard({ plan, step, number, selected, dimmed, onSelect }: { plan: PlanView; step: PlanStep; number: number; selected: boolean; dimmed: boolean; onSelect: () => void }) {
  const { t } = useInterfaceTranslation();
  const word = stepDelegationWord(step);
  const current = plan.progress.current === step.key && !plan.progress.finished;
  const status = stepStatusWord(step, t);
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-pressed={selected}
      aria-label={`${step.title} · ${status}`}
      data-dependency-node={step.key}
      data-plan-step={step.key}
      data-step-status={step.status}
      data-step-current={current || undefined}
      data-step-dimmed={dimmed || undefined}
      className={cn(
        "flex w-(--size-mission-node) min-w-0 flex-col gap-xxs rounded-md border border-border bg-card px-sm py-xs text-left outline-none transition-opacity hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring",
        // The mission map's card looks (PRD mission-map-git-criteria B26): finished work a faint green surface, work now the working colour, a report or a block the decision colour.
        step.status === "done" && "bg-mission-done/10",
        step.waiting && "border-dashed border-muted-foreground bg-transparent",
        current && "border-mission-working ring-2 ring-mission-working/20",
        (word === "reported" || word === "blocked") && "border-mission-hold ring-2 ring-mission-hold/20",
        step.status === "dropped" && "opacity-(--opacity-dimmed)",
        selected && "outline-2 outline-offset-2 outline-primary",
        dimmed && "opacity-(--opacity-dimmed)",
      )}
    >
      <span className="flex min-w-0 items-center gap-xs">
        <StepNumber number={number} status={step.status} />
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
          <span className="shrink-0 text-micro font-semibold text-mission-working">{t("plan.current")}</span>
        ) : step.output && step.output !== "none" ? (
          <span className="shrink-0 font-mono text-micro text-muted-foreground" data-step-output={step.output}>{t(step.output === "pr" ? "plan.output.pr" : "plan.output.file")}</span>
        ) : null}
      </span>
      <CardLine plan={plan} step={step} />
    </button>
  );
}

/**
 * A card's second line (PRD hide-plan-delegate B25, D-33): for a step the
 * session does itself, only what is now, its own line or `예정`, what a done
 * step left, since the agent is the page's owner row; for a handed step the
 * child's mark and name, its device when it works away from this window, then
 * the child's current step and its count; for a step still to be handed, a
 * dashed circle and the steps it waits on. A child with no plan read yet shows
 * its row's state, and a child with neither shows its name alone (B32).
 */
function CardLine({ plan, step }: { plan: PlanView; step: PlanStep }) {
  const { t } = useInterfaceTranslation();
  const child = useShellStore((s) => childPlanOf(s.rest, step));
  const agents = usePaneAgents();
  const delegation = step.delegation;
  const device = useMissionDeviceName(delegation?.session?.device_id);
  const pending = stepDelegationWord(step) === "pending";
  const left = stepLeft(step);
  const what = (() => {
    if (pending) {
      const waits = waitingOn(plan, step).map((other) => other.title);
      return waits.length > 0 ? t("plan.delegation.waits", { steps: waits.join("·") }) : null;
    }
    if (step.status === "dropped") return null;
    if (step.status === "done") return left ? (left.kind === "evidence" ? evidenceWords(left.evidence) : t("plan.delegation.report")) : null;
    if (delegation) {
      const now = childNow(child);
      if (now) return `${now.step} ${now.progress}`;
      const row = delegation.pane_id ? agents.find((agent) => agent.pane_id === delegation.pane_id) : undefined;
      return row ? markWords(t, row) : null;
    }
    return step.status === "working" ? step.line : t("plan.status.planned");
  })();
  if (!pending && !delegation && !what) return null;
  return (
    <span className="flex min-w-0 items-center gap-xs pl-[calc(var(--size-icon)+var(--spacing-xs))] text-micro text-muted-foreground" data-step-line={step.key}>
      {pending ? (
        <Hint label={t("plan.delegation.pending")}>
          <CircleDashedIcon aria-label={t("plan.delegation.pending")} className="size-(--size-agent-mark) shrink-0 text-muted-foreground" data-step-who="pending" />
        </Hint>
      ) : delegation ? (
        <span className="flex min-w-0 max-w-1/2 shrink-0 items-center gap-xxs text-foreground" data-step-who="child">
          <AgentMark kind={delegation.agent} />
          <span className="min-w-0 truncate">{delegation.name}</span>
        </span>
      ) : null}
      {device ? <MissionDevice name={device} className="max-w-[calc(var(--size-mission-node)/3)] shrink-0" /> : null}
      {what && (pending || delegation) ? <span aria-hidden="true">·</span> : null}
      {what ? (
        <span className={cn("min-w-0 truncate", left?.kind === "evidence" && "font-mono text-mission-done")} title={what} data-step-what="true">
          {what}
        </span>
      ) : null}
    </span>
  );
}
