import type { TFunction } from "i18next";
import { ChevronRightIcon, HistoryIcon } from "lucide-react";
import { AgentMark } from "./AgentMark";
import { agentProductName } from "./agentAdapters";
import { MapPanel, MapPanelHead } from "./components/map-page";
import { PlanChecklist } from "./components/plan-checklist";
import { SummaryRow } from "./components/mission-summary";
import { StepNumber } from "./components/step-number";
import { Button } from "./components/ui/button";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { MissionDevice, useMissionDeviceName } from "./missionParts";
import { childNow, childPlanOf, childReachable, evidenceWords, planCount, stepLeft, stepStatusWord } from "./plans";
import type { PlanDelegation, PlanStep, PlanView } from "./snapshot";
import { useShellStore } from "./store";

// The step the operator picked on a plan's map, in the column beside it where
// a mission's node panel stands (hide-private tasks-and-pr-list, A plan's
// map): a head that stays while the panel scrolls (the step's number as its
// state, its title, ✕, and one line saying where in the plan it stands),
// then what it does now, what it waits on, what it left, the agent it was
// handed to with the condition that ends it and that agent's own plan, and
// why the last `set` changed it. A row with nothing to say is drawn as
// `아직 없음` only where the step could still have it; the rest are not drawn.

export function PlanStepPanel({ plan, step, number, onClose, openPane }: { plan: PlanView; step: PlanStep; number: number; onClose: () => void; openPane: (paneId: string) => void }) {
  const { t } = useInterfaceTranslation();
  const delegation = step.delegation;
  const open = step.status === "planned" || step.status === "working";
  const child = useShellStore((s) => childPlanOf(s.rest, step));
  const now = delegation ? childNow(child) : null;
  const nowLine = delegation ? (now ? `${now.step} ${now.progress}` : step.line) : step.line;
  const left = stepLeft(step);
  const nothing = t("plan.panel.nothing");
  return (
    <MapPanel data={{ "data-plan-step-detail": step.key }}>
      <MapPanelHead
        mark={<StepNumber number={number} status={step.status} />}
        title={step.title}
        close={t("plan.closeStep")}
        onClose={onClose}
        facts={
          <>
            <span className="tabular-nums">{t("plan.panel.of", { number, total: plan.steps.length })}</span>
            <span data-panel-status={step.status}>{stepStatusWord(step, t)}</span>
          </>
        }
      />
      <dl className="flex min-w-0 flex-col gap-md">
        {open ? (
          <SummaryRow name={t("plan.field.line")} field="now">
            <Quiet value={nowLine} nothing={nothing} />
          </SummaryRow>
        ) : null}
        <SummaryRow name={t("plan.field.depends")} field="depends">
          <Quiet value={step.depends_on.length > 0 ? dependsWords(plan, step, t) : null} nothing={t("plan.panel.none")} />
        </SummaryRow>
        {step.status !== "dropped" ? (
          <SummaryRow name={t("plan.side.left")} field="left">
            {left?.kind === "evidence" ? (
              <span className="min-w-0 truncate font-mono text-caption text-mission-done" title={left.evidence.kind === "file" ? left.evidence.path : undefined} data-plan-side-evidence={left.evidence.kind}>{evidenceWords(left.evidence)}</span>
            ) : (
              <Quiet value={left?.kind === "report" ? t("plan.delegation.report") : null} nothing={nothing} />
            )}
          </SummaryRow>
        ) : null}
        {delegation ? (
          <SummaryRow name={t("plan.field.agent")} field="agent">
            <Handed step={step} delegation={delegation} openPane={openPane} />
          </SummaryRow>
        ) : null}
        {step.change ? (
          <SummaryRow name={t("plan.field.change")} field="change">
            <span className="flex min-w-0 items-start gap-xs" data-panel-change="true">
              <HistoryIcon aria-hidden="true" className="mt-xxs size-(--size-icon-sm) shrink-0 text-mission-hold" />
              <span className="min-w-0 break-words">{step.change}</span>
            </span>
          </SummaryRow>
        ) : null}
      </dl>
    </MapPanel>
  );
}

/** A value in the body's size, or the dim words saying there is none yet. */
function Quiet({ value, nothing }: { value: string | null; nothing: string }) {
  return value ? <span className="min-w-0 break-words">{value}</span> : <span className="text-muted-foreground">{nothing}</span>;
}

/**
 * The agent a step went to (PRD hide-plan-delegate B27): its mark, name and
 * product, its device when it works away from this window, the condition that
 * ends the step, the child's own plan as `<이름>의 작업 단계 n/m` over the
 * same checklist the pane chip opens, or one line when it works without one,
 * and the way to its pane while it has one. Nothing is said of a plan while
 * the child is still being started (B32).
 */
function Handed({ step, delegation, openPane }: { step: PlanStep; delegation: PlanDelegation; openPane: (paneId: string) => void }) {
  const { t } = useInterfaceTranslation();
  const child = useShellStore((s) => childPlanOf(s.rest, step));
  // A child on a device that does not answer is not said to work without a plan.
  const reachable = useShellStore((s) => childReachable(s.rest, step));
  const device = useMissionDeviceName(delegation.session?.device_id);
  const paneId = delegation.pane_id;
  return (
    <>
      <span className="flex min-w-0 flex-wrap items-center gap-x-xs gap-y-xxs" data-plan-handed={delegation.name}>
        <AgentMark kind={delegation.agent} />
        <span className="min-w-0 break-words font-semibold">{delegation.name}</span>
        <span className="text-caption text-muted-foreground">{agentProductName(delegation.agent)}</span>
        <MissionDevice name={device} />
      </span>
      <span className="text-caption text-muted-foreground" data-plan-ends="true">{t("plan.field.ends")}: {t(step.output === "pr" ? "plan.delegation.endsPr" : "plan.delegation.ends", { name: delegation.name })}</span>
      {child ? (
        <div className="flex min-w-0 flex-col gap-xxs border-t border-border pt-sm text-caption" data-plan-child-plan={delegation.name}>
          <p className="flex items-baseline gap-xs text-muted-foreground">
            {t("plan.delegation.childPlan", { name: delegation.name })}
            <span className="font-semibold tabular-nums text-foreground">{planCount(child)}</span>
          </p>
          <PlanChecklist plan={child} />
        </div>
      ) : delegation.session && reachable ? (
        <p className={cn("border-t border-border pt-sm text-caption text-muted-foreground")} data-plan-child-plan="none">
          {t("plan.delegation.noPlan")}
        </p>
      ) : null}
      {paneId ? (
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
