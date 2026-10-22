import type { TFunction } from "i18next";
import { layerDependencies, type LayeredGraph } from "./projectBoard";
import type { PlanStep, PlanView, RowMark, SnapshotRest, WorkSession } from "./snapshot";

// A session's own plan (PRD hide-plan): what the chip, the map and a
// mission node's detail read of the core's `PlanView`. The core decides the
// current step, the count and which steps wait; the shell words and draws.

/** The plan of the session the pane `paneId` runs, as the agent rows name the pane. */
export function planOfPane(rest: SnapshotRest | null | undefined, paneId: string): PlanView | null {
  return rest?.plans?.items.find((plan) => plan.pane_id === paneId) ?? null;
}

export function sameSession(a: WorkSession, b: WorkSession): boolean {
  return a.device_id === b.device_id && a.agent === b.agent && a.session_id === b.session_id;
}

/** The plan of `session` while a pane runs it. */
export function planOfSession(rest: SnapshotRest | null | undefined, session: WorkSession): PlanView | null {
  return rest?.plans?.items.find((plan) => sameSession(plan.session, session)) ?? null;
}

/** Whether the plan waits for the operator's review: put up with `--ask` and not finished (B4, B14). */
export function planProposed(plan: PlanView): boolean {
  return plan.review === "proposed" && !plan.progress.finished;
}

export function stepOf(plan: PlanView, key: string | null): PlanStep | null {
  return key == null ? null : (plan.steps.find((step) => step.key === key) ?? null);
}

/** `n/m`: done over done, working and planned; dropped steps count in neither (D-25). */
export function planCount(plan: PlanView): string {
  return `${plan.progress.done}/${plan.progress.total}`;
}

/** `지금: <step> (n/m)`, the one number a plan shows (D-08); null for a finished plan. */
export function planNowLine(plan: PlanView, t: TFunction<"translation">): string | null {
  const current = stepOf(plan, plan.progress.current);
  if (!current || plan.progress.finished) return null;
  return t("plan.now", { step: current.title, progress: planCount(plan) });
}

/** The pane head's second line while the plan runs: `<step> · <line>` (B7). */
export function planWorkLine(plan: PlanView): string | null {
  const current = stepOf(plan, plan.progress.current);
  if (!current || plan.progress.finished || planProposed(plan)) return null;
  return current.line ? `${current.title} · ${current.line}` : current.title;
}

/** A step's status mark: done, moving, or quiet for planned and dropped. */
export function stepMark(step: PlanStep): RowMark {
  if (step.status === "done") return "done";
  if (step.status === "working") return "going";
  return "quiet";
}

/** The steps by what they wait on, top down; a dropped step stays where it was (D-26). */
export function planGraph(plan: PlanView): LayeredGraph<PlanStep> {
  const byKey = new Map(plan.steps.map((step) => [step.key, step]));
  return layerDependencies(plan.steps, (step) => step.key, (step) => step.depends_on.flatMap((key) => byKey.get(key) ?? []));
}
