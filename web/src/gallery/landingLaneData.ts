// What the landing's lane shots open in the lane's 보기 slot, over the hero's
// story: the card of pull request 431, the lead's session record, and the mission
// the lead conducts. Invented English values; nothing here is read or sent.
import type { AgentRow, DetailItem, LinkPanel, LinkedSession, Mission, MissionNode, PrCard, PullRequest, SessionRecord, WorkFlow, WorkOutput, WorkRequest, WorkSession } from "../snapshot";

const MINUTE = 60_000;
/** An issue or pull request number as the page writes it, built so no literal reads as a color. */
const ref = (n: number) => `#${n}`;

/** The lead's pane, which conducts the mission and whose record the record shot opens. */
export const LEAD_PANE = "a1";
/** The project the hero shows, by the id the frozen sidebar fixture gives it. */
const PROJECT = "hide";
const REPOSITORY = "example/billing-app";
const pullUrl = (n: number) => `https://example.invalid/pull/${n}`;

/** Which item the lane shot opens. */
export type LandingLane = "pr" | "record" | "conductor";

/** A pane's session id in the story: the frozen sidebar rows carry none, so the lane shots give each one. */
const sessionId = (pane: string) => `session-${pane}`;

/** The rows with their sessions, so a record or a card finds the live pane its session runs in. */
export function withSessions(agents: AgentRow[]): AgentRow[] {
  return agents.map((row) => (row.session_id ? row : { ...row, session_id: sessionId(row.pane_id) }));
}

/** The lead's session, as the record and the mission name it. */
export function leadSession(agents: AgentRow[]): WorkSession {
  const lead = agents.find((row) => row.pane_id === LEAD_PANE);
  return { device_id: "local", agent: lead?.agent_kind ?? "claude", session_id: lead?.session_id ?? sessionId(LEAD_PANE) };
}

export function laneItem(lane: LandingLane, agents: AgentRow[]): DetailItem {
  if (lane === "pr") return { kind: "pull_request", workspace_id: PROJECT, number: 431 };
  if (lane === "record") return { kind: "record", session: leadSession(agents) };
  return { kind: "mission", mission: "mission-export" };
}

/** Pull request 431 as GitHub would answer the card: its body, files and a mergeable head. */
export function landingPrCard(): PrCard {
  return {
    workspace_id: PROJECT, number: 431, reading: false, head: "c".repeat(40),
    body: [
      "## Summary",
      "Invoices can now be exported as CSV from the invoices page.",
      "One header row, then one row per invoice, sorted by issue date. An empty list still exports the header.",
      "",
      "## Review",
      "Check the column order in `toCsv` and the quoting of customer names with commas.",
      "",
      "## Evidence",
      "`pnpm test export`: 38 passed. Checked in the browser: Export CSV downloads 128 rows.",
    ].join("\n"),
    files: [
      { path: "src/export/csv.ts", additions: 64, deletions: 3, generated: false },
      { path: "src/export/button.tsx", additions: 24, deletions: 3, generated: false },
      { path: "src/export/csv.test.ts", additions: 88, deletions: 0, generated: false },
    ],
    files_total: 3, threads: [], merge: { methods: ["merge", "squash"], queue: false, can_merge: true }, failed: [], write: null,
  };
}

/** What the pull request's row carries for the card's head and checks. */
export function landingPrWork(pull: PullRequest): PullRequest {
  return {
    ...pull, review: "approved", checks: "passing",
    work: { author: "you", author_is_bot: false, mine: true, merge_state: "clean", review_requests: [], asked_of_me: "none", failed_checks: [], pending_checks: 0, check_counts: { total: 12, passed: 12, failed: 0, running: 0, waiting: 0, skipped: 0, unknown: 0 }, queue: { state: "not_queued" }, review_requested_at_unix_ms: null },
  } as PullRequest;
}

