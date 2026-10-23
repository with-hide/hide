// Chapter 6's world: one invented project with an agent at work beside the
// right lane. It answers the lane opening and closing and showing a tool, as
// the core's `workspace_view` does, and the Explorer's folds, and sends what
// the lane's tools read: the project's 현황 and 할 일 with two pull requests
// waiting on the operator, the checkout's files and the ones the agent
// changed, the project's cost over the week, and its Memory's cards.

import type { ProjectStatus } from "../../agentScope";
import type { NodeUsage } from "../../generated/hided-ws";
import { TOOLS, type Tool } from "../../laneTools";
import type { ChangesSnapshot, MemoryRow, MemorySection, PullRequest, Workspace } from "../../snapshot";
import type { DirectoryList } from "../../store";
import type { CoreEvent, PracticeWorld, WorldSnapshot } from "../world";
import { agent, applyDesk, deskSnapshot, oneTabDesk, shellScreen, type DeskModel, type PracticePane, type PracticePlace } from "./kit";

const PLACE: PracticePlace = { project: "hide-practice", label: "hide-practice", branch: "main" };
const ROOT = `/work/${PLACE.project}`;
const HOUR_MS = 3_600_000;
const DAY_MS = 24 * HOUR_MS;

/** What the world can change: the desk, whether the lane is open and which tool its column shows, and the Explorer's open folders. */
export type ProjectModel = { desk: DeskModel; open: boolean; tool: Tool | null; expanded: string[] };

/** Whether the lane draws `tool` now. */
export const showsTool = (model: ProjectModel, tool: Tool) => model.open && model.tool === tool;
/** Whether the lane is drawn at all: open with a column to show; the documents are none here. */
export const laneDrawn = (model: ProjectModel) => model.open && model.tool !== null;

function initial(): ProjectModel {
  const lead = agent({ pane_id: "lead", identity_label: "Fix the checkout 500", agent_kind: "claude", group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "4m" });
  // The lane comes back on the Project tool, the shape it was left in.
  return { desk: oneTabDesk({ id: "lead", agent: lead, title: "Fix the checkout 500" }), open: false, tool: "project", expanded: [`${ROOT}/src`, `${ROOT}/src/payment`] };
}

// --- The checkout's files --------------------------------------------------------

/** Every folder of the checkout and what it holds, folders first; a name ending in `/` is a folder. */
const TREE: Record<string, string[]> = {
  "": ["docs/", "src/", "package.json", "README.md"],
  docs: ["payments.md"],
  src: ["auth/", "payment/", "app.ts"],
  "src/auth": ["session.ts"],
  "src/payment": ["checkout.test.ts", "checkout.ts", "retry.ts"],
};

function listings(): Record<string, DirectoryList> {
  return Object.fromEntries(Object.entries(TREE).map(([folder, names]) => {
    const path = folder ? `${ROOT}/${folder}` : ROOT;
    const entries = names.map((name) => ({ name: name.replace(/\/$/, ""), path: `${path}/${name.replace(/\/$/, "")}`, is_directory: name.endsWith("/") }));
    return [path, { kind: "file_list", root_path: path, entries, truncated: false }];
  }));
}

/** What the agent changed so far, uncommitted. */
const CHANGED: { file: string; status: "modified" | "untracked"; added: number; removed: number }[] = [
  { file: "src/payment/checkout.ts", status: "modified", added: 14, removed: 5 },
  { file: "src/payment/checkout.test.ts", status: "modified", added: 22, removed: 0 },
  { file: "src/payment/retry.ts", status: "untracked", added: 31, removed: 0 },
];

function changes(): ChangesSnapshot {
  return {
    root_path: ROOT,
    entries: CHANGED.map(({ file, status, added, removed }) => ({ path: `${ROOT}/${file}`, relative_path: file, previous_relative_path: null, status, added_lines: added, removed_lines: removed })),
    committed: [], base_branch: "main", selected_path: null, selected_committed: false, diff: null, unavailable_reason: null,
  };
}

// --- Pull requests ---------------------------------------------------------------

/** The two pull requests waiting on the operator; every practice pull request is an example, and its address goes nowhere. */
const REVIEW = 7;
const MERGE = 8;
const prUrl = (number: number) => `https://example.invalid/${PLACE.project}/pull/${number}`;

