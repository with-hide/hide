// What the screen says about the rows and checkouts Hide AI started (PRD
// hide-ai-origin-mark D-01 to D-09). The core decides who started what, from
// the coordination record of the spawn; this module only words it and joins
// it to the checkouts a project draws. Nothing here reads a branch name.
import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";
import { catalogWorkspaces, type AgentRow, type AutomationsSection, type Checkout, type HideAi, type HideAiCheckout, type HideAiTask, type HomeTurn, type SnapshotRest, type Workspace } from "./snapshot";

/** The task kinds the catalogs name; a kind of another build shows as its code (LOCALIZATION.md). */
const TASK_KEYS: Record<string, MessageKey> = {
  memory_promote: "hideAi.task.memory_promote",
  suggestion_apply: "hideAi.task.suggestion_apply",
};

/** Each automation's name by its id, which an automation run's task names (`task.target`). */
export type AutomationNames = ReadonlyMap<string, string>;

const NO_NAMES: AutomationNames = new Map();
const namesOf = new WeakMap<AutomationsSection, AutomationNames>();

/** The names of this core's automations, one map per section, so a store selector that returns it is stable. */
export function automationNames(section: AutomationsSection | null): AutomationNames {
  if (!section) return NO_NAMES;
  let names = namesOf.get(section);
  if (!names) {
    names = new Map(section.items.map((automation) => [automation.id, automation.name]));
    namesOf.set(section, names);
  }
  return names;
}

/**
 * `Memory 반영하기 · m72`: the work and what it is about; an automation run's
 * `↻ 자동화: <이름>` (PRD automations B15), `↻ 자동화` while its name is not
 * known; null when the core named no task (B10).
 */
export function taskLine(t: TFunction<"translation">, task: HideAiTask | undefined, names: AutomationNames = NO_NAMES): string | null {
  if (!task) return null;
  if (task.kind === "automation") {
    const name = task.target ? names.get(task.target) : undefined;
    return name ? t("hideAi.task.automation", { name }) : t("hideAi.task.automationUnnamed");
  }
  const key = TASK_KEYS[task.kind];
  const kind = key ? t(key) : task.kind;
  return task.target ? `${kind} · ${task.target}` : kind;
}

/** What a started thing is called on the screen: its task line, or `Hide AI` alone while the core names no task (B10). */
export function originLine(t: TFunction<"translation">, hideAi: HideAi, names: AutomationNames = NO_NAMES): string {
  return taskLine(t, hideAi.task, names) ?? t("hideAi.name");
}

/** What the home waits for, in words (`PR #n 리뷰`, `CI 실패`, `바뀐 것 보기`); the core decides the turn and the screen only words it (PRD hide-ai-work-home D-14). */
export function homeTurnLine(t: TFunction<"translation">, turn: HomeTurn): string {
  switch (turn.kind) {
    case "pr_review": return t("hideAi.turn.pr_review", { number: turn.pr_number });
    case "ci_failed": return t("hideAi.turn.ci_failed");
    case "branch_ready": return t("hideAi.turn.branch_ready");
  }
}

/** Hide AI's part of a row's accessible name (D-09, B12): the name, the task and the turn, so a screen reader hears who started it and what waits. */
export function hideAiWords(t: TFunction<"translation">, hideAi: HideAi | undefined, names: AutomationNames = NO_NAMES): string | null {
  if (!hideAi) return null;
  const task = taskLine(t, hideAi.task, names);
  const turn = hideAi.turn ? homeTurnLine(t, hideAi.turn) : null;
  return [t("hideAi.name"), task, turn].filter(Boolean).join(" · ");
}

/** The place line of a row Hide AI started: the project and the task in the branch's place (`herdr-ide › Memory 반영하기 · m72`). */
export function hideAiPlace(t: TFunction<"translation">, project: string, hideAi: HideAi, names: AutomationNames = NO_NAMES): string {
  return `${project} › ${originLine(t, hideAi, names)}`;
}

/**
 * The run an automation run's row opens in the automation screen (PRD
 * automations B15): its automation and the run whose terminal is its pane;
 * null for any other row, or a run the section no longer lists.
 */
export function automationOpening(section: AutomationsSection | null, agent: Pick<AgentRow, "hide_ai" | "pane_id"> | undefined): { id: string; run: number | null } | null {
  const target = agent?.hide_ai?.task?.kind === "automation" ? agent.hide_ai.task.target : undefined;
  const automation = target ? section?.items.find((row) => row.id === target) : undefined;
  if (!automation || !agent) return null;
  const run = automation.records.find((record) => record.kind === "run" && record.pane_id === agent.pane_id);
  return { id: automation.id, run: run?.kind === "run" ? run.number : null };
}

/** The row the core lists for this checkout, Hide AI's or a tool's that keeps its work, by device and path; absent for an operator's checkout. */
function listedCheckout(listed: readonly HideAiCheckout[] | undefined, workspace: Pick<Workspace, "device_id">, checkout: Pick<Checkout, "path">): HideAiCheckout | null {
  return listed?.find((row) => row.device_id === workspace.device_id && row.path === checkout.path) ?? null;
}

/** The task Hide AI started this checkout for; absent for an operator's checkout and for an external tool's, which is no Hide AI work. */
export function hideAiCheckoutOf(listed: readonly HideAiCheckout[] | undefined, workspace: Pick<Workspace, "device_id">, checkout: Pick<Checkout, "path">): HideAiCheckout | null {
  const row = listedCheckout(listed, workspace, checkout);
  return row && !row.tool ? row : null;
}

/**
 * Whether a project's tree draws `checkout`. One Hide AI made, or one made
 * for an external tool's agent while the tool keeps its work, is drawn only
 * while it is the front checkout or an operator-owned agent works in it, so
 * neither crowds the tree (PRD hide-ai-work-home D-17, B3; PRD
 * hide-external-control B26); its agent row is never in a tree, the core
 * leaves it out. `holdsAgent` is whether the checkout's tree has any row,
 * and every row in it is the operator's.
 */
export function drawnInTree(listed: readonly HideAiCheckout[] | undefined, workspace: Pick<Workspace, "device_id">, checkout: Pick<Checkout, "path">, front: boolean, holdsAgent: boolean): boolean {
  return front || holdsAgent || !listedCheckout(listed, workspace, checkout);
}

/**
 * Where a Needs You row its home owns opens: the Hide AI tool of its Project
 * on 제안, with its attempt's row open (PRD hide-ai-suggestions B11; a Memory
 * promotion's row is named by its card, `m72`, B2); null when the core has
 * not named them.
 */
export function homeOpening(rest: SnapshotRest | null, agent: Pick<AgentRow, "hide_ai"> | undefined): { deviceId: string; workspaceId: string; row: string } | null {
  const hideAi = agent?.hide_ai;
  const target = hideAi?.task?.target ?? "";
  if (!hideAi?.home || !hideAi.workspace_id || !/^[ms]\d+$/.test(target)) return null;
  const workspace = catalogWorkspaces(rest).find((row) => row.id === hideAi.workspace_id);
  return workspace ? { deviceId: workspace.device_id, workspaceId: workspace.id, row: target } : null;
}
