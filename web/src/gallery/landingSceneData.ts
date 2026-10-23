// The product shots on the introduction site (site/index.html, the hero's
// tabs and the Hide AI section): the story of its one-night timeline, drawn by the
// shell's own components. Everything is the gallery's frozen sidebar fixture
// with its invented text replaced, never a second projection: the groups,
// counts and tree shape are what the core wrote for that fixture, and only the
// words and agent kinds a visitor reads are English and match the page. No
// name here is a person, a machine or a path of anyone's.

import type { PrBoard, TaskCardScope } from "../agentScope";
import type { NodeUsage } from "../generated/hided-ws";
import type { AgentRow, Checkout, Device, PullRequest, SidebarLens, SnapshotRest, Workspace } from "../snapshot";
import type { MobileState } from "../mobileSettings";
import { EMPTY_GALLERY_SCOPE } from "./agentScopes";
import { overviewWorld, type OverviewWorld } from "./overviewSceneData";
import { REFERENCE_FOLDS, sidebarScene } from "./sceneData";

/** A pull request or issue reference as the page writes it. */
const ref = (number: number) => `#${number}`;

/** What one agent row is called and which CLI runs it, by the fixture's pane id. */
type Words = { title: string; kind: string; detail?: string; pull?: [number, string] };

const AGENT_WORDS: Record<string, Words> = {
  a3: { title: `Approve ${ref(431)} and ${ref(436)}?`, kind: "claude", detail: "Both checked by Claude Code. Tests pass." },
  q1: { title: `${ref(433)}: CSV or XLSX for the export?`, kind: "claude", detail: "A product call the lead won't make for you." },
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

/** Replaces the words of one agent row and leaves everything the core decided as it was. */
function reword(row: AgentRow): AgentRow {
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
export function landingSidebar(nowMs: number, expanded: string[], width: number): { rest: SnapshotRest; agents: AgentRow[] } {
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
  return { rest, agents: reworded(base.agents.map(reword)) };
}

const NOW_SKEW = 90_000;

const MACHINE = (platform: string): NonNullable<Device["host"]> => ({ consent: "granted", helper_root: "~/.hide/host-helper", cli_dir: "~/.local/bin", contract: 3, bound_identity: "SHA256:example", granted_at_unix_ms: 1, state: "ready", message: null, platform, helper_path: "~/.hide/host-helper/current/hided" });
const KIT: NonNullable<Device["kit"]> = { unavailable: null, busy: false, components: [{ id: "cli", label: "hide command", state: "installed", reason: null, location: "~/.local/bin/hide" }], agents: [], offers_reinstall: false, shares_account_with: null };

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
export function landingWorkspace(nowMs: number, width: number, expanded: string[] = [], lens: SidebarLens = "projects"): { rest: SnapshotRest; agents: AgentRow[] } {
  const scene = landingSidebar(nowMs, expanded, width);
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
  const devices = navigator.devices?.map((device, index) => (index === 0 ? { ...device, agent_scope: { ...device.agent_scope, tasks: landingTasks(scene.agents, workspaces, nowMs) } } : device));
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

const MINUTE = 60_000;
const DAY = 1440 * MINUTE;

/** The billing-app's 현황 as the core would derive it for the story: two pull requests to merge, and the worktrees' marks. */
const HERO_STATUS = {
  todo: [{ kind: "pr" as const, number: 431 }, { kind: "pr" as const, number: 436 }, { kind: "ask" as const, pane_id: "q1" }],
  todo_total: 3,
  todo_agents: 1,
  prs: { open: 4, mergeable: 2, failed: 1, review: 0, checking: 1 },
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
  a2: { state: "working" },
  tn1: { state: "resting" },
};

/** The device's tasks as the core lists them: each of the story's lineage roots with everything it started, in the checkouts it works in. */
export function landingTasks(agents: AgentRow[], workspaces: Workspace[], nowMs: number): { cards: TaskCardScope[]; needs_you: number; recency: string } {
  const where = new Map(workspaces.flatMap((project) => project.checkouts.flatMap((checkout) => checkout.tabs.flatMap((tab) => tab.panes.map((pane) => [pane.id, checkout.id] as const)))));
  const descendants = (pane: string): string[] => agents.filter((row) => row.lineage_parent_pane_id === pane).flatMap((row) => [row.pane_id, ...descendants(row.pane_id)]);
  const cards = Object.entries(TASK_STATES).flatMap(([root, { state, stage }], index) => {
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
  "h-a3": { title: `Approve ${ref(431)} and ${ref(436)}?`, ask: "Both pass their checks. Merge them into main?" },
  "h-a1": { title: "Split the invoice export in three", now: "Waiting on 3 workers" },
  "h-a1c1": { title: "Export invoices as CSV", now: "Writing the rows" },
  "h-a1c2": { title: `Check ${ref(431)} and ${ref(436)}` },
  "h-a1c3": { title: "Sort the export by date", now: "Running the tests" },
  "h-a1c4": { title: "Pick the CSV column order" },
  "h-a1g1": { title: "Add a test for sorted output", now: "Writing the test", kind: "opencode" },
  "h-e1": { title: "Task detail API", now: "Wiring the detail route" },
  "h-q2": { title: "Search the invoices", now: "Pull request merged" },
  "h-r1": { title: "Draft the release notes", now: "Draft done" },
  "h-r2": { title: "Update the dependencies", now: "List done" },
  "b-1": { title: "Retry failed payments", ask: "Run the migration on the production database?" },
  "b-2": { title: "Verify webhook signatures", now: "Fixing the failing check" },
  "s-1": { title: "Polish the pricing page", now: "Three drafts done" },
};
const MAP_PROJECTS: Record<string, string> = { hide: "billing-app", "billing-api": "payments-api", site: "docs-site", "team-notes": "notes" };

/** The Overview's work graph over the page's story: the reference world with every word a visitor reads in English. */
export function landingMap(nowMs: number): OverviewWorld {
  const world = overviewWorld(nowMs);
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

const CLAUDE = "\x1b[38;2;215;119;87m";
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
    `${CLAUDE}✻${RESET} ${BOLD}Claude Code${RESET}`,
    `  ${DIM}~/billing-app · main${RESET}`,
    "",
    `${DIM}>${RESET} Check ${ref(431)} and ${ref(436)}, then ask me`,
    "",
    `● ${BOLD}Bash${RESET}(pnpm test export)`,
    `  ${DIM}⎿${RESET}  ${GREEN}38 passed${RESET}`,
    "",
    "● Both pull requests are ready:",
    `  ${ref(431)} Export invoices as CSV`,
    `  ${ref(436)} Add the export button`,
    "",
    `${DIM}${RULE}${RESET}`,
    ` ${BOLD}Merge both into main?${RESET}`,
    "",
    ` ${CLAUDE}❯ 1. Yes, merge both${RESET}`,
    `   2. Only ${ref(431)}`,
    "   3. Not yet",
  ),
};
