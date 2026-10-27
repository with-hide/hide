// The Project tool's rules (PRD lane-two-columns D-14..D-19, B26-B39): which
// project it is about, how its 할 일 is cut, and how its cost reads. The core
// derives every count (`agent_state/tally/project_status.rs`,
// `usage_ledger/project_cost.rs`); these functions only shape what it sent,
// so they are testable without a page.

import type { ProjectStatus, ProjectTodo } from "./agentScope";
import type { NodeUsage, ProjectCost } from "./generated/hided-ws";
import type { InterfaceLanguage } from "./i18n/locale";
import { openMissedWork } from "./sessionWork";
import { catalogWorkspaces, frontCheckout, type LinkSummaries, type SnapshotRest, type Workspace } from "./snapshot";

/** The 할 일 rows shown before `+N 더` (B37). */
export const TODO_SHOWN = 5;


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
  /** Rows the core sent past the cut, `+N 더`; 0 when every sent row shows. */
  more: number;
  /** Rows past the core's cap that it did not send, by where they are: agents' rows in the sidebar's 태스크, pull requests in its PR list. */
  beyond: { agents: number; prs: number };
};

/**
 * The 할 일 the list draws (B34, B35, B37): the core's rows in their order,
 * the first five until `open`, the rest of the sent rows counted, and apart
 * from them the rows `todo_total` counts past the core's cap.
 */
export function todoView(status: ProjectStatus, open: boolean): TodoView {
  const shown = open ? status.todo : status.todo.slice(0, TODO_SHOWN);
  return {
    asks: shown.filter((row) => row.kind === "ask"),
    revisit: shown.filter((row) => row.kind === "revisit"),
    prs: shown.filter((row) => row.kind === "pr"),
    more: open ? 0 : status.todo.length - shown.length,
    beyond: {
      agents: Math.max(0, status.todo_agents - status.todo.filter((row) => row.kind !== "pr").length),
      prs: Math.max(0, status.todo_total - status.todo_agents - status.todo.filter((row) => row.kind === "pr").length),
    },
  };
}

/** The open missed work of a project (B28), none while Hide AI's summaries are off. */
export function missedCount(links: LinkSummaries | null, projectId: string, enabled: boolean): number {
  return enabled ? openMissedWork(links?.projects[projectId]?.work ?? []).length : 0;
}

/**
 * A project's cost as the core's usage frame carries it (B31, B33): `pending`
 * before the core's first build, `failed` when no build has succeeded, or
 * `ready` with the cost (null when no priced call ran there in seven days)
 * and whether the latest build failed, the last one kept.
 */
export type CostRead = { state: "pending" } | { state: "failed" } | { state: "ready"; cost: ProjectCost | null; failed: boolean };

export function projectCost(node: Pick<NodeUsage, "projects" | "projects_failed"> | undefined, projectId: string): CostRead {
  if (node?.projects === undefined) return node?.projects_failed ? { state: "failed" } : { state: "pending" };
  return { state: "ready", cost: node.projects.find((row) => row.workspace_id === projectId) ?? null, failed: node.projects_failed === true };
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
