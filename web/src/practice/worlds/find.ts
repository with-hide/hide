// Chapter 5's world: one invented project with an agent at work, two
// finished tasks in the sidebar's 태스크 list, and a session from yesterday
// that only search still finds, with the pull request it made. It answers
// ⌘K's session search, the record and the card the lane's 보기 slot shows,
// the record's Restore session, and the ★ that puts a task in 다시 볼 것.

import type { AgentScope } from "../../agentScope";
import type { AgentRow, AgentState, DetailItem, LinkPanel, PrCard, PullRequest, SessionRecord, SessionWorkSearch, TaskOperation, WorkSession } from "../../snapshot";
import type { CoreEvent, PracticeWorld } from "../world";
import { agent, applyDesk, deskSnapshot, oneTabDesk, shellScreen, type DeskModel, type PracticePane, type PracticePlace } from "./kit";

const PLACE: PracticePlace = { project: "hide-practice", label: "hide-practice", branch: "main" };
const HOUR_MS = 3_600_000;
const MIN_MS = 60_000;

/** The session from yesterday: no pane holds it any more, so only search finds it. */
export const LOGIN_SESSION: WorkSession = { device_id: "local", agent: "codex", session_id: "practice-login" };
const LOGIN_TITLE = "Login drops after a minute";
/** The pull request that session made; every practice pull request is an example, at the address set aside for examples. */
export const PR_NUMBER = 3;
const PR_TITLE = "Fix session expiry";
const PR_URL = `https://example.com/${PLACE.project}/pull/${PR_NUMBER}`;
/** ⌘K's row for that pull request. */
export const PR_ROW = `[data-palette-row="pr:${PLACE.project}:${PR_NUMBER}"]`;
/** The pane yesterday's session comes back in when its record's Restore session is pressed. */
const RESTORED_PANE = "login";

/**
 * What the world can change: the desk, what ⌘K asked last, the 보기 slot,
 * which tasks wear the ★, and the start that brought yesterday's session back.
 */
export type FindModel = { desk: DeskModel; searched: boolean; query: string | null; detail: DetailItem[]; starred: string[]; restore: { requestId: string | null } | null };

/** The 보기 slot's item in front. */
export const shownDetail = (model: FindModel): DetailItem | null => model.detail.at(-1) ?? null;

/** How a root that finished reads, as the core states it: done and unread, or seen. */
function restedState(read: boolean, since: number, line: string): AgentState {
  return {
    attention: false, needs_you: false, root: true, title_emphasized: !read, selection_emphasizes_title: true,
    asking: false, working: false, waits_on_children: false,
    chip_tone: { kind: read ? "subtle" : "success", read: false }, mark_tone: { kind: read ? "subtle" : "success", read: false },
    line: null, branch_badge: null, bucket: read ? "resting" : "turn", attention_rank: read ? 4 : 2, link_rank: 2, search_tone: read ? "muted" : "done", subtree: "quiet", link: "idle",
    descendant_asking: 0, verb: "idle", request_todo: false, request_since: since,
    session: { group: "idle", line, unfinished: false }, ask: null, tree_rank: 3, pr: null,
  };
}

function rested(paneId: string, title: string, read: boolean, hoursAgo: number, line: string, revisit: AgentRow["revisit"] = null): AgentRow {
  const since = Date.now() - hoursAgo * HOUR_MS;
  return {
    ...agent({ pane_id: paneId, identity_label: title, group: read ? "seen" : "done", mark: read ? "quiet" : "done", status_code: "idle", elapsed: `${hoursAgo}h` }),
    state: restedState(read, since, line),
    activity: "stopped",
    last_prompt_at_unix_ms: since,
    revisit,
  };
}