function pullRequests(nowMs: number): PullRequest[] {
  const checks = { total: 3, passed: 3, failed: 0, running: 0, waiting: 0, skipped: 0, unknown: 0 };
  return [
    {
      number: REVIEW, title: "Add a retry to the payment call", url: prUrl(REVIEW), badge: "review", review: "review_required", is_draft: false, checks: "passing", state: "pending",
      created_at_unix_ms: nowMs - 5 * HOUR_MS, updated_at_unix_ms: nowMs - 2 * HOUR_MS, head_branch: "payment-retry", base_branch: "main", closing_issues: [],
      work: { author: "teammate", author_is_bot: false, mine: false, merge_state: "blocked", review_requests: [{ kind: "user", name: "you" }], asked_of_me: "direct", failed_checks: [], pending_checks: 0, check_counts: checks, queue: { state: "not_queued" }, review_requested_at_unix_ms: nowMs - 2 * HOUR_MS },
    },
    {
      number: MERGE, title: "Sort the export by date", url: prUrl(MERGE), badge: "open", review: "approved", is_draft: false, checks: "passing", state: "mergeable",
      created_at_unix_ms: nowMs - DAY_MS, updated_at_unix_ms: nowMs - HOUR_MS, head_branch: "export-sort", base_branch: "main", closing_issues: [],
      work: { author: "you", author_is_bot: false, mine: true, merge_state: "clean", review_requests: [], asked_of_me: "none", failed_checks: [], pending_checks: 0, check_counts: checks, queue: { state: "not_queued" }, review_requested_at_unix_ms: null },
    },
  ];
}

/** The core's board for them: both in Needs You, one to review and one to merge. */
function board(nowMs: number): Workspace["agent_scope"]["prs"] {
  const row = (number: number, kind: "review" | "merge", since: number) => ({
    number, group: "needs_you" as const, fold: null, reason: { kind, since_unix_ms: since }, agents: [], agent_pane: null, checkout_id: null, checkout_ids: [], issue: null,
  });
  return { rows: [row(REVIEW, "review", nowMs - 2 * HOUR_MS), row(MERGE, "merge", nowMs - HOUR_MS)], groups: [{ group: "needs_you", numbers: [REVIEW, MERGE] }], needs_you: 2 };
}

/** The 현황 and 할 일 as the core derives them from the board and the checkout (`tally/project_status.rs`). */
function status(checkoutId: string): ProjectStatus {
  return {
    todo: [{ kind: "pr", number: REVIEW }, { kind: "pr", number: MERGE }], todo_total: 2, todo_agents: 0,
    // One pending with checks passed (a review to come) and one mergeable.
    prs: { open: 2, mergeable: 1, failed: 0, review: 1, checking: 0 },
    // The main checkout holds the agent's uncommitted changes.
    git: { worktrees: 1, changed: 1, unpushed: 0, removable: 0, dots: [{ checkout_id: checkoutId, state: "changed" }] },
  };
}

/** The project's priced calls over the last seven days, in millionths of a dollar. */
function usage(nowMs: number): NodeUsage {
  const spent = [0.42, 1.1, 0, 0.86, 1.64, 0.35, 0.95].map((usd) => Math.round(usd * 1_000_000));
  const days = spent.map((micro_usd, index) => ({ date: new Date(nowMs - (6 - index) * DAY_MS).toISOString().slice(0, 10), micro_usd }));
  const total = spent.reduce((sum, micro) => sum + micro, 0);
  const codex = Math.round(total * 0.3);
  return {
    node: "local", agents: [], history: null,
    projects: [{
      workspace_id: PLACE.project, micro_usd: total, days, models: [], sessions: [],
      agents: [{ agent: "claude", micro_usd: total - codex }, { agent: "codex", micro_usd: codex }],
      unpriced_agents: [], partial: false, waiting: false, stale: false, limited: false,
    }],
  };
}

// --- Memory ----------------------------------------------------------------------

const CARDS: [kind: MemoryRow["kind"], title: string, daysAgo: number][] = [
  ["decision", "Sessions expire by the server's clock, not the browser's", 1],
  ["rule", "Changes reach main only through a pull request", 6],
  ["lesson", "The payment provider times out after 3 seconds; retry once", 3],
];

function memory(nowMs: number): MemorySection {
  return {
    project: {
      workspace_id: PLACE.project, unavailable: null, loading: false, enabled: true, capacity_reached: false, evicted_cards: 0, evicted_through_unix_ms: null,
      rows: CARDS.map(([kind, title, daysAgo], index) => ({
        number: index + 1, title, kind, human_source_count: 1, provided_session_count: 4 - index, pull_count: index, updated_at_unix_ms: nowMs - daysAgo * DAY_MS, verifying: false, promotion: null,
      })),
      questions: [], analysis: "idle", consent_required: false, usable_cards: CARDS.length, checking_card: null, check_resume_at_unix_ms: null,
      forgotten: null, pending_action: null, expanded: null, search: null,
    },
  };
}

// --- The snapshot ----------------------------------------------------------------

