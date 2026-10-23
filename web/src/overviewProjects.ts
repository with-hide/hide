// The Overview's Projects tab as data (PRD overview-device-tasks D-14, D-24..D-27,
// B47-B56): each project's tasks counted by state, its backlog, its disk as
// the sweep left it, what is left to clean, and which rows fold as quiet.

import type { TaskCardScope } from "./agentScope";
import { LOW_FREE_BYTES, reclaimable } from "./diskCleanup";
import { backlogOf } from "./overviewBacklog";
import { cleanupOf } from "./overviewLens";
import type { BoardProject } from "./projectBoard";
import type { ProjectDisk, Workspace } from "./snapshot";

export type NowCounts = { needs: number; working: number; waiting: number; done: number; resting: number; total: number };

export function nowCounts(cards: readonly Pick<TaskCardScope, "state">[]): NowCounts {
  const counts: NowCounts = { needs: 0, working: 0, waiting: 0, done: 0, resting: 0, total: cards.length };
  for (const card of cards) {
    if (card.state === "needs_you" || card.state === "blocked") counts.needs += 1;
    else if (card.state === "working") counts.working += 1;
    else if (card.state === "waiting") counts.waiting += 1;
    else if (card.state === "done") counts.done += 1;
    else counts.resting += 1;
  }
  return counts;
}

/** One count the 지금 cell names, in the cell's order. */
export type NowPart = { key: "needs" | "working" | "waiting" | "done"; value: number };

/** The counts the 지금 cell names (B48): Needs You, working, waiting and done, each only when it is not zero. */
export function nowParts(now: NowCounts): NowPart[] {
  const parts: NowPart[] = [
    { key: "needs", value: now.needs },
    { key: "working", value: now.working },
    { key: "waiting", value: now.waiting },
    { key: "done", value: now.done },
  ];
  return parts.filter((part) => part.value > 0);
}

/** A disk cell as the sweep left it (B53-B56): the last size, whether it is being measured or waits, and why it failed. */
export type DiskCellState = {
  /** The last size measured, kept while it is measured again or failed. */
  bytes: number | null;
  measuring: boolean;
  queued: boolean;
  failure: string | null;
};

export function diskCell(disk: ProjectDisk | undefined | null): DiskCellState {
  return {
    bytes: disk?.total_bytes ?? disk?.confirmed_bytes ?? null,
    measuring: disk?.measuring ?? false,
    queued: disk?.queued ?? false,
    failure: disk?.unavailable_reason ?? null,
  };
}

export type ProjectRowModel = {
  project: Workspace;
  folder: boolean;
  now: NowCounts;
  backlog: number;
  disk: DiskCellState;
  /** Bytes a cleanup would free (the purple share, B51). */
  reclaimable: number;
  /** Merged checkouts left on disk (`머지됨 N → 정리`, B49). */
  merged: number;
  /** Resting with nothing to clean (B50). */
  quiet: boolean;
};

export function projectRows(projects: readonly BoardProject[]): ProjectRowModel[] {
  return projects.map(({ workspace }) => {
    const now = nowCounts(workspace.agent_scope.tasks?.cards ?? []);
    const merged = workspace.checkouts.filter((checkout) => cleanupOf(workspace, checkout) === "merged").length;
    return {
      project: workspace,
      folder: workspace.is_git !== true,
      now,
      backlog: backlogOf(workspace).rows.length,
      disk: diskCell(workspace.disk),
      reclaimable: reclaimable(workspace),
      merged,
      quiet: now.total === now.resting && merged === 0,
    };
  });
}

/**
 * The rows in the tab's order (B50, B52): the active ones as the device lists
 * them and the quiet ones after; by size, every row largest first, a row
 * not measured yet last.
 */
export function orderedRows(rows: readonly ProjectRowModel[], bySize: boolean): { active: ProjectRowModel[]; quiet: ProjectRowModel[] } {
  if (bySize) return { active: [...rows].sort((a, b) => (b.disk.bytes ?? -1) - (a.disk.bytes ?? -1)), quiet: [] };
  return { active: rows.filter((row) => !row.quiet), quiet: rows.filter((row) => row.quiet) };
}

/** The tab's top line (B47): the device's free and total space, the projects' sum and what a cleanup frees. */
export type DiskSummary = { free: number | null; total: number | null; sum: number | null; reclaimable: number; low: boolean; measuring: boolean };

export function diskSummary(rows: readonly ProjectRowModel[]): DiskSummary {
  let free: number | null = null;
  let total: number | null = null;
  let sum: number | null = null;
  let freed = 0;
  let measuring = false;
  for (const row of rows) {
    const disk = row.project.disk;
    if (disk?.free_bytes != null) free = free === null ? disk.free_bytes : Math.min(free, disk.free_bytes);
    if (disk?.volume_total_bytes != null) total = Math.max(total ?? 0, disk.volume_total_bytes);
    if (row.disk.bytes !== null) sum = (sum ?? 0) + row.disk.bytes;
    freed += row.reclaimable;
    measuring ||= row.disk.measuring || row.disk.queued;
  }
  return { free, total, sum, reclaimable: freed, low: free !== null && free < LOW_FREE_BYTES, measuring };
}
