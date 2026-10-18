// The product shots on the introduction site (site/index.html, the hero's
// tabs and the Hide AI section): the story of its one-night timeline, drawn by the
// shell's own components. Everything is the gallery's frozen sidebar fixture
// with its invented text replaced, never a second projection: the groups,
// counts and tree shape are what the core wrote for that fixture, and only the
// words and agent kinds a visitor reads are English and match the page. No
// name here is a person, a machine or a path of anyone's.

import type { PrBoard } from "../agentScope";
import type { AgentRow, Checkout, Device, MemoryRow, ProjectMemory, PullRequest, SnapshotRest, Workspace } from "../snapshot";
import type { MobileState } from "../mobileSettings";
import { EMPTY_GALLERY_SCOPE } from "./agentScopes";
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
  // The one worktree name the core's frozen tiers and states carry is reworded where it sits.
  const reworded = <T,>(value: T): T => JSON.parse(JSON.stringify(value).replaceAll('"feat/ui"', '"feat/export-csv"'));
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
 * front, its lead and the agent asking to merge side by side, and the PRs tool
 * pinned beside them listing what those agents opened. The agents shot is the
 * same window with the lead's children unfolded in the tree (`expanded`), and
 * the memory shot pins the Memory tool instead of the PRs tool.
 */
export function landingWorkspace(nowMs: number, width: number, tool: "prs" | "memory" = "prs", expanded: string[] = []): { rest: SnapshotRest; agents: AgentRow[] } {
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
  const front: Workspace = { ...project, is_git: true, pull_requests: pulls, agent_scope: { ...project.agent_scope, prs }, checkouts: project.checkouts.map((checkout) => (checkout === main ? { ...checkout, github, tabs: [titled] } : checkout)) };
  const rest: SnapshotRest = {
    ...scene.rest,
    navigator: { ...navigator, focused_workspace_id: project.id, focused_checkout_id: main.id, workspaces: navigator.workspaces!.map((row) => (row === project ? front : row)) },
    pane_layouts: [{ workspace_id: project.id, tab_id: tab.id!, focused_pane_id: HERO_PANES[1], zoomed: false, root: { type: "split", direction: "right", ratio: 0.5, first: { type: "pane", pane_id: HERO_PANES[0] }, second: { type: "pane", pane_id: HERO_PANES[1] } } }],
    workspace_view: {
      agent_layout: { root: { area: { id: "g1", active: tab.id, displays: [{ id: tab.id! }] } }, active_area: "g1", canvases: {}, limits: { areas: 4, depth: 3, displays: 64 }, display_count: 1 },
      device_id: "local", path: main.path, tool, last_tool: tool, prs: { only_checkout: false }, pinned: true, tree: false, lane_width: HERO_LANE_WIDTH, panel_width: HERO_LANE_WIDTH, tree_width: null, lane_called: 0, lane_calls: 0,
      layout: { root: { area: { id: "v1", active: null, displays: [] } }, active_area: "v1", limits: { areas: 4, depth: 3, displays: 32 }, display_count: 0 },
    },
    terminal: { headers: { a1: { working: true, band: null } } },
  };
  return { rest, agents: scene.agents };
}

const MINUTE = 60_000;
const DAY = 1440 * MINUTE;

/**
 * The memory shot's cards: what the story's sessions decided about the
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
    workspace_id: workspaceId, unavailable: null, loading: false, enabled: true, capacity_reached: false,
    rows,
    questions: [{ kind: "promote", number: 13, title: rows[0]!.title, human_session_count: 2, pulled_session_count: 4 }],
    analysis: "idle", consent_required: false, usable_cards: rows.length, checking_card: null, check_resume_at_unix_ms: null, forgotten: null, pending_action: null,
    expanded: null,
    search: null,
  };
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
