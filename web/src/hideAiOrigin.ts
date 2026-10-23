// What the screen says about the rows and checkouts Hide AI started (PRD
// hide-ai-origin-mark D-01 to D-09). The core decides who started what, from
// the coordination record of the spawn; this module only words it and joins
// it to the checkouts a project draws. Nothing here reads a branch name.
import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";
import { catalogWorkspaces, type AgentRow, type Checkout, type HideAi, type HideAiCheckout, type HideAiTask, type HomeTurn, type SnapshotRest, type Workspace } from "./snapshot";

/** The task kinds the catalogs name; a kind of another build shows as its code (LOCALIZATION.md). */
const TASK_KEYS: Record<string, MessageKey> = {
  memory_promote: "hideAi.task.memory_promote",
};

/** `Memory 반영하기 · m72`: the work and what it is about; null when the core named no task (B10). */
export function taskLine(t: TFunction<"translation">, task: HideAiTask | undefined): string | null {
  if (!task) return null;
  const key = TASK_KEYS[task.kind];
  const kind = key ? t(key) : task.kind;
  return task.target ? `${kind} · ${task.target}` : kind;
}

/** What a started thing is called on the screen: its task line, or `Hide AI` alone while the core names no task (B10). */
export function originLine(t: TFunction<"translation">, hideAi: HideAi): string {
  return taskLine(t, hideAi.task) ?? t("hideAi.name");
}

/** What the home waits for, in words (`PR #n 리뷰`, `CI 실패`); the core decides the turn and the screen only words it (PRD hide-ai-work-home D-14). */
export function homeTurnLine(t: TFunction<"translation">, turn: HomeTurn): string {
  return turn.kind === "pr_review" ? t("hideAi.turn.pr_review", { number: turn.pr_number }) : t("hideAi.turn.ci_failed");
}

/** Hide AI's part of a row's accessible name (D-09, B12): the name, the task and the turn, so a screen reader hears who started it and what waits. */
export function hideAiWords(t: TFunction<"translation">, hideAi: HideAi | undefined): string | null {
  if (!hideAi) return null;
  const task = taskLine(t, hideAi.task);
  const turn = hideAi.turn ? homeTurnLine(t, hideAi.turn) : null;
  return [t("hideAi.name"), task, turn].filter(Boolean).join(" · ");
}

/** The place line of a row Hide AI started: the project and the task in the branch's place (`herdr-ide › Memory 반영하기 · m72`). */
export function hideAiPlace(t: TFunction<"translation">, project: string, hideAi: HideAi): string {
  return `${project} › ${originLine(t, hideAi)}`;
}

/** The task Hide AI started this checkout for, by the device and path the core lists; absent for an operator's checkout. */
export function hideAiCheckoutOf(listed: readonly HideAiCheckout[] | undefined, workspace: Pick<Workspace, "device_id">, checkout: Pick<Checkout, "path">): HideAiCheckout | null {
  return listed?.find((row) => row.device_id === workspace.device_id && row.path === checkout.path) ?? null;
}

/**
 * Whether a project's tree draws `checkout`. One Hide AI made is drawn only
 * while it is the front checkout or an operator-owned agent works in it, so
 * a promotion's worktree never crowds the tree (PRD hide-ai-work-home D-17,
 * B3); its agent row is never in a tree, the core leaves it out. `holdsAgent`
 * is whether the checkout's tree has any row, and every row in it is the
 * operator's.
 */
export function drawnInTree(listed: readonly HideAiCheckout[] | undefined, workspace: Pick<Workspace, "device_id">, checkout: Pick<Checkout, "path">, front: boolean, holdsAgent: boolean): boolean {
  return front || holdsAgent || !hideAiCheckoutOf(listed, workspace, checkout);
}

/** Where a Needs You row its home owns opens: the Memory tab of its Project, expanded on its attempt's row (B11); null when the core has not named them. */
export function homeOpening(rest: SnapshotRest | null, agent: Pick<AgentRow, "hide_ai"> | undefined): { deviceId: string; workspaceId: string; number: number } | null {
  const hideAi = agent?.hide_ai;
  const card = /^m(\d+)$/.exec(hideAi?.task?.target ?? "");
  if (hideAi?.home !== "memory" || !hideAi.workspace_id || !card) return null;
  const workspace = catalogWorkspaces(rest).find((row) => row.id === hideAi.workspace_id);
  return workspace ? { deviceId: workspace.device_id, workspaceId: workspace.id, number: Number(card[1]) } : null;
}