function snapshot(model: ProjectModel, nowMs: number): WorldSnapshot {
  const base = deskSnapshot([{ place: PLACE, model: model.desk }], PLACE.project, nowMs);
  const navigator = base.rest.navigator!;
  const pulls = pullRequests(nowMs);
  const prs = board(nowMs);
  const workspaces = navigator.workspaces!.map((project) => {
    const checkouts = project.checkouts.map((checkout) => ({ ...checkout, worktree: checkout.worktree && { ...checkout.worktree, dirty: true, changed_file_count: CHANGED.length } }));
    return { ...project, checkouts, pull_requests: pulls, agent_scope: { ...project.agent_scope, prs, status: status(checkouts[0]!.id) } };
  });
  return {
    ...base,
    rest: {
      ...base.rest,
      navigator: { ...navigator, workspaces, changes_root_path: ROOT },
      ui_state: { ...base.rest.ui_state!, expanded_paths: model.expanded },
      // Pinned, the lane docks beside the agent when it is open, as the operator's would.
      workspace_view: { ...base.rest.workspace_view!, open: model.open, tool: model.tool, pinned: true },
      // Hide AI on, so the Memory tool reads as it does for an operator who keeps one.
      status: { ...base.rest.status, background_ai: { enabled: true, provider: "claude", chosen: true, agent_summary: true, providers: [], unavailable_reason: null } },
    },
    usage: usage(nowMs),
    frames: { listings: listings(), changes: changes(), memory: memory(nowMs) },
  };
}

// --- The events ------------------------------------------------------------------

/**
 * The lane after a `workspace_view` patch, as the core applies it: a tool
 * shown opens the lane, a reveal shows the Explorer, and a lane opened with
 * no column comes back on the Explorer. Widths and the pin are the core's
 * alone here and change nothing on the screen.
 */
function applyView(model: ProjectModel, p: Record<string, unknown>): ProjectModel | null {
  let { open, tool } = model;
  if (typeof p.tool === "string" && (TOOLS as readonly string[]).includes(p.tool)) [tool, open] = [p.tool as Tool, true];
  if (p.tool === null) tool = null;
  if (typeof p.reveal === "string") [tool, open] = ["explorer", true];
  if (p.open === false) open = false;
  if (p.open === true) {
    open = true;
    tool ??= "explorer";
  }
  return open === model.open && tool === model.tool ? null : { ...model, open, tool };
}

function apply(model: ProjectModel, event: CoreEvent): ProjectModel | null {
  const p = event.payload;
  switch (event.kind) {
    case "workspace_view":
      return applyView(model, p);
    // The Project's Memory reached from elsewhere: the core brings its checkout forward with the Memory tool.
    case "project_memory_navigate":
      return p.workspace_id === PLACE.project ? applyView(model, { tool: "memory" }) : null;
    // The Explorer folds a folder through the core's ui state; only its folders are this world's.
    case "ui_state_update": {
      const paths = p.expanded_paths;
      if (!Array.isArray(paths)) return null;
      const expanded = paths.filter((path): path is string => typeof path === "string");
      return expanded.join("\n") === model.expanded.join("\n") ? null : { ...model, expanded };
    }
    default: {
      const desk = applyDesk(model.desk, event);
      return desk ? { ...model, desk } : null;
    }
  }
}

/** What each pane's terminal shows: the agent changing the payment code, or a shell. */
function paneScreen(pane: PracticePane): string {
  if (!pane.agent) return shellScreen(PLACE);
  return [
    "\x1b[38;5;209m✻\x1b[0m Claude Code",
    "",
    "\x1b[2m>\x1b[0m Checkout returns a 500 when I pay. Fix it",
    "",
    "\x1b[38;5;209m●\x1b[0m The provider timed out and the error was not caught.",
    "",
    "\x1b[38;5;209m●\x1b[0m \x1b[1mUpdate\x1b[0m(src/payment/checkout.ts)",
    "  \x1b[2m⎿  Catch the timeout and retry once\x1b[0m",
    "\x1b[38;5;209m●\x1b[0m \x1b[1mWrite\x1b[0m(src/payment/retry.ts)",
    "",
    "\x1b[38;5;209m✻\x1b[0m \x1b[2mRunning the payment tests… (esc to interrupt)\x1b[0m",
  ].join("\r\n");
}

export const PROJECT_WORLD: PracticeWorld<ProjectModel> = {
  initial,
  apply,
  snapshot,
  panes: (model) => model.desk.panes.map((pane) => pane.id),
  screen: (model, paneId) => paneScreen(model.desk.panes.find((pane) => pane.id === paneId)!),
  lane: true,
  examples: ["[data-todo-pr] button > span.flex-col > span:first-child"],
};
