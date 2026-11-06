import type { Actions } from "./actions";
import { agentProductName } from "./agentAdapters";
import { AgentMark } from "./AgentMark";
import { markWords } from "./agentRow";
import { formatElapsed, useElapsed } from "./components/elapsed";
import { OWNER_ROW } from "./components/mission-conductor-row";
import { MapPage, MapSplit } from "./components/map-page";
import { SummaryRow } from "./components/mission-summary";
import { PILL_TONE, ProgressCells, WorkPill } from "./components/mission-tone";
import { ChildTag, DelegationWord } from "./components/plan-checklist";
import { StatusMark } from "./components/status-mark";
import { Button } from "./components/ui/button";
import type { MessageKey } from "./i18n/catalogs";
import { useInterfaceTranslation } from "./i18n/client";
import { requireInterfaceLanguage } from "./i18n/locale";
import { cn } from "./lib/utils";
import { usePaneAgents } from "./PaneHeaderBand";
import { useMapPick } from "./mapPick";
import { PlanGraph } from "./PlanGraph";
import { PlanStepPanel } from "./PlanStepPanel";
import { evidenceWords, planCells, planOfSession, planProposed, stepDelegationWord, stepOf } from "./plans";
import type { PlanStep, PlanView, WorkSession } from "./snapshot";
import { useShellStore } from "./store";

// A session's plan drawn large in the lane's 보기 slot (PRD hide-plan B11-B13),
// in the mission page's shape so the two read alike: the head (state, time,
// a cell per step, `k / m 끝`), one band while the plan waits for the
// operator's review (answered in the conversation, so its one button goes
// there), the session that owns the plan with the way to its terminal, and
// the map with the plan's summary beside it when the page is wide and under
// it when narrow. Only a mission adds its decision, criteria, merge order and
// workers.

export function PlanMap({ session, actions }: { session: WorkSession; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const plan = useShellStore((s) => planOfSession(s.rest, session));
  if (!plan) return <p className="px-md py-md text-body text-muted-foreground" data-plan-state="gone">{t("plan.gone")}</p>;
  return <PlanBody plan={plan} actions={actions} />;
}

export type PlanState = "review" | "finished" | "running";

/** A plan's state pill: `검토` in the warning colour while it waits for review, a grey `✓ 끝남` once finished, else `진행 중` as a running mission's. */
export const PLAN_PILL: Record<PlanState, { key: MessageKey; tone: string }> = {
  review: { key: "plan.review", tone: PILL_TONE.hold },
  finished: { key: "mission.pill.done", tone: PILL_TONE.quiet },
  running: { key: "mission.pill.running", tone: PILL_TONE.working },
};

export function planState(plan: PlanView): PlanState {
  return planProposed(plan) ? "review" : plan.progress.finished ? "finished" : "running";
}

/** The pill, the plan's cells and its count, the head both the popover and the page start with. */
export function PlanPill({ plan }: { plan: PlanView }) {
  const { t } = useInterfaceTranslation();
  const state = planState(plan);
  const pill = PLAN_PILL[state];
  return (
    <WorkPill tone={pill.tone} done={state === "finished"} data-plan-pill={state}>
      {t(pill.key)}
    </WorkPill>
  );
}

/**
 * `진행 중 · 32분째 · ▬▬▬ · 2 / 5 끝 · 3분 전 갱신`; `검토 · 8단계` while it
 * waits for review (B12), and `✓ 끝남 · 1시간 걸림`, from the plan's making to
 * its last update, once finished (PRD mission-plan-visibility B6).
 */
function PlanHead({ plan }: { plan: PlanView }) {
  const { t, i18n } = useInterfaceTranslation();
  const state = planState(plan);
  const running = useElapsed(state === "finished" ? null : plan.created_at_unix_ms);
  const updated = useElapsed(state === "finished" ? null : plan.updated_at_unix_ms);
  const took = state === "finished" ? formatElapsed(requireInterfaceLanguage(i18n.language), plan.updated_at_unix_ms - plan.created_at_unix_ms) : null;
  const time = took ? t("mission.elapsed.total", { elapsed: took }) : running ? t("mission.elapsed.running", { elapsed: running }) : null;
  const count = state === "review" ? t("plan.steps", { count: plan.progress.total }) : t("mission.doneOf", { done: String(plan.progress.done), total: String(plan.progress.total) });
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-x-md gap-y-xs px-md py-sm text-body" data-plan-summary={state}>
      <PlanPill plan={plan} />
      {time ? <span className="shrink-0 text-caption text-muted-foreground" data-plan-stat="time">{time}</span> : null}
      <ProgressCells tones={planCells(plan)} size="head" className="max-w-(--size-pr-popover)" />
      <span className="shrink-0 text-caption text-muted-foreground tabular-nums" data-plan-stat="done">{count}</span>
      {updated ? <span className="shrink-0 text-caption text-muted-foreground" data-plan-summary-when="true">{t("plan.updated", { elapsed: updated })}</span> : null}
    </div>
  );
}

