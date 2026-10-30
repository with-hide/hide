import type { TFunction } from "i18next";
import { deviceConnected } from "./devices";
import type { CellTone } from "./missions";
import { layerDependencies, orderByDependencies, type LayeredGraph } from "./projectBoard";
import type { PlanEvidence, PlanStep, PlanView, RowMark, SnapshotRest, WorkSession } from "./snapshot";

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

/** The steps as the checklist reads them (PRD mission-plan-visibility B3): by what they wait on, a step beside an earlier one at its depth indented. */
export function planChecklist(plan: PlanView): { step: PlanStep; parallel: boolean }[] {
  return orderByDependencies(plan.steps, (step) => step.key, (step) => step.depends_on).map(({ item, parallel }) => ({ step: item, parallel }));
}

/**
 * The chip's cells (B1): one per step still in the plan, in the checklist's
 * order, done green, the current step blue or, while the plan waits for its
 * review, in the warning colour; a dropped step has no cell.
 */
export function planCells(plan: PlanView): CellTone[] {
  const review = planProposed(plan);
  return planChecklist(plan).flatMap(({ step }): CellTone[] => {
    if (step.status === "dropped") return [];
    if (step.status === "done") return ["done"];
    if (step.key === plan.progress.current && !plan.progress.finished) return [review ? "hold" : "working"];
    return [step.status === "working" ? "working" : "waiting"];
  });
}

/**
 * The short word a handed step carries where its tail goes (PRD
 * hide-plan-delegate B25, B28): its child's report came, it sent a block, or
 * its pane closed for good; `pending` while the step is still to be handed
 * (B2, B3). A step the main does itself, or a child still at work, has none.
 */
export type StepMarkWord = "reported" | "blocked" | "closed" | "pending";

export function stepDelegationWord(step: PlanStep): StepMarkWord | null {
  // A finished step says what it left instead.
  if (step.status === "done" || step.status === "dropped") return null;
  const state = step.delegation?.state;
  if (state === "reported" || state === "blocked" || state === "closed") return state;
  if (!step.delegation && step.delegate && step.status === "planned") return "pending";
  return null;
}

/** What a finished step left, as its tail says it: `#n` or the file's name (B26). */
export function evidenceWords(evidence: PlanEvidence): string {
  if (evidence.kind === "pr") return `#${evidence.number}`;
  const parts = evidence.path.split(/[\\/]/);
  return parts[parts.length - 1] || evidence.path;
}

/**
 * The plan of the child a step is handed to, found by the child's session
 * (B20); null while the child has none or it is not read yet, and while the
 * child's device does not answer, since its step and count would be a value
 * nobody can vouch for (B32, design principle 10).
 */
export function childPlanOf(rest: SnapshotRest | null | undefined, step: PlanStep): PlanView | null {
  const session = step.delegation?.session;
  if (!session || !childReachable(rest, step)) return null;
  return planOfSession(rest, session);
}

/** Whether the device of the child a step is handed to answers now; a child with no session yet is not on another device. */
export function childReachable(rest: SnapshotRest | null | undefined, step: PlanStep): boolean {
  const session = step.delegation?.session;
  return !session || deviceConnected(rest ?? null, session.device_id);
}

/** The child's current step and its count, `<step> n/m`, from the child's own plan; null without one, or once it has no current step (B25, B32). */
export function childNow(child: PlanView | null): { step: string; progress: string } | null {
  if (!child || child.progress.finished) return null;
  const current = stepOf(child, child.progress.current);
  return current ? { step: current.title, progress: planCount(child) } : null;
}

/** The steps `step` still waits on, neither done nor dropped (B25's `<기다리는 단계들> 기다림`). */
export function waitingOn(plan: PlanView, step: PlanStep): PlanStep[] {
  return step.depends_on.flatMap((key) => {
    const other = stepOf(plan, key);
    return other && other.status !== "done" && other.status !== "dropped" ? [other] : [];
  });
}

/** The panes of the children a plan's steps are handed to: the pane head counts its other direct children as `맡긴 일 N` (B29). */
export function handedPanes(plan: PlanView | null): Set<string> {
  return new Set(plan?.steps.flatMap((step) => (step.delegation?.pane_id ? [step.delegation.pane_id] : [])) ?? []);
}
