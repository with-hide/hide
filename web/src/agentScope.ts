import type { AgentRow, DescendantCounts, MarkCounts, RequestVerb, SnapshotRest } from "./snapshot";

/** Core-owned membership, status counts and display order for one scope. */
export type AgentRef = { pane_id: string; occurrence: number };


/** One row of a project's 할 일 (B34, B35): an agent by its pane, or a pull request by its number. */
/** `record`: an open ask the session's record holds, which no 태스크 card shows (PRD lane-two-columns B34). */
export type ProjectTodo = { kind: "ask" | "revisit"; pane_id: string; record?: true } | { kind: "pr"; number: number };

/** A project's 현황 tiles' counts and its 할 일 as the core derived them (`agent_state/tally/project_status.rs`). */
export type ProjectStatus = {
  todo: ProjectTodo[];
  /** Every 할 일 row, the ones the core left off past its cap included: the rail's number (B38). */
  todo_total: number;
  /** How many of `todo_total` are agents' rows, the rest pull requests: where the rows past the core's cap are (B37). */
  todo_agents: number;
  prs: { open: number; mergeable: number; failed: number; review: number; checking: number };
  git: {
    worktrees: number;
    behind?: number;
    changed: number;
    unpushed: number;
    removable: number;
    dots: { checkout_id: string; state: "changed" | "unpushed" | "removable" | "clean" }[];
  };
};

export type AgentScope = {
  overview_needs_you: number;
  has_working: boolean;
  relations: Record<string, { issues: string[]; project_id: string; checkout_id: string; rows: { pane_id: string; occurrence: number; depth: number; tag: "here" | "parent" | null; caption_parent: string | null }[] }[]>;
  listed: { pane_id: string; device_id: string; device_label: string | null; remote: boolean; index: number }[];
  places: Record<string, { project_id: string; checkout_id: string; kind: "home" | "folder" | "checkout" }>;
  places_live: boolean;
  /** A project's or, on a device's scope, every project's tasks (`agent_scope.tasks`, PRD overview-device-tasks D-05, sidebar-task-lens D-40); absent while there are none. */
  tasks?: ProjectTasksScope;
  closes: Record<string, CloseScope>;
  raised: { group: "needs_you" | "done"; shown: AgentRef[]; more: AgentRef[] }[];
  owners: Record<string, string>;
  badge_total: number;
  prs: PrBoard;
  /** A project's 현황 (PRD lane-two-columns D-15, D-16), derived by the core with its tasks and PRs; absent from a checkout's or a device's scope. */
  status?: ProjectStatus;
  /** A Git checkout's own mark, the rule its project's dots use, uncapped (PRD lane-two-columns B30); absent elsewhere and while its worktree is unread. */
  git_mark?: ProjectStatus["git"]["dots"][number]["state"];
  rows: AgentRef[];
  total: number;
  overview_total: number;
  roots: AgentRef[];
  groups: { needs_you: number; done: number; working: number; seen: number };
  marks: MarkCounts;
  group_rows: { group: string; rows: AgentRef[] }[];
  descendants: Record<string, number>;
  children: Record<string, string[]>;
  sections: { group: string; rows: (AgentRef & { depth: number; descendants: number })[]; count: number }[];
  members: { pane_id: string; project_id: string; checkout_id: string }[];
  buckets: { turn: number; working: number; delegating: number; resting: number };
  turns: { question: number; approval: number; error: number; done: number };
  folded: Record<string, {
    tiers: { key: string; candidates: string[]; branch: string | null; pull_request: number | null; device_id: string | null }[][];
    overflow: number;
    badge_descendants: number;
    badge_counts: DescendantCounts;
    badge_children: string[];
  }>;
  tree: AgentTreeScope;
  sidebar_tree: AgentTreeScope;
  global_tree: AgentTreeScope;
  requests: {
    rows: { member: number; children: string[] }[];
    groups: { verb: RequestVerb; rows: number[] }[];
    counts: Record<RequestVerb, number>;
    todo: number;
    answer: number;
  };
};

/** No snapshot yet means no known scope; a received scope is never rebuilt here. */
export function deviceScope(rest: SnapshotRest | null, id: string): AgentScope | null {
  return rest?.navigator?.devices?.find((device) => device.id === id)?.agent_scope ?? null;
}

export function scopeRows(ids: readonly string[], agents: readonly AgentRow[]): AgentRow[] {
  const byPane = new Map(agents.map((agent) => [agent.pane_id, agent]));
  return ids.map((id) => {
    const row = byPane.get(id);
    if (!row) throw new Error(`Agent scope references a missing row: ${id}`);
    return row;
  });
}