/** The session that owns the plan, under the head as a mission's Conductor is: its agent, its state and the way to its terminal. */
function PlanOwner({ plan, actions }: { plan: PlanView; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const row = usePaneAgents().find((agent) => agent.pane_id === plan.pane_id) ?? null;
  const product = agentProductName(plan.session.agent) ?? plan.session.agent;
  const word = row ? markWords(t, row) : null;
  const terminal = t("mission.toTerminal");
  return (
    <div className={OWNER_ROW} data-plan-owner={row?.mark ?? "unknown"}>
      <AgentMark kind={plan.session.agent} />
      <span className="shrink-0 font-semibold">{product}</span>
      {word ? <span className={cn("min-w-0 flex-1 truncate text-caption text-muted-foreground", row?.mark === "going" && "text-mission-working")}>{word}</span> : <span className="flex-1" />}
      <Button size="sm" variant="ghost" aria-label={[terminal, product, word].filter(Boolean).join(" · ")} onClick={() => actions.openAgent(plan.pane_id)} data-plan-owner-terminal="true">
        {terminal}
      </Button>
    </div>
  );
}

function PlanBody({ plan, actions }: { plan: PlanView; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const proposed = planProposed(plan);
  const openPane = (paneId: string) => actions.openAgent(paneId);
  // The pick works as a mission's does: a card opens its panel beside the map, Escape or a press on the map's empty ground lets go of it (B30).
  const { selected, select, panel, onKeyDown, onMapClick } = useMapPick(
    plan.steps.map((one) => one.key),
    "[data-plan-step]",
  );
  const step = selected ? plan.steps.find((one) => one.key === selected)! : null;
  return (
    <MapPage
      data={{ "data-plan": plan.pane_id, "data-plan-review": plan.review, "data-plan-finished": plan.progress.finished || undefined }}
      scrollData={{ "data-plan-scroll": "true" }}
      onKeyDown={onKeyDown}
    >
      <PlanHead plan={plan} />
      {proposed ? (
        <div className="mx-md mb-sm flex min-w-0 items-center gap-sm rounded-md border border-mission-hold px-sm py-xs" data-plan-banner="review">
          <StatusMark mark="turn" />
          <span className="min-w-0 flex-1 truncate text-body text-warning" title={plan.ask ?? undefined}>
            {t("plan.reviewBanner")}
          </span>
          <Button size="sm" variant="secondary" onClick={() => actions.openAgent(plan.pane_id)} data-plan-conversation="true">
            {t("plan.toConversation")}
          </Button>
        </div>
      ) : null}
      {proposed && plan.ask ? <p className="px-md pb-sm text-caption text-muted-foreground" data-plan-ask="true">{plan.ask}</p> : null}
      <PlanOwner plan={plan} actions={actions} />
      <MapSplit
        onMapClick={onMapClick}
        mapData={{ "data-plan-map": "true" }}
        map={
          // A plan's map has no zoom of its own: its box scrolls while the page is wide.
          <div className="min-w-0 px-md pb-md pt-sm mission-wide:min-h-0 mission-wide:flex-1 mission-wide:overflow-auto">
            <PlanGraph plan={plan} selected={selected} onSelect={(key) => select(selected === key ? null : key)} />
          </div>
        }
        sideRef={panel}
        sideData={{ "data-plan-panel": step?.key }}
        side={
          /* The column beside the map: a pick's panel, else the plan's summary. */
          step ? (
            <PlanStepPanel plan={plan} step={step} number={plan.steps.indexOf(step) + 1} onClose={() => select(null)} openPane={openPane} />
          ) : (
            <PlanSide plan={plan} openPane={openPane} />
          )
        }
      />
    </MapPage>
  );
}

/**
 * The column beside the map, as a mission's summary: the step at work and its
 * line, the step after it, what the done steps left and the agents the plan
 * handed steps to, each with where it stands. A row with nothing in it is
 * not drawn.
 */
function PlanSide({ plan, openPane }: { plan: PlanView; openPane: (paneId: string) => void }) {
  const { t } = useInterfaceTranslation();
  const now = plan.progress.finished ? null : stepOf(plan, plan.progress.current);
  const next = plan.progress.finished ? null : stepOf(plan, plan.progress.next);
  const left = plan.steps.flatMap((step) => (step.status === "done" && step.evidence ? [{ step, evidence: step.evidence }] : []));
  const handed = plan.steps.filter((step) => step.delegation && step.status !== "dropped");
  return (
    <section aria-label={t("mission.summary")} className="flex min-w-0 flex-col px-md py-sm" data-plan-side="summary">
      <dl className="flex min-w-0 flex-col gap-md text-body">
        {now ? (
          <SummaryRow name={t("mission.field.now")} field="now">
            <span className="min-w-0 break-words">{now.title}</span>
            {now.line ? <span className="truncate text-caption text-muted-foreground" title={now.line}>{now.line}</span> : null}
          </SummaryRow>
        ) : null}
        {next ? (
          <SummaryRow name={t("plan.side.next")} field="next">
            <span className="min-w-0 break-words">{next.title}</span>
          </SummaryRow>
        ) : null}
        {left.length > 0 ? (
          <SummaryRow name={t("plan.side.left")} field="left">
            <span className="flex min-w-0 flex-wrap gap-x-sm gap-y-xxs font-mono text-caption text-mission-done">
              {left.map(({ step, evidence }) => (
                <span key={step.key} className="min-w-0 truncate" title={step.title} data-plan-side-evidence={evidence.kind}>{evidenceWords(evidence)}</span>
              ))}
            </span>
          </SummaryRow>
        ) : null}
        {handed.length > 0 ? (
          <SummaryRow name={t("plan.side.handed")} field="handed">
            {handed.map((step) => <HandedRow key={step.key} step={step} openPane={openPane} />)}
          </SummaryRow>
        ) : null}
      </dl>
    </section>
  );
}

/** An agent a step went to: its tag, which goes to its pane, and where it stands (`작업 중`, `보고 옴`, `막힘`, `닫힘`, `끝남`). */
function HandedRow({ step, openPane }: { step: PlanStep; openPane: (paneId: string) => void }) {
  const { t } = useInterfaceTranslation();
  const delegation = step.delegation!;
  const word = stepDelegationWord(step);
  return (
    <span className="flex min-w-0 items-center gap-xs" data-plan-side-handed={step.key}>
      <ChildTag delegation={delegation} openPane={openPane} />
      <span className="ml-auto shrink-0">
        {word ? (
          <DelegationWord word={word} />
        ) : (
          <span className={cn("text-micro", step.status === "done" ? "text-mission-done" : "text-mission-working")}>
            {step.status === "done" ? t("plan.status.done") : t("plan.delegation.state.working")}
          </span>
        )}
      </span>
    </span>
  );
}