function initial(): FindModel {
  const lead = agent({ pane_id: "lead", identity_label: "Fix the checkout 500", agent_kind: "claude", group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "2m", last_prompt_at_unix_ms: Date.now() - 2 * MIN_MS });
  const desk = oneTabDesk({ id: "lead", agent: lead, title: "Fix the checkout 500" });
  const copy: PracticePane = { id: "copy", agent: rested("copy", "Reword the sign-up page", false, 3, "New wording is live"), title: "Reword the sign-up page" };
  const provider: PracticePane = {
    id: "provider",
    agent: rested("provider", "Compare payment providers", true, 5, "Which one should we try first?", { starred: false, read_ask: true, since_unix_ms: Date.now() - 4 * HOUR_MS }),
    title: "Compare payment providers",
  };
  const tabs = [...desk.tabs, ...[copy, provider].map((pane, at) => ({ id: `t${at + 2}`, name: null, root: { type: "pane" as const, pane_id: pane.id }, focused: pane.id, zoomed: false }))];
  return {
    desk: { ...desk, panes: [...desk.panes, copy, provider], tabs, groups: [{ id: null, name: null, tabs: tabs.map((tab) => tab.id) }], nextId: 4 },
    searched: false,
    query: null,
    detail: [],
    starred: [],
    restore: null,
  };
}

// --- What the core answers -----------------------------------------------------

/** What of yesterday's session `query` finds: its title, or the first request that holds it; null when it finds nothing. */
function loginMatch(query: string): string | null {
  const wanted = query.trim().toLowerCase();
  if (!wanted || wanted.startsWith("#")) return null;
  const said = LOGIN_REQUESTS.find(([, text]) => text.toLowerCase().includes(wanted));
  return said ? said[1] : LOGIN_TITLE.toLowerCase().includes(wanted) ? LOGIN_TITLE : null;
}

/** Whether `query` is whole words of one of `texts`, as a learner types a word they remember rather than its first letters. */
function wholeWords(query: string, texts: readonly string[]): boolean {
  const words = new RegExp(`(^|\\W)${query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}($|\\W)`);
  return texts.some((text) => words.test(text.toLowerCase()));
}

/**
 * Whether ⌘K's last query finds yesterday's session by whole words of it,
 * whichever ones the learner typed: `clock` does, and so does `login`, but not
 * its first letter, which the search already answers while the word is typed.
 */
export const findsLogin = (model: FindModel) => {
  const wanted = model.query?.trim().toLowerCase();
  return !!wanted && loginMatch(wanted) !== null && wholeWords(wanted, [LOGIN_TITLE, ...LOGIN_REQUESTS.map(([, text]) => text)]);
};

/** Whether ⌘K's last query lists the pull request, as the shell lists one: by its number, with or without `#`, or by whole words of its title. */
export const findsPr = (model: FindModel) => {
  const wanted = model.query?.trim().toLowerCase();
  return !!wanted && (wanted === `#${PR_NUMBER}` || wanted === String(PR_NUMBER) || wholeWords(wanted, [PR_TITLE]));
};

/** ⌘K's session search: the words of yesterday's session match a query its title or requests hold. */
function sessionSearch(query: string, nowMs: number): SessionWorkSearch {
  const matched = loginMatch(query);
  return {
    query,
    loading: false,
    failure: null,
    results: matched ? [{
      session: LOGIN_SESSION, title: LOGIN_TITLE, matched, at_unix_ms: nowMs - 22 * HOUR_MS, cwd: `/work/${PLACE.project}`, project_root: `/work/${PLACE.project}`,
      status: "done", match_count: 1, request_count: LOGIN_REQUESTS.length,
    }] : [],
  };
}

/** What the operator asked yesterday's session, hours before now. */
const LOGIN_REQUESTS: [hoursAgo: number, text: string][] = [
  [24, "The login drops people after a minute. Find out why"],
  [23.6, "Use the server's clock for the expiry"],
  [23.2, "Add a test for it too"],
];

