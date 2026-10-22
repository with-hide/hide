// The Project tool's rules (PRD lane-two-columns D-14..D-19, B26-B39): which
// project it is about, how its 할 일 is cut, and how its cost reads. The core
// derives every count (`agent_state/tally/project_status.rs`,
// `usage_ledger/project_cost.rs`); these functions only shape what it sent,
// so they are testable without a page.

import type { ProjectStatus, ProjectTodo } from "./agentScope";
import type { ProjectCost } from "./generated/hided-ws";
import type { InterfaceLanguage } from "./i18n/locale";
import { openMissedWork } from "./sessionWork";
import { catalogWorkspaces, frontCheckout, type LinkSummaries, type SnapshotRest, type Workspace } from "./snapshot";

/** The 할 일 rows shown before `+N 더` (B37). */
export const TODO_SHOWN = 5;

/** The recent cards 현황 names (B39). */
export const RECENT_CARDS = 5;

/** The Project of the checkout in front, or null at Home or in a device's Home folder. */
export function frontProject(rest: SnapshotRest | null): Workspace | null {
  const front = frontCheckout(rest);
  if (!front) return null;
  const project = catalogWorkspaces(rest).find((row) => row.id === front.workspace_id && row.checkouts.some((checkout) => checkout.id === front.id));
  return project && !project.is_home ? project : null;
}

export type TodoView = {
  /** New asks, in the core's order. */
  asks: ProjectTodo[];
  /** 다시 볼 것, under the asks in the same group. */
  revisit: ProjectTodo[];
  /** The operator's pull requests and reviews. */
  prs: ProjectTodo[];
  /** Rows past the cut, `+N 더`; 0 when every row shows. */
  more: number;
};

/**
 * The 할 일 the list draws (B34, B35, B37): the core's rows in their order,
 * the first five until `open`, the rest counted from `todo_total`, which
 * also counts rows the core left off past its cap.
 */
export function todoView(status: ProjectStatus, open: boolean): TodoView {
  const shown = open ? status.todo : status.todo.slice(0, TODO_SHOWN);
  return {
    asks: shown.filter((row) => row.kind === "ask"),
    revisit: shown.filter((row) => row.kind === "revisit"),
    prs: shown.filter((row) => row.kind === "pr"),
    more: open ? 0 : Math.max(0, status.todo_total - shown.length),
  };
}

/** The open missed work of a project (B28), none while Hide AI's summaries are off. */
export function missedCount(links: LinkSummaries | null, projectId: string, enabled: boolean): number {
  return enabled ? openMissedWork(links?.projects[projectId]?.work ?? []).length : 0;
}

/** A project's cost as the core's usage frame carries it, or null when no priced call ran there in seven days. */
export function projectCost(projects: readonly ProjectCost[] | undefined, projectId: string): ProjectCost | null {
  return projects?.find((row) => row.workspace_id === projectId) ?? null;
}

/** Millionths of a dollar as money, to the cent. */
export function usd(micro: number, language: InterfaceLanguage): string {
  return new Intl.NumberFormat(language, { style: "currency", currency: "USD", currencyDisplay: "narrowSymbol", maximumFractionDigits: 2 }).format(micro / 1_000_000);
}

/** An amount the core said is a lower bound reads `≥`; an exact conversion reads `≈` (`UsageCost`). */
export function costPrefix(cost: Pick<ProjectCost, "partial" | "waiting" | "limited">): "≥" | "≈" {
  return cost.partial || cost.waiting || cost.limited ? "≥" : "≈";
}

/** Each priced agent's share of the total, largest first (B31). */
export function agentShares(cost: Pick<ProjectCost, "agents" | "micro_usd">): { agent: string; share: number }[] {
  if (cost.micro_usd === 0) return [];
  return cost.agents.map((row) => ({ agent: row.agent, share: row.micro_usd / cost.micro_usd }));
}

/** A bar's height in the day chart: its share of the largest day, 0 for none. */
export function dayHeights(days: readonly { micro_usd: number }[]): number[] {
  const largest = Math.max(0, ...days.map((day) => day.micro_usd));
  return days.map((day) => (largest === 0 ? 0 : day.micro_usd / largest));
}