/** The card's timeline: the worker that opened it, the agent that checked it, and the lead. */
export function landingPrLink(nowMs: number, agents: AgentRow[], pull: PullRequest): LinkPanel {
  const session = (pane: string, agent: string, minutes: number, role: LinkedSession["role"], request: string): LinkedSession => {
    const id = agents.find((row) => row.pane_id === pane)?.session_id ?? sessionId(pane);
    return { agent, id, ids: [id], device_id: "local", role, pr: 431, request, started_at_unix_ms: nowMs - minutes * MINUTE, ended_at_unix_ms: null, path: `/sessions/${id}`, cwd: "/work/billing-app", file: "present", parent: null };
  };
  const sessions = [
    session("a1c1", "codex", 52, "created", "Export invoices as CSV"),
    session("a3", "codex", 18, "worked", `Check ${ref(431)} and ${ref(436)}, then ask me`),
    session(LEAD_PANE, "claude", 64, "worked", "Split the invoice export in three"),
  ];
  return {
    workspace_id: PROJECT, target: { kind: "pr", number: 431 }, loading: false, failure: null,
    pr: { number: 431, branch: pull.head_branch ?? "feat/export-csv", title: pull.title, url: pull.url, created_at_unix_ms: nowMs - 52 * MINUTE, closed_at_unix_ms: null, merged_at_unix_ms: null, state: "mergeable", issues: [], worktrees: [] },
    prs: [], sessions, total: sessions.length,
  } as LinkPanel;
}

const request = (nowMs: number, offset: number, minutes: number, text: string, patch: Partial<WorkRequest> = {}): WorkRequest =>
  ({ offset, text, at_unix_ms: nowMs - minutes * MINUTE, author: "operator", images: 0, summary: text, answers: null, letter_from: null, count: 1, goal_command: false, ...patch });

/** The lead's record: what you asked, how it went, the workers it started and the pull requests they opened. */
export function landingRecord(nowMs: number, agents: AgentRow[]): SessionRecord {
  const lead = leadSession(agents);
  const pr = (number: number, title: string, state: string, source: WorkSession, name: string): WorkOutput =>
    ({ kind: "pr", repository: REPOSITORY, number, title, url: pullUrl(number), state, stale: false, last_read_at_unix_ms: nowMs, created_at_unix_ms: nowMs - number * 1000, source, source_name: name });
  const flow = (id: string, name: string, minutes: number, requests: WorkRequest[], result: WorkFlow["result"]): WorkFlow =>
    ({ id, name, at_unix_ms: nowMs - minutes * MINUTE, requests, request_count: requests.filter((one) => !one.letter_from).length, result });
  const child = (pane: string, agent: string, title: string, end: "done" | "working", line: string, outputs: [number, string, string][]): SessionRecord => {
    const row = agents.find((one) => one.pane_id === pane);
    const session = { device_id: "local", agent: row?.agent_kind ?? agent, session_id: row?.session_id ?? sessionId(pane) };
    const made = outputs.map(([number, name, state]) => pr(number, name, state, session, title));
    return {
      session, title, fallback_title: agent, native_title: title, manual_title: null, state: end === "working" ? "working" : "idle", cwd: "/work/billing-app", path: null,
      started_at_unix_ms: nowMs - 50 * MINUTE, ended_at_unix_ms: null, status: { end, line, at_unix_ms: nowMs - 4 * MINUTE, limited: null, settled: null },
      asks: [], flows: [], omitted_requests: 0, outputs: made, pr_groups: made.length ? [{ child: null, name: title, outputs: made }] : [], artifacts: [], children: [],
    };
  };
  const children = [
    child("a1c1", "codex", "Export invoices as CSV", "done", `Opened ${ref(431)}, tests pass`, [[431, "Export invoices as CSV", "mergeable"]]),
    child("a1c3", "codex", "Sort the export by date", "working", "Running the tests", [[432, "Sort the export by date", "pending"]]),
    child("a1c6", "codex", "Add the export button", "done", "Checked the button in the browser", [[436, "Add the export button", "mergeable"]]),
  ];
  const outputs = children.flatMap((one) => one.outputs);
  return {
    session: lead, title: "Split the invoice export in three", fallback_title: "Claude Code", native_title: "Invoice export", manual_title: null, state: "working",
    cwd: "/work/billing-app", path: `/sessions/${lead.session_id}`, started_at_unix_ms: nowMs - 70 * MINUTE, ended_at_unix_ms: null,
    status: { end: "waiting", line: "Waiting on 3 workers", at_unix_ms: nowMs - 2 * MINUTE, limited: null, settled: null },
    asks: [{ id: "ask-format", kind: "ask", text: "CSV only, or XLSX too?", status: "open", at_unix_ms: nowMs - 30 * MINUTE, turn_offset: 4, issue_repository: null, issue_number: null, creating: false, backlog_key: null }],
    flows: [
      flow("plan", "Plan the export", 70, [
        request(nowMs, 1, 70, "Customers want their invoices as a file. Plan the export and tell me how you'd split it.", { summary: "Plan the invoice export" }),
        request(nowMs, 2, 62, "Go with three parts, one Codex worker each.", { summary: "Three parts, one worker each", answers: "How to split the work" }),
      ], { kind: "turn", end: "done", line: "Three workers started", at_unix_ms: nowMs - 58 * MINUTE }),
      flow("check", "Check each pull request", 40, [
        request(nowMs, 3, 40, "Check each pull request before you bring it to me.", { summary: "Check each pull request first" }),
        request(nowMs, 4, 22, `${ref(431)} is ready and its tests pass.`, { letter_from: "Export invoices as CSV", summary: null }),
      ], null),
    ],
    omitted_requests: 0, outputs, pr_groups: children.map((one) => ({ child: one.session, name: one.title, outputs: one.outputs })).filter((group) => group.outputs.length), artifacts: [], children,
  };
}