function loginRecord(nowMs: number): SessionRecord {
  const at = (hoursAgo: number) => nowMs - hoursAgo * HOUR_MS;
  const output = { kind: "pr" as const, repository: `example/${PLACE.project}`, number: PR_NUMBER, title: PR_TITLE, url: PR_URL, state: "open", stale: false, last_read_at_unix_ms: at(1), created_at_unix_ms: at(23), source: LOGIN_SESSION, source_name: LOGIN_TITLE };
  return {
    session: LOGIN_SESSION, title: LOGIN_TITLE, fallback_title: "Codex", native_title: LOGIN_TITLE, manual_title: null, state: "idle",
    cwd: `/work/${PLACE.project}`, project: PLACE.label, repository: `example/${PLACE.project}`, path: `/work/.codex/sessions/${LOGIN_SESSION.session_id}.jsonl`, started_at_unix_ms: at(24), ended_at_unix_ms: at(22.8),
    status: { end: "done", line: "Expiry follows the server's clock; test added", at_unix_ms: at(22.8), limited: null, settled: null },
    asks: [{ id: "review", kind: "follow_up", text: `Review #${PR_NUMBER} before it merges`, status: "open", at_unix_ms: at(22.8), turn_offset: 3, issue_repository: null, issue_number: null, backlog_key: null, creating: false }],
    flows: [{
      id: "login-flow", name: LOGIN_TITLE, at_unix_ms: at(24), request_count: LOGIN_REQUESTS.length,
      requests: LOGIN_REQUESTS.map(([hoursAgo, text], offset) => ({ offset, at_unix_ms: at(hoursAgo), text, author: "operator" as const, images: 0, summary: null, answers: null, letter_from: null, count: 1, goal_command: false })),
      result: { kind: "turn", end: "done", line: "Expiry follows the server's clock; test added", at_unix_ms: at(22.8) },
    }],
    omitted_requests: 0,
    outputs: [output],
    pr_groups: [{ child: null, name: LOGIN_TITLE, outputs: [output] }],
    artifacts: [],
    children: [],
  };
}

function pullRequest(nowMs: number): PullRequest {
  return {
    number: PR_NUMBER, title: PR_TITLE, url: PR_URL, badge: "review", review: "review_required", is_draft: false, checks: "passing",
    created_at_unix_ms: nowMs - 23 * HOUR_MS, updated_at_unix_ms: nowMs - 22 * HOUR_MS, head_branch: "fix/session-expiry", base_branch: "main", closing_issues: [], state: "mergeable",
    work: {
      author: "you", author_is_bot: false, mine: true, merge_state: "clean", review_requests: [{ kind: "user", name: "you" }], asked_of_me: "direct",
      failed_checks: [], pending_checks: 0, check_counts: { total: 3, passed: 3, failed: 0, running: 0, waiting: 0, skipped: 0, unknown: 0 },
      queue: { state: "not_queued" }, review_requested_at_unix_ms: nowMs - 22 * HOUR_MS,
    },
  };
}

function prCard(nowMs: number): PrCard {
  return {
    workspace_id: PLACE.project, number: PR_NUMBER, reading: false, head: "0000003",
    body: "Sessions now expire by the server's clock, so a laptop whose clock runs fast no longer logs people out.",
    files: [{ path: "src/auth/session.ts", additions: 18, deletions: 6, generated: false }, { path: "src/auth/session.test.ts", additions: 34, deletions: 0, generated: false }],
    files_total: 2, threads: [], merge: { methods: ["squash"], queue: false, can_merge: true }, failed: [], checks_completed_at_unix_ms: nowMs - 22 * HOUR_MS, write: null,
  };
}

function linkPanel(nowMs: number): LinkPanel {
  return {
    workspace_id: PLACE.project, target: { kind: "pr", number: PR_NUMBER }, loading: false, failure: null,
    pr: {
      number: PR_NUMBER, branch: "fix/session-expiry", title: PR_TITLE, url: PR_URL, created_at_unix_ms: nowMs - 23 * HOUR_MS, closed_at_unix_ms: null, merged_at_unix_ms: null,
      state: "pending", issues: [], worktrees: [],
    },
    prs: [],
    sessions: [{
      agent: LOGIN_SESSION.agent, id: LOGIN_SESSION.session_id, ids: [LOGIN_SESSION.session_id], device_id: LOGIN_SESSION.device_id, role: "created", pr: PR_NUMBER,
      request: LOGIN_REQUESTS[0]![1], started_at_unix_ms: nowMs - 24 * HOUR_MS, ended_at_unix_ms: nowMs - 22.8 * HOUR_MS, path: null, cwd: `/work/${PLACE.project}`, file: "present", parent: null,
    }],
    total: 1,
  };
}

