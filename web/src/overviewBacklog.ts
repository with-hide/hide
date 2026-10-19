// Mixed backlog projections and filters (overview-redesign B11-B13).
// The default backlogOf call retains the existing unstarted-open tally;
// the read view explicitly requests all loaded GitHub issues and local memos.

import type { BoardProject } from "./projectBoard";
import type { BacklogFailure, BacklogItem, IssueLabel, Task, TaskSource, Workspace } from "./snapshot";
import { isMemoKey } from "./overviewTasks";

/** A title's longest form (B29): the core refuses more. */
export const BACKLOG_TITLE_LIMIT = 256;

/** Rows a section shows before `나머지 이슈 N ›` (B27). */
export const BACKLOG_SHOWN = 50;

export type BacklogRow = {
  key: string;
  /** `#N` or `L-N`, when the source names one. */
  token: string | null;
  title: string;
  url: string | null;
  labels: readonly IssueLabel[];
  /** When it was made, else last changed; the order and the age. */
  at: number | null;
  local: boolean;
  author: string | null;
  assignees: readonly string[];
  open: boolean;
  started: boolean;
  updated: number | null;
};

export type ProjectBacklog = {
  source: TaskSource | null;
  /** Open items no checkout works on, newest first. */
  rows: BacklogRow[];
  /** Open items a checkout works on (`진행 중 N`). */
  started: number;
};

function rowOf(task: Task, started = false): BacklogRow {
  return {
    key: task.key,
    token: task.id,
    title: task.title,
    url: task.url,
    labels: task.labels ?? [],
    at: task.created_at_unix_ms ?? task.updated_at_unix_ms ?? null,
    local: isMemoKey(task.key),
    author: task.author ?? null,
    assignees: task.assignees ?? [],
    open: task.open,
    started,
    updated: task.updated_at_unix_ms ?? task.created_at_unix_ms ?? null,
  };
}

/**
 * Whether a failed undo offers 다시 시도: only a refusal is final, since it
 * sent nothing anywhere and would be refused again; Hide's record that could
 * not be read, and every GitHub failure, may clear.
 */
export function undoRetryable(failure: BacklogFailure | null): boolean {
  return failure !== null && failure !== "refused";
}

/** Whether a checkout of the project works on the item: the item is started (B28). */
export function startedKeys(workspace: Pick<Workspace, "checkouts">): Set<string> {
  return new Set(workspace.checkouts.flatMap((checkout) => (checkout.task_key ? [checkout.task_key] : [])));
}

export function backlogOf(workspace: Pick<Workspace, "checkouts" | "tasks">, all = false): ProjectBacklog {
  const started = startedKeys(workspace);
  const open = (workspace.tasks?.tasks ?? []).filter((task) => task.open);
  return {
    source: workspace.tasks?.source ?? null,
    rows: (all ? workspace.tasks?.tasks ?? [] : open.filter((task) => !started.has(task.key))).map((task) => rowOf(task, started.has(task.key))).sort((a, b) => (all ? b.updated ?? b.at ?? 0 : b.at ?? 0) - (all ? a.updated ?? a.at ?? 0 : a.at ?? 0)),
    started: open.filter((task) => started.has(task.key)).length,
  };
}

/** The 백로그 tab's count: every unstarted open item of the device (B3). */
export function backlogTally(projects: readonly BoardProject[]): { open: number } {
  return { open: projects.reduce((sum, { workspace }) => sum + backlogOf(workspace).rows.length, 0) };
}

/**
 * The rows the core is still making for a project, or failed to, newest
 * first: a made item stays until its source lists it, so the row never blinks
 * out between the create and the next read (B29).
 */
export function pendingItems(items: readonly BacklogItem[] | undefined, workspace: Pick<Workspace, "id" | "tasks">): BacklogItem[] {
  const listed = new Set((workspace.tasks?.tasks ?? []).map((task) => task.key));
  return (items ?? [])
    .filter((item) => item.workspace_id === workspace.id && (item.phase !== "created" || (item.task_key !== null && !listed.has(item.task_key))))
    .sort((a, b) => b.at_unix_ms - a.at_unix_ms);
}

/** The search box and the label chips (B27): words in the token or title, and any one picked label. */
export type BacklogFilters = { source: "all" | "github" | "local"; status: "open" | "started" | "closed" | "all"; author: string; assignee: string };
export const BACKLOG_FILTERS: BacklogFilters = { source: "all", status: "open", author: "", assignee: "" };

export function filterRows(rows: readonly BacklogRow[], query: string, labels: readonly string[], filters?: BacklogFilters): BacklogRow[] {
  const words = query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  return rows.filter((row) => {
    if (filters) {
      if (filters.source !== "all" && row.local !== (filters.source === "local")) return false;
      if (filters.status === "open" && (!row.open || row.started)) return false;
      if (filters.status === "started" && (!row.open || !row.started)) return false;
      if (filters.status === "closed" && row.open) return false;
      if (filters.author && row.author !== filters.author) return false;
      if (filters.assignee && !row.assignees.includes(filters.assignee)) return false;
    }
    if (labels.length && !row.labels.some((label) => labels.includes(label.name))) return false;
    return words.every((word) => {
      if (word.startsWith("label:")) return row.labels.some((label) => label.name.toLocaleLowerCase().includes(word.slice(6)));
      if (word.startsWith("author:")) return row.author?.toLocaleLowerCase().includes(word.slice(7)) ?? false;
      if (word.startsWith("assignee:")) return row.assignees.some((person) => person.toLocaleLowerCase().includes(word.slice(9)));
      if (word === "is:open") return row.open;
      if (word === "is:closed") return !row.open;
      if (word === "is:started") return row.started;
      return `${row.token ?? ""}\n${row.title}`.toLocaleLowerCase().includes(word);
    });
  });
}

/** Every label the device's backlog rows carry, once, by name. */
export function backlogLabels(projects: readonly BoardProject[]): IssueLabel[] {
  const labels = new Map<string, IssueLabel>();
  for (const { workspace } of projects) for (const row of backlogOf(workspace, true).rows) for (const label of row.labels) if (!labels.has(label.name)) labels.set(label.name, label);
  return [...labels.values()].sort((a, b) => a.name.localeCompare(b.name));
}

/** What the input keeps of typed or pasted text (B29, B42): one line, at most the limit in characters. */
export function backlogTitle(text: string): string {
  return Array.from(text.replace(/\s*[\r\n]+\s*/g, " ")).slice(0, BACKLOG_TITLE_LIMIT).join("");
}

/** The remaining characters the input shows near the limit (B29). */
export function titleLeft(title: string): number {
  return BACKLOG_TITLE_LIMIT - Array.from(title).length;
}

/** One intent key per add, so a retry of the same add converges on one item (D-34). */
export function backlogIntent(): string {
  const bytes = new Uint8Array(18);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (byte) => "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"[byte % 64]).join("");
}

/** Where an item goes, for the input's hint and the quick add's line (B26, B39). */
export type Destination = "public" | "private" | "unknown" | "local";

export function destinationOf(source: TaskSource | null): Destination {
  if (!source || source.kind !== "github") return "local";
  return source.public === true ? "public" : source.public === false ? "private" : "unknown";
}

export function githubSource(workspace: Pick<Workspace, "tasks"> | null): TaskSource | null {
  const tasks = workspace?.tasks;
  return tasks?.sources?.find((source) => source.kind === "github") ?? (tasks?.source?.kind === "github" ? tasks.source : null);
}

export type BacklogDestination = { kind: "local" } | { kind: "github"; repository: string };
