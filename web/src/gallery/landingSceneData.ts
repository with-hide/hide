// The product shots on the introduction site (site/index.html, the hero's
// tabs and the Hide AI section): the story of its one-night timeline, drawn by the
// shell's own components. Everything is the gallery's frozen sidebar fixture
// with its invented text replaced, never a second projection: the groups,
// counts and tree shape are what the core wrote for that fixture, and only the
// words and agent kinds a visitor reads are English and match the page. No
// name here is a person, a machine or a path of anyone's.

import type { PrBoard, TaskCardScope } from "../agentScope";
import type { NodeUsage } from "../generated/hided-ws";
import type { AgentRow, Checkout, Device, EditorDocumentSnapshot, EditorTabSnapshot, MemoryRow, PrState, ProjectMemory, PullRequest, SidebarLens, SnapshotRest, ViewDisplaySnapshot, Workspace } from "../snapshot";
import type { MobileState } from "../mobileSettings";
import { EMPTY_GALLERY_SCOPE } from "./agentScopes";
import { overviewWorld, type OverviewWorld } from "./overviewSceneData";
import { REFERENCE_FOLDS, sidebarScene } from "./sceneData";

/** A pull request or issue reference as the page writes it. */
const ref = (number: number) => `#${number}`;

/** What one agent row is called and which CLI runs it, by the fixture's pane id. */
type Words = { title: string; kind: string; detail?: string; pull?: [number, string] };

const AGENT_WORDS: Record<string, Words> = {
  a3: { title: `Approve ${ref(431)} and ${ref(436)}?`, kind: "codex", detail: "Both checked. Tests pass." },
  q1: { title: `${ref(433)}: CSV or XLSX for the export?`, kind: "antigravity", detail: "A product call the lead won't make for you." },
  q2: { title: "Fix the test that broke the export", kind: "opencode" },
  e1: { title: "Task detail API", kind: "codex" },
  tn1: { title: "Tidy the meeting notes", kind: "claude" },
  a1: { title: "Split the invoice export in three", kind: "claude", pull: [431, "Export invoices as CSV"] },
  a1c1: { title: "Export invoices as CSV", kind: "codex" },
  a1c2: { title: `Check ${ref(431)} and ${ref(436)}`, kind: "claude" },
  a1c3: { title: "Sort the export by date", kind: "codex", pull: [432, "Sort the export by date"] },
  a1g1: { title: "Find why the test broke", kind: "opencode" },
  a1g2: { title: "Add a test for sorted output", kind: "opencode" },
  a1c4: { title: "Pick the CSV column order", kind: "codex", pull: [434, "Pick the CSV column order"] },
  a1c5: { title: "Name the export file", kind: "codex" },
  a1c6: { title: "Add the export button", kind: "codex", pull: [436, "Add the export button"] },
  a1c7: { title: "Handle an empty invoice list", kind: "codex", pull: [435, "Handle an empty invoice list"] },
  a1c8: { title: "Link the export guide", kind: "codex" },
  a2: { title: "Draft the release notes", kind: "claude" },
  a2c1: { title: "Which changes to mention?", kind: "codex", detail: "Should the CSV export lead the notes?" },
  a2c2: { title: "Collect the merged pull requests", kind: "codex" },
};

const PROJECT_LABELS: Record<string, string> = {
  hide: "billing-app",
  sasu: "docs-site",
  "team-notes": "notes",
  "codex-daemon-apply-spec": "mobile-app",
  "sticky-notes": "scratch",
  investment: "pricing-page",
  "landing-page": "blog",
  "modakbul-site": "status-page",
  "old-prototype": "prototype",
  dotfiles: "dotfiles",
  "research-notes": "reading-list",
};

const BRANCHES: Record<string, string> = {
  "hide:155": "feat/433-export-format",
  "hide:154": "fix/export-test",
  "hide:electron": "feat/task-detail-api",
  "hide:ux": "docs/export-guide",
  "hide:fix": "fix/empty-invoice-list",
  "hide:legacy": "export-v1",
  "hide:inactive-0": "feat/export-csv",
};

const PURPOSES: Record<string, string> = {
  "hide:main": "Split the invoice export in three",
  "hide:155": `${ref(433)} CSV or XLSX for the export`,
  "hide:154": `${ref(434)} Fix the test that broke the export`,
  "hide:electron": "Task detail API",
  "hide:ux": "Export guide",
  "team-notes:folder": "Tidy the meeting notes",
  "sasu:main": "Docs search",
};

/**
 * The pull requests the story keeps: the frozen tree holds one on most of the
 * lead's children, which reads as clutter on a task row, so the page keeps one
 * or two per task. A pane listed with a state gains that pull request; a pane
 * the fixture gave one and this does not list loses it.
 */
const STORY_PULLS: Record<string, [number, string, PrState] | null> = {
  a1c4: null,
  a1c7: null,
  q2: [437, "Fix the test that broke the export", "merged"],
  e1: [438, "Task detail API", "pending"],
  a2: [439, "Release notes for the CSV export", "pending"],
};

/**
 * The pull requests only Hide AI linked: their sessions opened them outside
 * the window the link reader matches, and the label judged that the session
 * made them (docs/status-model.md, `created_prs`). With Hide AI off the rows
 * do not carry them.
 */
const LABEL_PULLS = new Set([438, 439]);

/**
 * The roots whose question only Hide AI found, in the turn's plain text, and
 * that the operator has read: on, they wait under Revisit; off, text alone
 * makes no question (docs/status-model.md, With and without Hide AI).
 */
