// What a card on a desk is called (PRD lane-desk-pair-view D-13): the core
// sends a card's subject, not its name, so the shell names it from what it
// holds of that subject now, the same on its tab, its menus and the recent
// tabs list.

import { projectPullWords } from "./codeHost";
import type { TFunction } from "i18next";
import { summariesEnabled } from "./hideAi";
import { missionOf } from "./missions";
import { allAgents } from "./paneAgents";
import { recordTitle, workKey } from "./sessionWork";
import { catalogWorkspaces, type DetailItem, type WorkSession } from "./snapshot";
import type { useShellStore } from "./store";

type ShellState = ReturnType<typeof useShellStore.getState>;
type Translate = TFunction<"translation">;

/** What a record is called: its agent's name while the agent is listed, else the record's own title once read. */
function sessionTitle(s: ShellState, session: WorkSession): string | null {
  const listed = allAgents(s.agents, s.rest?.status?.remote).find((row) => row.session_id === session.session_id)?.identity_label;
  if (listed) return listed;
  const read = s.sessionRecord;
  return read?.session && read.record && workKey(read.session) === workKey(session) ? recordTitle(read.record, summariesEnabled(s.rest?.status?.background_ai)) : null;
}

/**
 * A card's name, read from the shell's state now. `bare` is its name on its
 * desk tab (PRD lane-desk-pair-view D-30): the tab's mark already says the
 * kind a mission or plan title opens with, and a desk is one checkout's, so a
 * pull request leaves out its project and a record is `기록` alone; the full
 * name is the tab's tooltip.
 */
export function cardTitle(s: ShellState, t: Translate, item: DetailItem | null, bare = false): string {
  if (!item) return "";
  if (item.kind === "pull_request") {
    const project = catalogWorkspaces(s.rest).find((row) => row.id === item.workspace_id);
    return bare ? t("detail.prTab", projectPullWords(project, item.number)) : t("detail.pr", { ...projectPullWords(project, item.number), project: project?.label ?? "" });
  }
  if (item.kind === "mission") {
    const title = missionOf(s.rest, item.mission)?.title ?? item.mission;
    return bare ? title : t("missionScreen.card", { title });
  }
  if (item.kind === "branch") {
    const row = s.suggestions?.project?.workspace_id === item.workspace_id ? s.suggestions.project.rows.find((candidate) => candidate.id === item.row) : undefined;
    const name = row?.attempt?.branch?.name ?? item.row;
    return item.path ? t("detail.branchFile", { name, path: item.path }) : t("detail.branch", { name });
  }
  if (item.kind === "automation_run") return t("automationScreen.run.summaryTab");
  if (item.kind === "worktrees" || item.kind === "cost") {
    const project = catalogWorkspaces(s.rest).find((row) => row.id === item.workspace_id)?.label ?? "";
    return t(item.kind === "worktrees" ? "projectTool.worktrees.title" : "projectTool.cost.title", { project });
  }
  if (item.kind === "remains") {
    const checkout = catalogWorkspaces(s.rest).find((row) => row.id === item.workspace_id)?.checkouts.find((row) => row.id === item.checkout_id);
    return t("projectTool.remains.title", { name: checkout?.branch ?? checkout?.label ?? "" });
  }
  const title = sessionTitle(s, item.session);
  if (bare && item.kind === "record") return t("detail.recordUntitled");
  if (item.kind === "plan") return bare ? (title ?? t("detail.planUntitled")) : title ? t("detail.plan", { title }) : t("detail.planUntitled");
  return title ? t("detail.record", { title }) : t("detail.recordUntitled");
}

/**
 * A card's path line (PRD lane-desk-pair-view D-30), where a file's path
 * stands over a file: what the card points at beside its tab's short name.
 * A pull request reads its number and title, a record or a plan its agent,
 * a mission its title; a card whose tab already names it whole reads the
 * place it covers instead: a project's or a worktree's folder, the week a
 * cost counts.
 */
export function cardPath(s: ShellState, t: Translate, item: DetailItem): string {
  const workspace = "workspace_id" in item ? catalogWorkspaces(s.rest).find((row) => row.id === item.workspace_id) : undefined;
  switch (item.kind) {
    case "pull_request": {
      const title = workspace?.pull_requests?.find((row) => row.number === item.number)?.title;
      return title ? t("detail.prPath", { ...projectPullWords(workspace, item.number), title }) : cardTitle(s, t, item);
    }
    case "record": {
      const title = sessionTitle(s, item.session);
      return title ? t("detail.recordPath", { title }) : t("detail.recordPathUntitled");
    }
    case "worktrees":
      return workspace?.path ?? cardTitle(s, t, item);
    case "cost":
      return t("detail.costPath", { project: workspace?.label ?? "" });
    case "remains":
      return workspace?.checkouts.find((row) => row.id === item.checkout_id)?.path ?? cardTitle(s, t, item);
    case "branch":
      return workspace ? [workspace.path, item.path].filter(Boolean).join(" › ") : cardTitle(s, t, item);
    default:
      return cardTitle(s, t, item);
  }
}