/**
 * The desk with yesterday's session back in a tab of its own and in front, as
 * the core leaves it once Restore session's agent has started: Codex, idle on
 * the conversation it ended with.
 */
function restored(desk: DeskModel): DeskModel {
  const row: AgentRow = { ...rested(RESTORED_PANE, LOGIN_TITLE, true, 0, "Expiry follows the server's clock; test added"), agent_kind: LOGIN_SESSION.agent, session_id: LOGIN_SESSION.session_id };
  const tab = { id: `t${desk.nextId}`, name: null, root: { type: "pane" as const, pane_id: RESTORED_PANE }, focused: RESTORED_PANE, zoomed: false };
  return {
    ...desk,
    panes: [...desk.panes, { id: RESTORED_PANE, agent: row, title: LOGIN_TITLE }],
    tabs: [...desk.tabs, tab],
    groups: desk.groups.map((group, at) => (at === 0 ? { ...group, tabs: [...group.tabs, tab.id] } : group)),
    activeTab: tab.id,
    nextId: desk.nextId + 1,
  };
}

/** The core's answer to the start Restore session sent: ready, its agent started in the pane it opened. */
function restoreOperation(requestId: string | null): TaskOperation {
  return {
    id: 1, kind: "agent_start", phase: "ready", repository_root: `/work/${PLACE.project}`, branch: PLACE.branch, base_branch: null, path: `/work/${PLACE.project}`,
    pane_id: RESTORED_PANE, agent_kind: LOGIN_SESSION.agent, failure: null, agent_phase: "started", agent_reason: null, agent_line: null, request_id: requestId,
  };
}

// --- The snapshot ---------------------------------------------------------------

/** The tasks the sidebar's 태스크 list draws, in the core's order: open work first, 다시 볼 것 above the finished one. */
function tasksOf(model: FindModel, nowMs: number) {
  const revisit = (paneId: string) => paneId === "provider" || model.starred.includes(paneId);
  const card = (root: string, state: string, minutesAgo: number) => ({ root, state, members: [root], checkouts: [`${PLACE.project}:${PLACE.branch}`], recency: new Date(nowMs - minutesAgo * MIN_MS).toISOString() });
  return {
    cards: [
      card("lead", "working", 2), ...(model.restore ? [card(RESTORED_PANE, "resting", 0)] : []),
      card("provider", "revisit", 240), card("copy", revisit("copy") ? "revisit" : "done", 180),
    ],
    needs_you: 0,
    recency: new Date(nowMs).toISOString(),
  } as AgentScope["tasks"];
}

function withRevisit(row: AgentRow, model: FindModel): AgentRow {
  if (!model.starred.includes(row.pane_id)) return row;
  return { ...row, revisit: { starred: true, read_ask: row.revisit?.read_ask ?? false, since_unix_ms: Date.now() } };
}

function snapshot(model: FindModel, nowMs: number) {
  const base = deskSnapshot([{ place: PLACE, model: model.desk }], PLACE.project, nowMs);
  const agents = base.agents.map((row) => withRevisit(row, model));
  const tasks = tasksOf(model, nowMs);
  const scoped = <S extends { agent_scope: AgentScope }>(holder: S): S => ({ ...holder, agent_scope: { ...holder.agent_scope, tasks } });
  const navigator = base.rest.navigator!;
  const pulls = [pullRequest(nowMs)];
  const workspaces = navigator.workspaces!.map((project) => ({ ...scoped(project), pull_requests: pulls, checkouts: project.checkouts.map(scoped) }));
  const top = shownDetail(model);
  return {
    rest: {
      ...base.rest,
      navigator: { ...navigator, agents, workspaces, agent_scope: { ...navigator.agent_scope!, tasks }, devices: navigator.devices!.map(scoped) },
      ui_state: { ...base.rest.ui_state!, sidebar_lens: "tasks" as const, detail: { items: model.detail, shown: model.detail.length > 0 } },
      pr_work: { link: null, feedback: null, cards: [prCard(nowMs)] },
      task_operation: model.restore ? restoreOperation(model.restore.requestId) : null,
      // Opening the 보기 slot opens the lane, as the core does on `detail_open`.
      workspace_view: { ...base.rest.workspace_view!, open: model.detail.length > 0 },
      // Hide AI's summaries on, so the record reads each flow's result as the operator's would.
      status: { ...base.rest.status, background_ai: { enabled: true, provider: "claude", chosen: true, agent_summary: true, providers: [], unavailable_reason: null } },
    },
    agents,
    focused: base.focused,
    usage: null,
    frames: {
      sessionWorkSearch: model.query === null ? null : sessionSearch(model.query, nowMs),
      sessionRecord: top?.kind === "record" ? { session: top.session, loading: false, failure: null, record: loginRecord(nowMs) } : null,
      linkPanel: top?.kind === "pull_request" ? linkPanel(nowMs) : null,
    },
  };
}