const READ_ASKS = ["a2"];

/** One pull request a row holds the duty of, in the shape the fixture's own rows carry. */
function storyPull(row: AgentRow, [number, title, state]: [number, string, PrState]): AgentRow {
  const pull = {
    number, title, url: `https://example.invalid/billing-app/pull/${number}`, badge: state === "merged" ? "merged" as const : "open" as const,
    checks: state === "failed" ? "failed" as const : state === "pending" ? "pending" as const : "passing" as const,
    review: state === "pending" ? "review_required" as const : "approved" as const,
    head_branch: `agents/${number}`, closing_issues: [], live: true, duty: true, created: true, settled_at_unix_ms: state === "merged" ? 0 : null,
  };
  const request = { verb: "working" as const, verb_since_unix_ms: 0, line: undefined, request: null, later_by: null, ...row.request, pull_requests: [pull] };
  return { ...row, request, state: { ...row.state, pr: { count: 1, worst: state, pulls: [{ index: 0, state }] } } };
}

/** Keeps the story's pull requests on a row and drops the fixture's others, and with Hide AI off the ones only it linked. */
function trimPulls(row: AgentRow, ai: boolean): AgentRow {
  if (!(row.pane_id in STORY_PULLS)) return row;
  const kept = STORY_PULLS[row.pane_id];
  if (kept && (ai || !LABEL_PULLS.has(kept[0]))) return storyPull(row, kept);
  return { ...row, request: row.request ? { ...row.request, pull_requests: [] } : row.request, state: { ...row.state, pr: null } };
}

/** The roots the operator starred to come back to: the Tasks list's Revisit group (docs/status-model.md, `revisit`). */
const STARRED = ["tn1"];

/**
 * What Hide AI writes under each task on the Tasks list: what a waiting ask
 * wants, and the sentence of a finished, working or read turn
 * (docs/status-model.md, With and without Hide AI). Off, an ask keeps only its
 * native question's first line and every other sentence is gone.
 */
const TASK_LINES: Record<string, { ask?: string; line?: string; question?: true }> = {
  a3: { ask: "Both pass. Merge them?" },
  q1: { ask: "Only you can pick this" },
  q2: { line: "38 tests pass again" },
  a2: { line: "Lead with the CSV export?", question: true },
  tn1: { line: "Three decisions, sorted" },
  e1: { line: "Wiring the detail endpoint" },
};

/** A row with the sentences Hide AI wrote for it. */
function withLines(row: AgentRow): AgentRow {
  const words = TASK_LINES[row.pane_id];
  if (!words) return row;
  const ask = words.ask ? { verb: "answer" as const, what: words.ask, more: 0 } : row.state.ask;
  const session = words.line ? { ...row.state.session, line: words.line } : row.state.session;
  // A question only the label found leads with Question, as a native one does.
  return { ...row, ...(words.question ? { status_code: "question" as const } : {}), state: { ...row.state, ask, session } };
}

/** Replaces the words of one agent row and leaves everything the core decided as it was. */
function reword(fixture: AgentRow, ai: boolean): AgentRow {
  const row = trimPulls(fixture, ai);
  const words = AGENT_WORDS[row.pane_id];
  if (!words) return row;
  const request = row.request?.pull_requests?.length && words.pull
    ? { ...row.request, pull_requests: row.request.pull_requests.map((pull) => ({ ...pull, number: words.pull![0], title: words.pull![1], url: `https://example.invalid/billing-app/pull/${words.pull![0]}`, head_branch: `agents/${words.pull![0]}` })) }
    : row.request;
  const line = row.state.line && words.detail !== undefined ? { ...row.state.line, text: words.detail } : row.state.line;
  return {
    ...row,
    agent_kind: words.kind,
    identity_label: words.title,
    request,
    state: { ...row.state, line },
  };
}

function renameCheckout(checkout: Checkout): Checkout {
  const branch = BRANCHES[checkout.id];
  const purpose = PURPOSES[checkout.id];
  return {
    ...checkout,
    ...(branch ? { branch, label: branch } : {}),
    ...(purpose ? { purpose: { text: purpose, origin: "operator" as const } } : {}),
    worktree: branch && checkout.worktree ? { ...checkout.worktree, branch } : checkout.worktree,
    pull_request: checkout.pull_request ? { ...checkout.pull_request, title: purpose ?? checkout.pull_request.title } : checkout.pull_request,
  };
}

function renameProject(project: Workspace): Workspace {
  const label = PROJECT_LABELS[project.id];
  return { ...project, ...(label ? { label } : {}), pinned: false, checkouts: project.checkouts.map(renameCheckout) };
}

