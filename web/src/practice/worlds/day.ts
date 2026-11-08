// The start-of-day flow's world: the Overview, with one invented project that
// has an agent at work, two finished tasks, a backlog of issues, and three
// sessions from the days before that no pane holds any more. It answers the
// Overview's History search and the record the search opens, the record's
// Restore session (which brings the session back in a tab of its own), and
// the backlog's issue detail. The Work tab reads the same tasks the sidebar
// does, so nothing here is drawn twice.

import type { AgentScope, ProjectTasksScope } from "../../agentScope";
import type { AgentRow, AgentState, IssueDetail, SessionRecord, SessionSearchOptions, SessionSearchStatus, SessionWorkSearch, Task, TaskOperation, TaskSource, WorkSession, Workspace } from "../../snapshot";
import type { CoreEvent, PracticeWorld, WorldSnapshot } from "../world";
import { agent, applyDesk, deskSnapshot, oneTabDesk, shellScreen, type DeskModel, type PracticePane, type PracticePlace } from "./kit";

const PLACE: PracticePlace = { project: "hide-practice", label: "hide-practice", branch: "main" };
const REPOSITORY = `example/${PLACE.project}`;
const HOUR_MS = 3_600_000;
const MIN_MS = 60_000;

type Past = { session: WorkSession; title: string; hoursAgo: number; line: string; requests: [hoursAgo: number, text: string][]; pr: boolean };

/** The sessions from the days before: no pane holds them, so only History and search find them. */
export const LOGIN_SESSION: WorkSession = { device_id: "local", agent: "codex", session_id: "practice-login" };
const PAST: Past[] = [
  {
    session: LOGIN_SESSION, title: "Login drops after a minute", hoursAgo: 24, line: "Expiry follows the server's clock; test added", pr: true,
    requests: [[24, "The login drops people after a minute. Find out why"], [23.6, "Use the server's clock for the expiry"], [23.2, "Add a test for it too"]],
  },
  {
    session: { device_id: "local", agent: "claude", session_id: "practice-signup" }, title: "Reword the sign-up page", hoursAgo: 30, line: "New wording is live", pr: false,
    requests: [[30, "Reword the sign-up page so it reads shorter"]],
  },
  {
    session: { device_id: "local", agent: "codex", session_id: "practice-provider" }, title: "Compare payment providers", hoursAgo: 52, line: "Which one should we try first?", pr: false,
    requests: [[52, "Compare two payment providers for us"]],
  },
];
const LOGIN = PAST[0]!;
/** The pane yesterday's session comes back in when its record's Restore session is pressed. */
const RESTORED_PANE = "restored";

/** The backlog's issues, newest first; the first is the one the learner reads. */
const ISSUES: [number, string, string[]][] = [
  [12, "Show the card brand on the receipt", ["ux"]],
  [11, "Retry a failed payment once before giving up", ["bug"]],
  [9, "Document the webhook headers", ["docs"]],
];
export const ISSUE_KEY = `github:${REPOSITORY}#${ISSUES[0]![0]}`;

/**
 * What the world can change: the desk, what History asked last, which session's
 * record is open, the issue whose detail is open, and the start that brought a
 * session back.
 */
export type DayModel = { desk: DeskModel; query: string | null; options: SessionSearchOptions | undefined; record: string | null; issue: string | null; restore: { requestId: string | null; session: string } | null };

/** Whether History's last query holds `session` by whole words of its title or requests, as a learner types a word they remember. */
function finds(query: string | null, past: Past): boolean {
  const wanted = query?.trim().toLowerCase();
  if (!wanted) return false;
  const words = new RegExp(`(^|\\W)${wanted.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}($|\\W)`);
  return [past.title, ...past.requests.map(([, text]) => text)].some((text) => words.test(text.toLowerCase()));
}

export const findsLogin = (model: DayModel) => finds(model.query, LOGIN);
export const openedLogin = (model: DayModel) => model.record === LOGIN.session.session_id;
export const restoredLogin = (model: DayModel) => model.restore?.session === LOGIN.session.session_id;
export const readsIssue = (model: DayModel) => model.issue === ISSUE_KEY;

// --- The desk ------------------------------------------------------------------------------

/** How a root that finished reads, as the core states it: done and unread, or seen. */
function restedState(read: boolean, since: number, line: string): AgentState {
  return {
    attention: false, needs_you: false, root: true, title_emphasized: !read, selection_emphasizes_title: true,
    asking: false, working: false, waits_on_children: false,
    chip_tone: { kind: read ? "subtle" : "success", read: false }, mark_tone: { kind: read ? "subtle" : "success", read: false },
    line: null, branch_badge: null, bucket: read ? "resting" : "turn", attention_rank: read ? 4 : 2, link_rank: 2, search_tone: read ? "muted" : "done", subtree: "quiet", link: "idle",
    descendant_asking: 0, verb: "idle", request_todo: false, request_since: since,
    session: { group: "idle", line, unfinished: false }, ask: null, tree_rank: 3,
  };
}