// --- The events --------------------------------------------------------------

function apply(model: FindModel, event: CoreEvent): FindModel | null {
  const p = event.payload;
  switch (event.kind) {
    case "session_work_search":
      return { ...model, searched: true, query: String(p.query ?? "") };
    case "detail_open": {
      const item = p.item as DetailItem;
      return item.kind === "record" || item.kind === "pull_request" ? { ...model, detail: [...model.detail, item] } : null;
    }
    case "pr_open":
      return p.pr_number === PR_NUMBER ? { ...model, detail: [...model.detail, { kind: "pull_request", workspace_id: PLACE.project, number: PR_NUMBER }] } : null;
    // Restore session on yesterday's record: the session comes back once; a second press finds it running.
    case "agent_start_in_checkout":
      if (p.resume_session_id !== LOGIN_SESSION.session_id || model.restore) return null;
      return { ...model, desk: restored(model.desk), restore: { requestId: typeof p.request_id === "string" ? p.request_id : null } };
    case "revisit_star":
      return model.starred.includes(String(p.pane_id)) ? null : { ...model, starred: [...model.starred, String(p.pane_id)] };
    case "revisit_done":
      return { ...model, starred: model.starred.filter((id) => id !== p.pane_id) };
    default: {
      const desk = applyDesk(model.desk, event);
      return desk ? { ...model, desk } : null;
    }
  }
}

/** What each pane's terminal shows: the agent at work, the two that finished, or a shell. */
function paneScreen(pane: PracticePane): string {
  if (!pane.agent) return shellScreen(PLACE);
  if (pane.id === RESTORED_PANE) {
    return [
      "\x1b[1m>_ OpenAI Codex\x1b[0m \x1b[2m(resumed)\x1b[0m",
      "",
      ...LOGIN_REQUESTS.map(([, text]) => `\x1b[2m›\x1b[0m ${text}`),
      "",
      "\x1b[32m•\x1b[0m Expiry follows the server's clock; test added.",
      "",
      "\x1b[2m›\x1b[0m ",
    ].join("\r\n");
  }
  if (pane.id === "lead") {
    return [
      "\x1b[38;5;209m✻\x1b[0m Claude Code",
      "",
      "\x1b[2m>\x1b[0m Checkout returns a 500 when I pay. Fix it",
      "",
      "\x1b[38;5;209m●\x1b[0m Following the payment request through the code.",
      "  \x1b[2m⎿  Reading src/payment/checkout.ts…\x1b[0m",
      "",
      "\x1b[38;5;209m✻\x1b[0m \x1b[2mLooking for the cause… (esc to interrupt)\x1b[0m",
    ].join("\r\n");
  }
  return ["\x1b[38;5;209m✻\x1b[0m Claude Code", "", `\x1b[2m>\x1b[0m ${pane.title}`, "", `\x1b[38;5;209m●\x1b[0m ${pane.agent.state.session.line ?? ""}`].join("\r\n");
}

export const FIND_WORLD: PracticeWorld<FindModel> = {
  initial,
  apply,
  snapshot,
  panes: (model) => model.desk.panes.map((pane) => pane.id),
  screen: (model, paneId) => paneScreen(model.desk.panes.find((pane) => pane.id === paneId)!),
  examples: [`[data-pr-card="${PR_NUMBER}"] [data-pr-card-title]`, PR_ROW],
};