/** The window's projects and agents for every shot: one machine, the page's story. */
export function landingSidebar(nowMs: number, expanded: string[], width: number, ai = true): { rest: SnapshotRest; agents: AgentRow[] } {
  const base = sidebarScene("reference", { ...REFERENCE_FOLDS, expandedAgents: expanded }, nowMs);
  const navigator = base.rest.navigator!;
  // The one worktree name the core's frozen tiers and states carry is reworded where it sits, and the project's folder is the billing-app's.
  const reworded = <T,>(value: T): T => JSON.parse(JSON.stringify(value).replaceAll('"feat/ui"', '"feat/export-csv"').replace(/\/work\/hide(?=["./])/g, "/work/billing-app"));
  const rest: SnapshotRest = reworded({
    ...base.rest,
    navigator: {
      ...navigator,
      workspaces: navigator.workspaces?.map(renameProject),
      sleeping_sessions: navigator.sleeping_sessions?.map((session) => ({ ...session, identity_label: "Sync the scratch notes" })),
    },
    ui_state: {
      ...base.rest.ui_state,
      interface_language: "en",
      agent_logo_mode: "shown",
      sidebar_width: width,
      workspace_registrations: base.rest.ui_state?.workspace_registrations?.map((registration) => ({ ...registration, pinned: false })),
    },
  });
  const revisit = (row: AgentRow): AgentRow => {
    const starred = STARRED.includes(row.pane_id);
    const readAsk = ai && READ_ASKS.includes(row.pane_id);
    return starred || readAsk ? { ...row, revisit: { starred, read_ask: readAsk, since_unix_ms: nowMs - 3 * 3_600_000 } } : row;
  };
  // The lead waits on its workers whatever Hide AI does: a proven wait (docs/status-model.md, `wait`).
  const waits = (row: AgentRow): AgentRow => (row.pane_id === "a1" ? { ...row, wait: "children", state: { ...row.state, line: { text: "", mode: "waiting", tone: { kind: "working", read: true } } } } : row);
  return { rest, agents: reworded(base.agents.map((row) => waits(revisit(reword(row, ai)))).map((row) => (ai ? withLines(row) : row))) };
}

/** What each kind of agent is called when nothing better names it: the provider fallback of Task identity. */
const PROVIDERS: Record<string, string> = { claude: "Claude Code", codex: "Codex", opencode: "OpenCode", antigravity: "Antigravity" };

/** A native question's first line, as the row shows it without Hide AI (40 characters with the ellipsis). */
const NATIVE_QUESTIONS: Record<string, string> = {
  a3: "Merge both into main?",
  q1: "Which format should the export use, CS…",
  a2c1: "Should the CSV export lead the notes?",
};

/**
 * The attention shot with Agent summaries off (docs/status-model.md, With and
 * without Hide AI): no generated title, so a Claude Code session keeps its own
 * automatic title and every other agent is called by its provider; no generated
 * sentence, so only a native question keeps its first line. Groups and marks
 * are the same, since every demand in the story is a native question.
 */
export function landingPlainSidebar(nowMs: number, width: number): { rest: SnapshotRest; agents: AgentRow[] } {
  return withoutHideAi(landingSidebar(nowMs, [], width, false));
}

/** Any of the page's scenes with Agent summaries off, as `landingPlainSidebar` describes. */
export function withoutHideAi(scene: { rest: SnapshotRest; agents: AgentRow[] }): { rest: SnapshotRest; agents: AgentRow[] } {
  const agents = scene.agents.map((row) => {
    const title = row.agent_kind === "claude" ? row.identity_label : PROVIDERS[row.agent_kind ?? ""] ?? row.identity_label;
    const question = NATIVE_QUESTIONS[row.pane_id];
    const line = question && row.state.line ? { ...row.state.line, text: question } : row.state.line?.mode === "waiting" ? row.state.line : null;
    // Only the label judges a turn finished, so an unread stop keeps the plain dot.
    const mark = row.mark === "done" ? "unread" as const : row.mark;
    const ask = question && row.group === "needs_you" ? { verb: "answer" as const, what: question, more: 0 } : null;
    return { ...row, identity_label: title, mark, request: row.request ? { ...row.request, request: null } : row.request, state: { ...row.state, line, ask, session: { ...row.state.session, line: null } } };
  });
  const rest: SnapshotRest = { ...scene.rest, status: { ...scene.rest.status, background_ai: { provider: null, providers: [], unavailable_reason: null, enabled: false, chosen: false, agent_summary: false } } as SnapshotRest["status"] };
  return { rest, agents };
}

const NOW_SKEW = 90_000;

const MACHINE = (platform: string): NonNullable<Device["host"]> => ({ consent: "granted", helper_root: "~/.hide/host-helper", cli_dir: "~/.local/bin", contract: 3, bound_identity: "SHA256:example", granted_at_unix_ms: 1, state: "ready", message: null, platform, helper_path: "~/.hide/host-helper/current/hided" });
const KIT: NonNullable<Device["kit"]> = { unavailable: null, busy: false, components: [{ id: "cli", label: "hide command", state: "installed", why: null, location: "~/.local/bin/hide" }], agents: [], offers_reinstall: false, shares_account_with: null };

/** The sidebar of the attention shot with two more machines registered beside the core's, and one paired phone. */
export function landingDevices(nowMs: number, width: number): { rest: SnapshotRest; agents: AgentRow[]; mobile: MobileState } {
  const scene = landingSidebar(nowMs, [], width);
  const navigator = scene.rest.navigator!;
  const [own, ...others] = navigator.devices ?? [];
  const remote = (id: string, label: string, platform: string, extra: Partial<Device>): Device => ({ ...own!, agent_scope: EMPTY_GALLERY_SCOPE, id, label, kind: "remote", state: "ready", message: null, ssh_alias: id, agent_count: 0, host: MACHINE(platform), kit: KIT, machine_name: null, ...extra });
  const devices: Device[] = [
    { ...own!, machine_name: "Mac mini", host: MACHINE("macos-arm64"), kit: KIT },
    ...others,
    remote("macbook", "MacBook", "macos-arm64", { ssh_alias: null, dials_in: true }),
    remote("workbox", "workbox", "linux-x86_64", {}),
  ];
  const connected = (target: string) => ({ target_id: target, state: "connected", message: null, herdr_version: "0.9.3", session: null });
  const rest = {
    ...scene.rest,
    navigator: { ...navigator, devices },
    status: { ...scene.rest.status, remote: [connected("macbook"), connected("workbox")] },
  } as SnapshotRest;
  const mobile: MobileState = {
    enabled: true,
    exposure: "exposed",
    checklist: { installed: "ok", logged_in: "ok", https: "ok", host_name: "mini.example.ts.net" },
    download_url: "https://example.invalid/tailscale",
    admin_url: "https://example.invalid/admin",
    foreign_target: null,
    failure: null,
    url: "https://mini.example.ts.net/m/",
    qr: null,
    code_expires_at_ms: null,
    phones: [{ id: "phone-1", name: "Phone", last_seen_ms: nowMs - NOW_SKEW, connected: true, notifications: "on", revoke_at_ms: nowMs + 30 * 86_400_000 }],
    max_phones: 5,
    push_mode: "app_closed",
    now_ms: nowMs,
  };
  return { rest, agents: scene.agents, mobile };
}

/**
 * Which group of the hero's PRs tool each pull request state goes in, and the
 * agent the row names for it: the core's own states for the story's pull
 * requests, so the list agrees with the marks the sidebar draws for them.
 */
const HERO_PULL_GROUPS: Record<PullRequest["state"], PrBoard["rows"][number]["group"] | null> = { mergeable: "needs_you", failed: "working", pending: "working", merged: "merged", draft: null, closed: null };
const HERO_PULL_AGENTS: Record<number, string> = { 431: "a3", 436: "a3", 432: "a1g1" };

/** The lane's width in the workspace shots, narrow enough that the two terminals beside it read whole. */
const HERO_LANE_WIDTH = 360;

/** The two panes the hero shows side by side, of the billing-app main checkout's one Agent tab. */
export const HERO_PANES = ["a1", "a3"] as const;


/**
 * The hero: the attention shot's window with the billing-app main checkout in
 * front, its lead and the agent asking to merge side by side, and the Project
 * tool pinned beside them with the project's 현황. The tasks and prs shots are
 * the same window with the sidebar on another list (`lens`).
 */
export function landingWorkspace(nowMs: number, width: number, expanded: string[] = [], lens: SidebarLens = "projects", ai = true): { rest: SnapshotRest; agents: AgentRow[] } {
  const scene = landingSidebar(nowMs, expanded, width, ai);
  const navigator = scene.rest.navigator!;
  const project = navigator.workspaces!.find((row) => row.id === "hide")!;
  const main = project.checkouts[0]!;
  const tab = main.tabs[0]!;
  const story = scene.agents.flatMap((row) => (row.request?.pull_requests ?? []).map((pull, index) => ({ pull, row, state: row.state.pr?.pulls.find((one) => one.index === index)?.state })));
  const listed = story.flatMap(({ pull, row, state }) => {
    const group = state ? HERO_PULL_GROUPS[state] : null;
    return state && group ? [{ pull, state, group, agent: HERO_PULL_AGENTS[pull.number] ?? row.pane_id }] : [];
  });
  const pulls: PullRequest[] = listed.map(({ pull, state }) => ({
    number: pull.number, title: pull.title, url: pull.url, badge: pull.badge, review: pull.review ?? null, is_draft: false, checks: pull.checks,
    head_branch: pull.head_branch, base_branch: "main", closing_issues: [], state, created_at_unix_ms: nowMs - 40 * 60_000, updated_at_unix_ms: nowMs - 2 * 60_000,
    ...(state === "merged" ? { merged_at_unix_ms: nowMs - 9 * 60_000 } : {}),
  }));
  const groups = (["needs_you", "working", "merged"] as const).map((group) => ({ group, numbers: listed.filter((item) => item.group === group).map((item) => item.pull.number) }));
  const prs: PrBoard = {
    rows: listed.map(({ pull, state, group, agent }) => ({
      number: pull.number, group, fold: null,
      reason: group === "needs_you" ? { kind: "merge" } : group === "merged" ? { kind: "merged", since_unix_ms: nowMs - 9 * 60_000 } : state === "failed" ? { kind: "failed", since_unix_ms: nowMs - 12 * 60_000 } : { kind: "working", since_unix_ms: nowMs - 6 * 60_000 },
      agents: [{ pane_id: agent, occurrence: 0 }], agent_pane: agent, checkout_id: main.id, checkout_ids: [main.id], issue: null,
    })),
    groups,
    needs_you: groups[0]!.numbers.length,
  };
  const github = { failure_category: null, available: true, loading: false, stale: false, last_success_at_unix_ms: nowMs - 60_000, unavailable_reason: null };
  // A pane's header reads the title its agent row carries.
  const titled = { ...tab, panes: tab.panes.map((pane) => ({ ...pane, identity_label: scene.agents.find((row) => row.pane_id === pane.id)?.identity_label ?? pane.identity_label })) };
  const status = { ...HERO_STATUS, git: { ...HERO_STATUS.git, dots: project.checkouts.slice(0, 6).map((checkout, index) => ({ checkout_id: checkout.id, state: HERO_DOTS[index % HERO_DOTS.length]! })) } };
  const front: Workspace = { ...project, is_git: true, pull_requests: pulls, agent_scope: { ...project.agent_scope, prs, status }, checkouts: project.checkouts.map((checkout) => (checkout === main ? { ...checkout, github, tabs: [titled] } : checkout)) };
  const workspaces = navigator.workspaces!.map((row) => (row === project ? front : row));
  const devices = navigator.devices?.map((device, index) => (index === 0 ? { ...device, agent_scope: { ...device.agent_scope, tasks: landingTasks(scene.agents, workspaces, nowMs, ai) } } : device));
  const rest: SnapshotRest = {
    ...scene.rest,
    ui_state: { ...scene.rest.ui_state!, sidebar_lens: lens },
    navigator: { ...navigator, devices, focused_workspace_id: project.id, focused_checkout_id: main.id, workspaces },
    pane_layouts: [{ workspace_id: project.id, tab_id: tab.id!, focused_pane_id: HERO_PANES[1], zoomed: false, root: { type: "split", direction: "right", ratio: 0.5, first: { type: "pane", pane_id: HERO_PANES[0] }, second: { type: "pane", pane_id: HERO_PANES[1] } } }],
    workspace_view: {
      agent_layout: { root: { area: { id: "g1", active: tab.id, displays: [{ id: tab.id! }] } }, active_area: "g1", canvases: {}, limits: { areas: 4, depth: 3, displays: 64 }, display_count: 1 },
      device_id: "local", path: main.path, open: true, documents: false, tool: "project", pinned: true, document_width: HERO_LANE_WIDTH, detail_width: null, explorer_width: null, panel_width: HERO_LANE_WIDTH, lane_called: 0, lane_calls: 0, lane_called_side: "view",
      layout: { root: { area: { id: "v1", active: null, displays: [] } }, active_area: "v1", limits: { areas: 4, depth: 3, displays: 32 }, display_count: 0 },
    },
    terminal: { headers: { a1: { working: true, notice: null, band: null } } },
  };
  return { rest, agents: scene.agents };
}


/** The browser shot's pane: the Codex worker that added the export button, alone in its tab. */
export const BROWSER_PANE = "a1c6";
/** The page the agent opened, as its View id: the name `hide browser` commands take. */
export const BROWSER_DISPLAY = "browser-1";
const BROWSER_URL = "http://localhost:5173/invoices";
/** The spec the agent reads before it tries the page: a PDF in the checkout, opened with `hide file open`. */
export const SPEC_PATH = "docs/export-spec.pdf";
/** The spec's View id. */
const SPEC_DISPLAY = "d-spec";

/** How far the browser story got: the spec open, then the page open, then the button pressed. */
export type BrowserStep = 1 | 2 | 3;

/** The export spec as a small PDF the shell's own viewer draws: two pages of plain text and one table. */
export function landingSpecPdf(): Uint8Array {
  const pages = [
    ["EXPORT SPEC - BILLING", "Invoice export", "Export CSV downloads every invoice of the month shown.", "Columns, in order: number, customer, issued_at, total.", "Rows sorted by issue date, newest first.", "Totals in USD with two decimals; no currency sign."],
    ["EXPORT SPEC - BILLING", "Checks before merge", "The button sits in the page header, right aligned.", "An empty month exports the header row only.", "The file is named invoices-<month>.csv.", "October has 128 invoices: the export has 129 lines."],
  ];
  const objects: string[] = [];
  const add = (body: string) => objects.push(body);
  add("<< /Type /Catalog /Pages 2 0 R >>");
  add(`<< /Type /Pages /Kids [${pages.map((_, index) => `${4 + index * 2} 0 R`).join(" ")}] /Count ${pages.length} >>`);
  add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
  pages.forEach(([kicker, title, ...body]) => {
    const text = body.map((line, index) => `0.25 0.25 0.25 rg BT /F1 12 Tf 48 ${300 - index * 26} Td (${line}) Tj ET`).join("\n");
    const rule = "0.85 0.85 0.85 rg 48 330 504 1 re f";
    const stream = `1 1 1 rg 0 0 600 440 re f\n0.33 0.4 0.95 rg BT /F1 9 Tf 48 392 Td (${kicker}) Tj ET\n0.1 0.1 0.1 rg BT /F1 24 Tf 48 358 Td (${title}) Tj ET\n${rule}\n${text}`;
    add(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 440] /Resources << /Font << /F1 3 0 R >> >> /Contents ${objects.length + 2} 0 R >>`);
    add(`<< /Length ${stream.length} >>\nstream\n${stream}\nendstream`);
  });
  let text = "%PDF-1.4\n";
  const offsets = objects.map((body, index) => {
    const at = text.length;
    text += `${index + 1} 0 obj\n${body}\nendobj\n`;
    return at;
  });
  const xref = text.length;
  text += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.map((at) => `${String(at).padStart(10, "0")} 00000 n \n`).join("")}`;
  text += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return new TextEncoder().encode(text);
}

/**
 * The browser shots: the Codex worker that added the export button, alone in
 * its pane, and beside it the View area it fills step by step: the spec it
 * opened with \`hide file open\`, then the invoices page it opened with
 * \`hide browser open\` and pressed with \`hide browser click\`. The page itself
 * is the desktop host's to draw, so the scene hands the shell a still of it
 * (`landing-invoices-<theme>.svg`).
 */
export function landingBrowser(nowMs: number, width: number, step: BrowserStep): { rest: SnapshotRest; agents: AgentRow[]; editor: { tabs: EditorTabSnapshot[]; active_tab_id: string | null }; documents: Record<string, EditorDocumentSnapshot> } {
  const scene = landingWorkspace(nowMs, width);
  const navigator = scene.rest.navigator!;
  const project = navigator.workspaces!.find((row) => row.id === "hide")!;
  const main = project.checkouts[0]!;
  const tab = main.tabs[0]!;
  const path = `${main.path}/${SPEC_PATH}`;
  const file: EditorTabSnapshot = { id: "t-spec", workspace_id: project.id, checkout_id: main.id, path, label: "export-spec.pdf", kind: "file", diff_committed: null, markdown_live: false, wrap: false, dirty: false, preview: false };
  const spec: ViewDisplaySnapshot = { id: SPEC_DISPLAY, tab_id: file.id, path, label: file.label, kind: "file", committed: null, preview: false, state: "open", reason: null };
  const page: ViewDisplaySnapshot = { id: BROWSER_DISPLAY, tab_id: null, path: "", label: "Invoices", kind: "browser", committed: null, preview: false, state: "open", reason: null, url: BROWSER_URL };
  const displays = step === 1 ? [spec] : [spec, page];
  const document: EditorDocumentSnapshot = { path, language: "pdf", document_kind: "pdf", contents_utf8: "", revision: "sha256:0", dirty: false, readonly_reason: null, conflict: null, save: null };
  const rest: SnapshotRest = {
    ...scene.rest,
    pane_layouts: [{ workspace_id: project.id, tab_id: tab.id!, focused_pane_id: BROWSER_PANE, zoomed: false, root: { type: "pane", pane_id: BROWSER_PANE } }],
    workspace_view: {
      ...scene.rest.workspace_view!,
      documents: true, tool: null, pinned: true, document_width: 820,
      layout: { root: { area: { id: "v1", active: step === 1 ? SPEC_DISPLAY : BROWSER_DISPLAY, displays } }, active_area: "v1", limits: { areas: 4, depth: 3, displays: 32 }, display_count: displays.length },
    },
    terminal: { headers: { [BROWSER_PANE]: { working: step < 3, notice: null, band: null } } },
  };
  return { rest, agents: scene.agents, editor: { tabs: [file], active_tab_id: null }, documents: { [file.id]: document } };
}

const MINUTE = 60_000;
const DAY = 1440 * MINUTE;

/** The billing-app's 현황 as the core would derive it for the story: two pull requests to merge, and the worktrees' marks. */
const HERO_STATUS = {
  todo: [{ kind: "pr" as const, number: 431 }, { kind: "pr" as const, number: 436 }, { kind: "ask" as const, pane_id: "q1" }],
  todo_total: 3,
  todo_agents: 1,
  prs: { open: 5, mergeable: 2, failed: 1, review: 0, checking: 2 },
  git: { worktrees: 7, behind: 0, changed: 2, unpushed: 1, removable: 1 },
};
const HERO_DOTS = ["clean", "changed", "changed", "unpushed", "clean", "removable"] as const;

/** Which state each of the story's lineage roots is in on the 태스크 list, and how far its pull request got. */
const TASK_STATES: Record<string, { state: TaskCardScope["state"]; stage?: TaskCardScope["stage"] }> = {
  a3: { state: "needs_you", stage: 3 },
  q1: { state: "needs_you", stage: 1 },
  a1: { state: "waiting", stage: 2 },
  q2: { state: "done", stage: 2 },
  e1: { state: "working", stage: 1 },
  a2: { state: "revisit" },
  tn1: { state: "revisit" },
};

/** Where a root stands with Hide AI off instead: the release notes' question was only in plain text, so it is not waited on. */
const TASK_STATES_WITHOUT_AI: typeof TASK_STATES = { a2: { state: "working" } };

/** The device's tasks as the core lists them: each of the story's lineage roots with everything it started, in the checkouts it works in. */
export function landingTasks(agents: AgentRow[], workspaces: Workspace[], nowMs: number, ai = true): { cards: TaskCardScope[]; needs_you: number; recency: string } {
  const where = new Map(workspaces.flatMap((project) => project.checkouts.flatMap((checkout) => checkout.tabs.flatMap((tab) => tab.panes.map((pane) => [pane.id, checkout.id] as const)))));
  const descendants = (pane: string): string[] => agents.filter((row) => row.lineage_parent_pane_id === pane).flatMap((row) => [row.pane_id, ...descendants(row.pane_id)]);
  const states = ai ? TASK_STATES : { ...TASK_STATES, ...TASK_STATES_WITHOUT_AI };
  const cards = Object.entries(states).flatMap(([root, { state, stage }], index) => {
    const checkout = where.get(root);
    if (!checkout) return [];
    const members = [root, ...descendants(root)];
    const checkouts = [...new Set(members.map((pane) => where.get(pane)).filter((id): id is string => id !== undefined))];
    return [{ root, state, members, checkouts, recency: new Date(nowMs - index * 4 * MINUTE).toISOString(), ...(stage ? { stage } : {}) }];
  });
  return { cards, needs_you: cards.filter((card) => card.state === "needs_you").length, recency: cards[0]?.recency ?? "" };
}

/** The billing-app's last seven days at API prices, for the Project tool's cost tile. */
export function landingUsage(nowMs: number): NodeUsage {
  const amounts = [6.1, 9.4, 4.2, 11.8, 8.3, 13.6, 7.9];
  const days = amounts.map((usd, index) => ({ date: new Date(nowMs - (6 - index) * DAY).toISOString().slice(0, 10), micro_usd: Math.round(usd * 1_000_000) }));
  const total = days.reduce((sum, day) => sum + day.micro_usd, 0);
  const share = (part: number) => Math.round(total * part);
  return {
    node: "local", agents: [], history: null,
    projects: [{
      workspace_id: "hide", micro_usd: total, days,
      models: [{ agent: "claude", model: "model-a", micro_usd: share(0.58) }, { agent: "codex", model: "model-c", micro_usd: share(0.34) }, { agent: "opencode", model: null, micro_usd: share(0.08) }],
      sessions: [{ agent: "claude", session_id: "session-a1", micro_usd: share(0.31) }],
      agents: [{ agent: "claude", micro_usd: share(0.58) }, { agent: "codex", micro_usd: share(0.34) }, { agent: "opencode", micro_usd: share(0.08) }],
      unpriced_agents: [], partial: false, waiting: false, stale: false, limited: false,
    }],
  };
}

/** The Overview's agents in the page's words: who they are, what they ask and what they are doing now, by pane id. */
const MAP_WORDS: Record<string, { title: string; now?: string; ask?: string; kind?: string }> = {
  "h-a3": { title: `Approve ${ref(431)} and ${ref(436)}?`, ask: "Both pass their checks. Merge them into main?", kind: "codex" },
  "h-a1": { title: "Split the invoice export in three", now: "Waiting on 3 workers" },
  "h-a1c1": { title: "Export invoices as CSV", now: "Writing the rows" },
  "h-a1c2": { title: `Check ${ref(431)} and ${ref(436)}`, kind: "antigravity" },
  "h-a1c3": { title: "Sort the export by date", now: "Running the tests" },
  "h-a1c4": { title: "Pick the CSV column order" },
  "h-a1g1": { title: "Add a test for sorted output", now: "Writing the test", kind: "opencode" },
  "h-e1": { title: "Task detail API", now: "Wiring the detail route" },
  "h-q2": { title: "Search the invoices", now: "Pull request merged" },
  "h-r1": { title: "Draft the release notes", now: "Draft done" },
  "h-r2": { title: "Update the dependencies", now: "List done" },
  "b-1": { title: "Retry failed payments", ask: "Run the migration on the production database?" },
  "b-2": { title: "Verify webhook signatures", now: "Fixing the failing check" },
  "s-1": { title: "Polish the pricing page", now: "Three drafts done", kind: "antigravity" },
};
const MAP_PROJECTS: Record<string, string> = { hide: "billing-app", "billing-api": "payments-api", site: "docs-site", "team-notes": "notes" };

/** The Overview's work graph over the page's story: the reference world with every word a visitor reads in English. */
export function landingMap(nowMs: number): OverviewWorld {
  // The lead's worktree is the story's, not the reference world's.
  const world: OverviewWorld = JSON.parse(JSON.stringify(overviewWorld(nowMs)).replaceAll("feat/sidebar-readable", "feat/export-split"));
  for (const row of world.agents) {
    const words = MAP_WORDS[row.pane_id];
    if (!words) continue;
    row.identity_label = words.title;
    if (words.kind) row.agent_kind = words.kind;
    if (words.now && row.state.line) row.state = { ...row.state, line: { ...row.state.line, text: words.now } };
    if (words.ask && row.state.ask) row.state = { ...row.state, ask: { ...row.state.ask, what: words.ask } };
  }
  const titles = new Map(world.agents.map((row) => [row.pane_id, row.identity_label]));
  const navigator = world.rest.navigator!;
  navigator.workspaces = navigator.workspaces!.map((project) => ({
    ...project,
    label: MAP_PROJECTS[project.id] ?? project.label,
    checkouts: project.checkouts.map((checkout) => ({ ...checkout, tabs: checkout.tabs.map((tab) => ({ ...tab, panes: tab.panes.map((pane) => ({ ...pane, identity_label: titles.get(pane.id) ?? pane.identity_label })) })) })),
  }));
  world.rest.ui_state = { ...world.rest.ui_state!, interface_language: "en", agent_logo_mode: "shown", workspace_registrations: world.rest.ui_state!.workspace_registrations?.map((row) => ({ ...row, label: MAP_PROJECTS[row.id] ?? row.label })) };
  world.rest.issue_work = { ...world.rest.issue_work!, backlog: [] };
  return world;
}

/**
 * The memory shot's cards (the Project tool's Memory tab): what the story's sessions decided about the
 * billing-app, in the kinds and states the core gives them. One rule is asked
 * about for the docs, an agent is writing another into them, and one already
 * lives in the repository's CONTRIBUTING.md.
 */
export function landingMemory(nowMs: number, workspaceId: string): ProjectMemory {
  const card = (number: number, title: string, kind: MemoryRow["kind"], given: number, pulled: number, human: number, ago: number, promotion: MemoryRow["promotion"] = null): MemoryRow => ({
    number, title, kind, provided_session_count: given, pull_count: pulled, human_source_count: human, updated_at_unix_ms: nowMs - ago, verifying: false, promotion,
  });
  const rows = [
    card(13, "Use pnpm, never npm", "rule", 6, 4, 2, 40 * MINUTE),
    card(14, "Exports stay in UTC; the invoice PDF shows local time", "decision", 4, 2, 1, 2 * 60 * MINUTE),
    card(16, "An empty invoice list exports a header row, not an empty file", "decision", 2, 1, 1, 25 * MINUTE),
    card(11, "Above 10,000 rows the export times out; page it", "lesson", 3, 2, 1, DAY),
    card(10, "Release notes wait for a person to read them", "rule", 5, 3, 2, 2 * DAY, { state: "running", agent: { name: "memory-promote-m10", pane_id: "w1:p9" }, pr_number: null, target_path: null, github_failure: null }),
    card(9, "Run the export tests before opening a pull request", "rule", 8, 6, 2, 3 * DAY, { state: "merged", agent: null, pr_number: 474, target_path: "CONTRIBUTING.md", github_failure: null }),
    card(7, "Invoices live in the billing schema, not public", "fact", 7, 5, 0, 4 * DAY),
  ];
  return {
    workspace_id: workspaceId, unavailable: null, loading: false, enabled: true, capacity_reached: false, evicted_cards: 0, evicted_through_unix_ms: null,
    rows,
    questions: [{ kind: "promote", number: 13, title: rows[0]!.title, human_session_count: 2, pulled_session_count: 4 }],
    analysis: "idle", consent_required: false, usable_cards: rows.length, checking_card: null, check_resume_at_unix_ms: null, forgotten: null, pending_action: null,
    expanded: null,
    search: null,
  };
}

const CLAUDE = "\x1b[38;2;215;119;87m";
const CODEX = "\x1b[36m";
const DIM = "\x1b[90m";
const BOLD = "\x1b[1m";
const GREEN = "\x1b[32m";
const RESET = "\x1b[0m";
const lines = (...rows: string[]) => rows.join("\r\n");

/**
 * What each hero pane's terminal shows: an agent CLI's own screen, as its
 * program would draw it, over the page's story. `RULE` is drawn across the
 * pane's width when it is written; the lead's screen starts two rows down,
 * under the row of its child that asks.
 */
export const RULE = "{rule}";
/** What the Codex worker's terminal shows at each step of the browser story. */
export function browserTerminal(step: BrowserStep): string {
  const head = [
    "",
    `${DIM}╭──────────────────────────────╮${RESET}`,
    `${DIM}│${RESET} ${BOLD}>_ Codex${RESET}                     ${DIM}│${RESET}`,
    `${DIM}│${RESET} ${DIM}~/billing-app · main${RESET}         ${DIM}│${RESET}`,
    `${DIM}╰──────────────────────────────╯${RESET}`,
    "",
    `${CODEX}›${RESET} Add the export button and check it against the spec`,
    "",
    `${DIM}•${RESET} ${BOLD}Edited${RESET} src/export/button.tsx ${GREEN}(+24${RESET} -3${GREEN})${RESET}`,
    "",
    `${DIM}•${RESET} ${BOLD}Ran${RESET} hide file open ${SPEC_PATH}`,
    `  ${DIM}└${RESET} 2 pages: columns, order, file name`,
  ];
  const page = ["", `${DIM}•${RESET} ${BOLD}Ran${RESET} hide browser open localhost:5173/invoices`, `  ${DIM}└${RESET} loaded · Invoices`];
  const click = [
    "",
    `${DIM}•${RESET} ${BOLD}Ran${RESET} hide browser click ${BROWSER_DISPLAY} --text "Export CSV"`,
    `  ${DIM}└${RESET} downloaded invoices-october.csv · 129 lines`,
    "",
    `${DIM}•${RESET} The export matches the spec: 128 invoices,`,
    "  newest first. The spec and the page are beside you.",
  ];
  return lines(...head, ...(step >= 2 ? page : []), ...(step >= 3 ? click : []));
}

export const HERO_TERMINALS: Record<(typeof HERO_PANES)[number], string> = {
  a1: lines(
    "",
    "",
    `${CLAUDE}✻${RESET} ${BOLD}Claude Code${RESET}`,
    `  ${DIM}~/billing-app · main${RESET}`,
    "",
    `${DIM}>${RESET} Split the invoice export in three`,
    "",
    "● Three parts, one Codex worker each.",
    "",
    `● ${BOLD}Bash${RESET}(hide agent spawn …)`,
    `  ${DIM}⎿${RESET}  Export invoices as CSV`,
    `● ${BOLD}Bash${RESET}(hide agent spawn …)`,
    `  ${DIM}⎿${RESET}  Sort the export by date`,
    `● ${BOLD}Bash${RESET}(hide agent spawn …)`,
    `  ${DIM}⎿${RESET}  Add the export button`,
    "",
    "● The workers report back to me, and I",
    "  check each pull request before you.",
    "",
    `${CLAUDE}✻ Waiting on 3 workers… ${DIM}(6m 12s)${RESET}`,
    "",
    `${DIM}${RULE}${RESET}`,
    `${DIM}>${RESET} `,
    `${DIM}${RULE}${RESET}`,
    `  ${DIM}⏵⏵ accept edits on${RESET}`,
  ),
  a3: lines(
    "",
    `${DIM}╭──────────────────────────────╮${RESET}`,
    `${DIM}│${RESET} ${BOLD}>_ Codex${RESET}                     ${DIM}│${RESET}`,
    `${DIM}│${RESET} ${DIM}~/billing-app · main${RESET}         ${DIM}│${RESET}`,
    `${DIM}╰──────────────────────────────╯${RESET}`,
    "",
    `${CODEX}›${RESET} Check ${ref(431)} and ${ref(436)}, then ask me`,
    "",
    `${DIM}•${RESET} ${BOLD}Ran${RESET} pnpm test export`,
    `  ${DIM}└${RESET} ${GREEN}38 passed${RESET}`,
    "",
    `${DIM}•${RESET} Both pull requests are ready:`,
    `  ${ref(431)} Export invoices as CSV`,
    `  ${ref(436)} Add the export button`,
    "",
    `${DIM}${RULE}${RESET}`,
    ` ${BOLD}Merge both into main?${RESET}`,
    "",
    ` ${CODEX}› 1. Yes, merge both${RESET}`,
    `   2. Only ${ref(431)}`,
    "   3. Not yet",
  ),
};