function rested(paneId: string, title: string, read: boolean, hoursAgo: number, line: string): AgentRow {
  const since = Date.now() - hoursAgo * HOUR_MS;
  return { ...agent({ pane_id: paneId, identity_label: title, group: read ? "seen" : "done", mark: read ? "quiet" : "done", status_code: "idle", elapsed: `${hoursAgo}h` }), state: restedState(read, since, line), activity: "stopped", last_prompt_at_unix_ms: since };
}

function initial(): DayModel {
  const lead = agent({ pane_id: "lead", identity_label: "Fix the checkout 500", agent_kind: "claude", group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "2m", last_prompt_at_unix_ms: Date.now() - 2 * MIN_MS });
  const desk = oneTabDesk({ id: "lead", agent: lead, title: "Fix the checkout 500" });
  const copy: PracticePane = { id: "copy", agent: rested("copy", "Update the receipt email", false, 3, "The new receipt is sent"), title: "Update the receipt email" };
  const tabs = [...desk.tabs, { id: "t2", name: null, root: { type: "pane" as const, pane_id: copy.id }, focused: copy.id, zoomed: false }];
  return { desk: { ...desk, panes: [...desk.panes, copy], tabs, groups: [{ id: null, name: null, tabs: tabs.map((tab) => tab.id) }], nextId: 3 }, query: null, options: undefined, record: null, issue: null, restore: null };
}

/** The desk with a session back in a tab of its own and in front, as the core leaves it once Restore session's agent has started. */
function restored(desk: DeskModel, past: Past): DeskModel {
  const row: AgentRow = { ...rested(RESTORED_PANE, past.title, true, 0, past.line), agent_kind: past.session.agent, session_id: past.session.session_id };
  const tab = { id: `t${desk.nextId}`, name: null, root: { type: "pane" as const, pane_id: RESTORED_PANE }, focused: RESTORED_PANE, zoomed: false };
  return {
    ...desk,
    panes: [...desk.panes, { id: RESTORED_PANE, agent: row, title: past.title }],
    tabs: [...desk.tabs, tab],
    groups: desk.groups.map((group, at) => (at === 0 ? { ...group, tabs: [...group.tabs, tab.id] } : group)),
    activeTab: tab.id,
    nextId: desk.nextId + 1,
  };
}

// --- What the core answers -----------------------------------------------------------------

/** History's answer: with no query every session from the days before, else those whose words hold it. */
function sessionSearch(model: DayModel, nowMs: number): SessionWorkSearch {
  const query = model.query ?? "";
  const hits = PAST.filter((past) => !query.trim() || finds(query, past) || [past.title, ...past.requests.map(([, text]) => text)].some((text) => text.toLowerCase().includes(query.trim().toLowerCase())));
  return {
    query, options: model.options, loading: false, failure: null, next_cursor: null,
    results: hits.map((past) => {
      const asked = past.requests.map(([hoursAgo, text], offset) => ({ kind: "request" as const, text, at_unix_ms: nowMs - hoursAgo * HOUR_MS, request_offset: offset }));
      const matches = query.trim() ? asked.filter((one) => one.text.toLowerCase().includes(query.trim().toLowerCase())) : asked.slice(0, 1);
      return {
        session: past.session, title: past.title, matched: (matches[0] ?? asked[0]!).text, at_unix_ms: nowMs - past.hoursAgo * HOUR_MS, cwd: `/work/${PLACE.project}`, project_root: `/work/${PLACE.project}`,
        status: "archived" as SessionSearchStatus, matches: matches.slice(0, 2), match_count: matches.length, request_count: asked.length,
      };
    }),
  } as SessionWorkSearch;
}