/** Resolve physical occurrences without changing their core-selected membership. */
export function scopeOccurrences(refs: readonly AgentRef[], agents: readonly AgentRow[]): AgentRow[] {
  const byPane = new Map<string, AgentRow[]>();
  for (const row of agents) { const found = byPane.get(row.pane_id) ?? []; found.push(row); byPane.set(row.pane_id, found); }
  return refs.map((ref) => {
    const row = byPane.get(ref.pane_id)?.[ref.occurrence];
    if (!row) throw new Error(`Missing agent occurrence: ${ref.pane_id}/${ref.occurrence}`);
    return row;
  });
}

export type AgentTreeScope = {
  rows: (AgentRef & { depth: number })[];
  visible_rows: (AgentRef & { depth: number })[];
  shown: string[];
  more: number;
  needs_you: boolean;
  turn_kind: "question" | "review" | null;
};

export type CloseScope = {
  decision: { action: "status_unknown"; label: string } | { action: "confirm" } | { action: "close" };
  stop_work: { rows: { pane_id: string; agent: boolean; label: string; state: "active" | "unknown" | "quiet" }[]; unknown: number | null };
  subtree: CloseSubtree | null;
  subtree_all: CloseSubtree | null;
};
export type CloseSubtree = {
  ids: string[];
  rows: { pane_id: string; depth: number; target: boolean; state: "working" | "waiting" | "unread" | "unknown" | "quiet" }[];
  counts: { working: number; waiting: number; unread: number; unknown: number };
  unknown: boolean;
  target_unknown: boolean;
};

/** Where a task stands, in the order its project's section draws it (`TaskState`, B7). */
export type TaskState = "needs_you" | "blocked" | "working" | "waiting" | "done" | "revisit" | "resting" | "cleanup";

/** One task: a lineage root and every descendant on any device (`tasks::Card`, B8-B10). */
export type TaskCardScope = {
  root: string;
  state: TaskState;
  /** `작업 → PR → 검토 → 머지` filled up to here; absent for a conversation with no worktree and no PR. */
  stage?: 1 | 2 | 3 | 4;
  /** Every pane of the task, the root first. */
  members: string[];
  /** This project's checkouts the task works in, the root's first. */
  checkouts: string[];
  out_devices?: { device_id: string; label: string | null; project_id: string; pane_ids: string[] }[];
  out_projects?: { project_id: string; label: string }[];
  recency: string;
};

/** Work a root on another device delegated into this project, drawn dashed and never counted (B14). */
export type IncomingTaskScope = {
  root: string;
  root_device_id: string;
  root_device_label: string | null;
  root_project_id: string;
  pane_ids: string[];
};

export type ProjectTasksScope = {
  cards: TaskCardScope[];
  incoming?: IncomingTaskScope[];
  needs_you: number;
  recency: string;
};

/** Why a pull request stands where it does, as a kind the web words (`Reason`, PRD work-tab-pr-lens D-16). */
export type PrReasonKind = "merge" | "update" | "assign_reviewer" | "review" | "team_review" | "reply" | "check" | "working" | "conflict" | "failed" | "changes" | "ci" | "waiting_review" | "draft" | "queued" | "merged" | "checking" | "unjudged";

export type PrReason = {
  kind: PrReasonKind;
  /** A person, team, agent or check the reason is about. */
  name?: string;
  /** A question's first line; for `unjudged`, the question that went unanswered (`ExtraQuery`). */
  text?: string;
  count?: number;
  /** The instant the reason began; durations are drawn from it. */
  since_unix_ms?: number;
  /** The base branch a conflict or update names. */
  base?: string;
};

export type PrGroupId = "needs_you" | "working" | "blocked" | "waiting" | "drafts" | "bots" | "merged";

/** The PRs basis of one scope, decided by the core (`Board`). */
export type PrBoard = {
  rows: {
    number: number;
    group: PrGroupId;
    /** `team` or `old`: drawn as a folded line inside the group, never counted. */
    fold: "team" | "old" | null;
    reason: PrReason;
    /** The pull request's live agents, most urgent first. */
    agents: AgentRef[];
    agent_pane: string | null;
    checkout_id: string | null;
    checkout_ids: string[];
    issue: { key: string; label: string; url: string | null; task_key: string | null } | null;
  }[];
  groups: { group: PrGroupId; numbers: number[] }[];
  /** Needs You rows that are not folded. */
  needs_you: number;
};