function node(key: string, title: string, extra: Partial<MissionNode>): MissionNode {
  return { key, title, kind: "change", depends_on: [], status: "planned", blocked: null, delivery: "pr", worker: null, agent_id: null, pane_id: null, machine: null, pr: null, base_pr: null, branch: null, pr_state: null, pr_url: null, stage: null, attempts: 0, line: null, change: null, review: null, criteria: null, started_at_unix_ms: null, finished_at_unix_ms: null, ...extra };
}

/** The mission the lead conducts: one research node, four changes and a check together, each on the hero's panes. */
export function landingMission(nowMs: number, agents: AgentRow[]): Mission {
  const ago = (minutes: number) => nowMs - minutes * MINUTE;
  const opus = { agent: "claude", model: "Opus", effort: "high", effort_auto: true };
  const codex = { agent: "codex", model: "GPT-5.6-Sol", effort: "medium", effort_auto: true };
  const done = (started: number, length: number) => ({ status: "done" as const, started_at_unix_ms: ago(started), finished_at_unix_ms: ago(started - length) });
  const working = (started: number) => ({ status: "working" as const, started_at_unix_ms: ago(started) });
  const nodes = [
    node("read", "Read how exports work today", { kind: "research", delivery: null, worker: opus, ...done(66, 8), line: "Findings in the plan" }),
    node("csv", "Export invoices as CSV", { depends_on: ["read"], worker: codex, pane_id: "a1c1", ...done(52, 30), pr: 431, pr_state: "mergeable", stage: 3, pr_url: pullUrl(431) }),
    node("sort", "Sort the export by date", { depends_on: ["read"], worker: codex, pane_id: "a1c3", ...working(40), pr: 432, pr_state: "pending", stage: 2, pr_url: pullUrl(432) }),
    node("button", "Add the export button", { depends_on: ["csv"], worker: codex, pane_id: "a1c6", ...done(30, 18), pr: 436, pr_state: "mergeable", stage: 3, pr_url: pullUrl(436) }),
    node("empty", "Handle an empty invoice list", { depends_on: ["csv"], worker: codex, pane_id: "a1c7", ...working(12), pr: 435, pr_state: "pending", stage: 2, pr_url: pullUrl(435) }),
    node("together", `Check ${ref(431)} and ${ref(436)} together`, { kind: "verify", delivery: null, depends_on: ["csv", "button"], worker: { agent: "codex", model: "GPT-5.6-Sol", effort: "low", effort_auto: false }, pane_id: "a3", ...done(18, 10), line: "38 passed" }),
  ];
  return {
    id: "mission-export", title: "Invoice export", workspace_id: PROJECT, project: "/work/billing-app", checkout_path: "/work/billing-app", device_id: "local",
    status: "running", scope: "pr", cap: 4,
    conductor: { pane_id: LEAD_PANE, name: "Invoice export · Conductor", agent_kind: "claude", session: leadSession(agents), present: true },
    nodes, merge_order: ["csv", "button", "sort", "empty"], check: "pnpm test", github: "example",
    decision: null, delay: null, report: null,
    created_at_unix_ms: ago(70), started_at_unix_ms: ago(66), closed_at_unix_ms: null, running: 2,
    stale: false, github_stale: false, github_read_at_unix_ms: nowMs,
  };
}