function recordOf(past: Past, nowMs: number): SessionRecord {
  const at = (hoursAgo: number) => nowMs - hoursAgo * HOUR_MS;
  const url = `https://example.com/${PLACE.project}/pull/3`;
  const output = { kind: "pr" as const, repository: REPOSITORY, number: 3, title: "Fix session expiry", url, state: "open", stale: false, last_read_at_unix_ms: at(1), created_at_unix_ms: at(23), source: past.session, source_name: past.title };
  return {
    session: past.session, title: past.title, fallback_title: past.session.agent === "codex" ? "Codex" : "Claude", native_title: past.title, manual_title: null, state: "idle",
    cwd: `/work/${PLACE.project}`, project: PLACE.label, repository: REPOSITORY, path: `/work/.${past.session.agent}/sessions/${past.session.session_id}.jsonl`, started_at_unix_ms: at(past.hoursAgo), ended_at_unix_ms: at(past.hoursAgo - 1),
    status: { end: "done", line: past.line, at_unix_ms: at(past.hoursAgo - 1), limited: null, settled: null },
    asks: [],
    flows: [{
      id: `${past.session.session_id}-flow`, name: past.title, at_unix_ms: at(past.hoursAgo), request_count: past.requests.length,
      requests: past.requests.map(([hoursAgo, text], offset) => ({ offset, at_unix_ms: at(hoursAgo), text, author: "operator" as const, images: 0, summary: null, answers: null, letter_from: null, count: 1, goal_command: false })),
      result: { kind: "turn", end: "done", line: past.line, at_unix_ms: at(past.hoursAgo - 1) },
    }],
    omitted_requests: 0,
    outputs: past.pr ? [output] : [],
    pr_groups: past.pr ? [{ child: null, name: past.title, pulls: [{ url, number: 3, title: "Fix session expiry", state: "pending", own: true, holders: [], stale: false, last_read_at_unix_ms: at(1) }] }] : [],
    artifacts: [],
    children: [],
  };
}

/** The core's answer to the start Restore session sent: ready, its agent started in the pane it opened. */
function restoreOperation(restore: NonNullable<DayModel["restore"]>): TaskOperation {
  const past = PAST.find((one) => one.session.session_id === restore.session)!;
  return {
    id: 1, kind: "agent_start", phase: "ready", repository_root: `/work/${PLACE.project}`, branch: PLACE.branch, base_branch: null, path: `/work/${PLACE.project}`,
    pane_id: RESTORED_PANE, agent_kind: past.session.agent, failure: null, agent_phase: "started", agent_reason: null, agent_line: null, request_id: restore.requestId,
  };
}

// --- The backlog ---------------------------------------------------------------------------

const PALETTE: Record<string, string> = { bug: "d73a4a", ux: "a2eeef", docs: "0075ca" };

function tasksSource(nowMs: number): { source: TaskSource; tasks: Task[]; overflow: boolean } {
  const tasks: Task[] = ISSUES.map(([number, title, labels], at) => ({
    key: `github:${REPOSITORY}#${number}`, source: "github", id: `#${number}`, url: `https://example.com/${REPOSITORY}/issues/${number}`, title, open: true,
    created_at_unix_ms: nowMs - (at + 1) * 5 * HOUR_MS, updated_at_unix_ms: nowMs - (at + 1) * 3 * HOUR_MS, labels: labels.map((name) => ({ name, color: PALETTE[name] ?? "cccccc" })),
  }));
  return { source: { kind: "github", label: "GitHub", name: REPOSITORY, reading: false, failure: null, last_read_at_unix_ms: nowMs - 2 * MIN_MS, public: false }, tasks, overflow: false };
}

function issueDetail(model: DayModel, nowMs: number): IssueDetail | null {
  const task = tasksSource(nowMs).tasks.find((one) => one.key === model.issue);
  if (!task) return null;
  return {
    workspace_id: PLACE.project, device_id: "local", task_key: task.key, phase: "ready", body: "## Goal\n\nA customer sees which card they paid with, on the page and in the email.\n\n- [ ] Read the brand from the payment\n- [ ] Show it beside the last four digits", failure: null,
    labels: task.labels ?? [], author: "mira", created_at_unix_ms: task.created_at_unix_ms ?? null, assignees: [], comment_count: 0, comments_cursor: null, comments_has_more: false, comments_limited: false, comments_loading: false, comments: [],
  } as IssueDetail;
}

// --- The snapshot --------------------------------------------------------------------------

function tasksOf(model: DayModel, nowMs: number): ProjectTasksScope {
  const card = (root: string, state: "working" | "done" | "resting", minutesAgo: number) => ({ root, state, members: [root], checkouts: [`${PLACE.project}:${PLACE.branch}`], recency: new Date(nowMs - minutesAgo * MIN_MS).toISOString(), stage: 1 as const });
  return { cards: [card("lead", "working", 2), ...(model.restore ? [card(RESTORED_PANE, "resting", 0)] : []), card("copy", "done", 180)], needs_you: 0, recency: new Date(nowMs).toISOString() };
}

function snapshot(model: DayModel, nowMs: number): WorldSnapshot {
  const base = deskSnapshot([{ place: PLACE, model: model.desk }], PLACE.project, nowMs);
  const tasks = tasksOf(model, nowMs);
  const scoped = <S extends { agent_scope: AgentScope }>(holder: S): S => ({ ...holder, agent_scope: { ...holder.agent_scope, tasks } });
  const navigator = base.rest.navigator!;
  const workspaces: Workspace[] = navigator.workspaces!.map((project) => ({ ...scoped(project), tasks: tasksSource(nowMs), last_activity_unix_ms: nowMs - 2 * MIN_MS, checkouts: project.checkouts.map(scoped) }));
  const record = model.record ? PAST.find((past) => past.session.session_id === model.record) ?? null : null;
  return {
    ...base,
    rest: {
      ...base.rest,
      navigator: { ...navigator, workspaces, agent_scope: { ...navigator.agent_scope!, tasks }, devices: navigator.devices!.map(scoped) },
      ui_state: { ...base.rest.ui_state!, sidebar_lens: "tasks" as const },
      issue_work: { backlog: [], made_keys: [], missed_keys: [], detail: issueDetail(model, nowMs), name: null },
      task_operation: model.restore ? restoreOperation(model.restore) : null,
      // Hide AI's summaries on, so a record reads each flow's result as the operator's would.
      status: { ...base.rest.status, background_ai: { enabled: true, provider: "claude", chosen: true, agent_summary: true, providers: [], unavailable_reason: null } },
    },
    frames: {
      sessionWorkSearch: model.query === null ? null : sessionSearch(model, nowMs),
      sessionRecord: record ? { session: record.session, loading: false, failure: null, record: recordOf(record, nowMs) } : null,
    },
  };
}

// --- The events ----------------------------------------------------------------------------

function apply(model: DayModel, event: CoreEvent): DayModel | null {
  const p = event.payload;
  switch (event.kind) {
    case "session_work_search":
      return { ...model, query: String(p.query ?? ""), options: p.options as SessionSearchOptions | undefined };
    case "session_record_open": {
      const session = p.session as WorkSession | undefined;
      const past = PAST.find((one) => one.session.session_id === session?.session_id);
      return past ? { ...model, record: past.session.session_id } : null;
    }
    case "issue_detail_request":
      return typeof p.task_key === "string" && p.task_key !== model.issue ? { ...model, issue: p.task_key } : null;
    // Restore session on a record: the session comes back once; a second press finds it running.
    case "agent_start_in_checkout": {
      const past = PAST.find((one) => one.session.session_id === p.resume_session_id);
      if (!past || model.restore) return null;
      return { ...model, desk: restored(model.desk, past), restore: { requestId: typeof p.request_id === "string" ? p.request_id : null, session: past.session.session_id } };
    }
    default: {
      const desk = applyDesk(model.desk, event);
      return desk ? { ...model, desk } : null;
    }
  }
}

/** What each pane's terminal shows: the agent at work, the one that finished, a session brought back, or a shell. */
function paneScreen(model: DayModel, pane: PracticePane): string {
  if (!pane.agent) return shellScreen(PLACE);
  if (pane.id === RESTORED_PANE) {
    const past = PAST.find((one) => one.session.session_id === model.restore?.session)!;
    return ["\x1b[1m>_ OpenAI Codex\x1b[0m \x1b[2m(resumed)\x1b[0m", "", ...past.requests.map(([, text]) => `\x1b[2m›\x1b[0m ${text}`), "", `\x1b[32m•\x1b[0m ${past.line}.`, "", "\x1b[2m›\x1b[0m "].join("\r\n");
  }
  if (pane.id === "lead") {
    return ["\x1b[38;5;209m✻\x1b[0m Claude Code", "", "\x1b[2m>\x1b[0m Checkout returns a 500 when I pay. Fix it", "", "\x1b[38;5;209m●\x1b[0m Following the payment request through the code.", "  \x1b[2m⎿  Reading src/payment/checkout.ts…\x1b[0m", "", "\x1b[38;5;209m✻\x1b[0m \x1b[2mLooking for the cause… (esc to interrupt)\x1b[0m"].join("\r\n");
  }
  return ["\x1b[38;5;209m✻\x1b[0m Claude Code", "", `\x1b[2m>\x1b[0m ${pane.title}`, "", `\x1b[38;5;209m●\x1b[0m ${pane.agent.state.session.line ?? ""}`].join("\r\n");
}

export const DAY_WORLD: PracticeWorld<DayModel> = {
  initial,
  apply,
  snapshot,
  panes: (model) => model.desk.panes.map((pane) => pane.id),
  screen: (model, paneId) => paneScreen(model, model.desk.panes.find((pane) => pane.id === paneId)!),
};
