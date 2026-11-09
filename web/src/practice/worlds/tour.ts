// The first flow's world: the introduction site's hero window (a lead, the agent
// asking to merge, and the Project tool beside them), which shows all three
// parts of the screen with work in them. Its snapshot is `../data/tour.json`,
// kept as the core would send it with each time written as how long before
// now it was. The sidebar opens on the task list, where what waits for the
// learner is grouped; only the list it shows (tasks or projects) ever changes.

import tour from "../data/tour.json";
import { missionNode as node } from "../../missionFixture";
import type { DirectoryList } from "../../store";
import type { AgentRow, ChangedFileSnapshot, ChangesSnapshot, EditorDocumentSnapshot, EditorTabSnapshot, Mission, ViewDisplaySnapshot } from "../../snapshot";
import type { PracticeScene } from "../chapter";
import type { PracticeWorld, WorldSnapshot } from "../world";

/** What the window shows: set by the card that is up (`PracticeScene`), and by the learner's own press on the list switch. */
export type TourModel = Required<PracticeScene>;

/** A time in the data file: milliseconds, seconds or an ISO time that long before now, or a date that many days before today. */
type Ago = { $msAgo?: number; $sAgo?: number; $isoAgo?: number; $daysAgo?: number };

const DAY_MS = 86_400_000;

/** The data file's values with each time placed that long before `nowMs`. */
function dated(value: unknown, nowMs: number): unknown {
  if (Array.isArray(value)) return value.map((item) => dated(item, nowMs));
  if (!value || typeof value !== "object") return value;
  const ago = value as Ago;
  if (ago.$msAgo !== undefined) return nowMs - ago.$msAgo;
  if (ago.$sAgo !== undefined) return nowMs / 1000 - ago.$sAgo;
  if (ago.$isoAgo !== undefined) return new Date(nowMs - ago.$isoAgo).toISOString();
  if (ago.$daysAgo !== undefined) return new Date(nowMs - ago.$daysAgo * DAY_MS).toISOString().slice(0, 10);
  return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, dated(item, nowMs)]));
}

const CLAUDE = "\x1b[38;2;215;119;87m";
const CODEX = "\x1b[36m";
const DIM = "\x1b[90m";
const BOLD = "\x1b[1m";
const GREEN = "\x1b[32m";
const RESET = "\x1b[0m";
const lines = (...rows: string[]) => rows.join("\r\n");
/** A pull request reference as the terminal writes it. */
const ref = (number: number) => `#${number}`;
/** Where a terminal's full-width rule goes; it is drawn as wide as the terminal. */
const RULE = "{rule}";

/** The four panes of the tab, and what each terminal shows: short enough to read whole in a pane a quarter of the screen. */
const TERMINALS: Record<string, string> = {
  a1: lines(
    `${CLAUDE}✻${RESET} ${BOLD}Claude Code${RESET} ${DIM}· ~/billing-app${RESET}`,
    "",
    `${DIM}>${RESET} Split the invoice export in three`,
    "",
    "● Three parts, one Codex worker each.",
    `● ${BOLD}Bash${RESET}(hide agent spawn …) ${DIM}×3${RESET}`,
    "● The workers report back to me.",
    "",
    `${CLAUDE}✻ Waiting on 3 workers… ${DIM}(6m 12s)${RESET}`,
    `${DIM}${RULE}${RESET}`,
    `${DIM}>${RESET} `,
    `${DIM}${RULE}${RESET}`,
    `  ${DIM}⏵⏵ accept edits on${RESET}`,
  ),
  a2: lines(
    `${CLAUDE}✻${RESET} ${BOLD}Claude Code${RESET} ${DIM}· ~/billing-app${RESET}`,
    "",
    `${DIM}>${RESET} /hide-mission Write the 1.4 release notes`,
    "",
    "● Plan: collect what changed, draft the",
    "  notes, check each claim.",
    `● ${BOLD}Bash${RESET}(hide mission show)`,
    `  ${DIM}⎿${RESET}  3 parts · 1 working · 1 asks you`,
    "",
    `${CLAUDE}✻ Waiting on 2 workers… ${DIM}(3m 40s)${RESET}`,
    `${DIM}${RULE}${RESET}`,
    `${DIM}>${RESET} `,
    `${DIM}${RULE}${RESET}`,
  ),
  a3: lines(
    `${BOLD}>_ Codex${RESET} ${DIM}· ~/billing-app${RESET}`,
    "",
    `${CODEX}›${RESET} Check ${ref(431)} and ${ref(436)}, then ask me`,
    "",
    `${DIM}•${RESET} ${BOLD}Ran${RESET} pnpm test export`,
    `  ${DIM}└${RESET} ${GREEN}38 passed${RESET}`,
    `${DIM}•${RESET} Both pull requests are ready.`,
    `${DIM}${RULE}${RESET}`,
    ` ${BOLD}Merge both into main?${RESET}`,
    ` ${CODEX}› 1. Yes, merge both${RESET}`,
    `   2. Only ${ref(431)}   3. Not yet`,
  ),
  q1: lines(
    `${BOLD}Antigravity${RESET} ${DIM}· ~/billing-app${RESET}`,
    "",
    `${DIM}>${RESET} Write the release notes`,
    "",
    "● Collected what changed since 1.3.",
    `● ${BOLD}Write${RESET}(release-notes.md) ${GREEN}+12${RESET}`,
    "● Opened it for you to look at.",
    "",
    `${DIM}${RULE}${RESET}`,
    `${DIM}>${RESET} `,
    `${DIM}${RULE}${RESET}`,
  ),
  e1: lines(
    `${BOLD}>_ Codex${RESET} ${DIM}· ~/billing-app${RESET}`,
    "",
    `${CODEX}›${RESET} Wire the task detail endpoint`,
    "",
    `${DIM}•${RESET} ${BOLD}Edited${RESET} api/tasks.ts ${GREEN}+42${RESET} ${DIM}-3${RESET}`,
    `${DIM}•${RESET} ${BOLD}Ran${RESET} pnpm test tasks`,
    `  ${DIM}└${RESET} ${GREEN}12 passed${RESET}`,
    `${DIM}•${RESET} Adding the route to the router…`,
    "",
    `${CODEX}⠋${RESET} Working ${DIM}(1m 05s • esc to interrupt)${RESET}`,
  ),
};

const MISSION = "mission-release-notes";
const ROOT = "/work/billing-app";
const NOTES_PATH = `${ROOT}/release-notes.md`;
const NOTES = [
  "# Release notes 1.4",
  "",
  "## New",
  "- Export invoices as CSV from the invoices page",
  "- Search sessions, files and pull requests with ⌘K",
  "",
  "## Fixed",
  "- The test that broke the export",
  "",
].join("\n");

/** The mission pane B conducts: a plan of three parts, one done, one in progress, one waiting for it. */
function tourMission(nowMs: number, agents: AgentRow[]): Mission {
  const minute = 60_000;
  const lead = agents.find((row) => row.pane_id === "a2");
  const opus = { agent: "claude", model: "Opus", effort: "high", effort_auto: true };
  const codex = { agent: "codex", model: "GPT-5.6-Sol", effort: "medium", effort_auto: true };
  return {
    id: MISSION, title: "Release notes 1.4", workspace_id: "hide", project: "/work/billing-app", checkout_path: "/work/billing-app", device_id: "local",
    status: "running", scope: "pr", cap: 4, exempt: false, strength: null,
    conductor: { pane_id: "a2", name: "Release notes 1.4 · Conductor", agent_kind: "claude", session: { device_id: "local", agent: lead?.agent_kind ?? "claude", session_id: lead?.session_id ?? "session-a2" }, present: true, takes_answers: true },
    nodes: [
      node("collect", "Collect what changed", { kind: "research", delivery: null, worker: opus, status: "done", started_at_unix_ms: nowMs - 40 * minute, finished_at_unix_ms: nowMs - 30 * minute, line: "Findings in the plan" }),
      node("draft", "Draft the release notes", { depends_on: ["collect"], worker: codex, pane_id: "a2c2", status: "working", started_at_unix_ms: nowMs - 12 * minute, pr: 439, pr_state: "pending", stage: 2, pr_url: "https://example.invalid/pull/439" }),
      node("claims", "Check each claim against the diff", { kind: "verify", delivery: null, depends_on: ["draft"], worker: { agent: "codex", model: "GPT-5.6-Sol", effort: "low", effort_auto: false } }),
    ],
    merge_order: ["draft"], landing_order: [], stack_waiting: 0, check: "pnpm test", github: "example",
    decision: null, question: null, answer: null, answer_failed: null, delay: null, report: null,
    created_at_unix_ms: nowMs - 45 * minute, started_at_unix_ms: nowMs - 42 * minute, closed_at_unix_ms: null, running: 1,
    stale: false, github_stale: false, github_read_at_unix_ms: nowMs,
  };
}

/** The Documents the lane shows on the card that opens it: the file an agent made for the learner to look at. */
function notes() {
  const tab: EditorTabSnapshot = { id: "t1", device_id: "local", workspace_id: "hide", checkout_id: "hide:main", path: NOTES_PATH, label: "release-notes.md", kind: "file", diff_committed: null, markdown_live: true, wrap: true, dirty: false, preview: false };
  const display: ViewDisplaySnapshot = { id: "d1", tab_id: "t1", path: NOTES_PATH, label: "release-notes.md", kind: "file", committed: null, preview: false, state: "open", reason: null };
  const document: EditorDocumentSnapshot = { path: NOTES_PATH, language: "markdown", document_kind: "markdown", contents_utf8: NOTES, revision: "sha256:0", dirty: false, readonly_reason: null, conflict: null, save: null };
  return { tab, display, document };
}

/** The folders the Explorer lists for that session's checkout: the project's root, with the api folder open. */
function listings(): Record<string, DirectoryList> {
  const list = (folder: string, entries: [string, boolean][]): DirectoryList => ({ kind: "file_list", root_path: folder, truncated: false, entries: entries.map(([name, is_directory]) => ({ name, path: `${folder}/${name}`, is_directory })) });
  return {
    [ROOT]: list(ROOT, [[".github", true], ["api", true], ["docs", true], ["src", true], ["package.json", false], ["README.md", false], ["release-notes.md", false]]),
    [`${ROOT}/api`]: list(`${ROOT}/api`, [["router.ts", false], ["tasks.ts", false]]),
  };
}

/** What the agent changed in its checkout, as the Explorer marks it: two files it edited and the one it made. */
function changes(): ChangesSnapshot {
  const entry = (relative: string, status: ChangedFileSnapshot["status"], added: number | null, removed: number | null): ChangedFileSnapshot => ({ path: `${ROOT}/${relative}`, relative_path: relative, previous_relative_path: null, status, added_lines: added, removed_lines: removed });
  return {
    root_path: ROOT,
    entries: [entry("api/router.ts", "modified", 4, 1), entry("api/tasks.ts", "modified", 42, 3), entry("release-notes.md", "untracked", null, null)],
    committed: [], base_branch: "main", selected_path: null, selected_committed: false, diff: null, unavailable_reason: null,
  };
}

/** One pane alone, on the cards whose lane takes the room of the other three. */
const solo = (pane: string) => ({ type: "pane", pane_id: pane }) as const;

/** One tab, four panes like a real day: a lead with children (A), a Conductor (B), a reviewer asking (C) and one agent working by itself (D). */
const PANES = {
  type: "split", direction: "down", ratio: 0.5,
  first: { type: "split", direction: "right", ratio: 0.5, first: { type: "pane", pane_id: "a1" }, second: { type: "pane", pane_id: "a2" } },
  second: { type: "split", direction: "right", ratio: 0.5, first: { type: "pane", pane_id: "a3" }, second: { type: "pane", pane_id: "e1" } },
} as const;

export const TOUR_WORLD: PracticeWorld<TourModel> = {
  initial: () => ({ lens: "tasks", lane: "closed", focus: null, solo: null }),
  scene: (model, scene) => ({ ...model, ...scene }),
  // The learner's own press on the list switch turns the list, as in Hide; nothing else changes the window.
  apply: (model, event) => {
    if (event.kind !== "sidebar_lens_set") return null;
    const lens = event.payload.lens;
    return (lens === "projects" || lens === "tasks") && lens !== model.lens ? { ...model, lens } : null;
  },
  snapshot(model, nowMs) {
    const { rest, agents: rows, usage } = dated(tour, nowMs) as Omit<WorldSnapshot, "focused">;
    // The core stamps each row with its machine; the children popover reads it to say whether a child's device is away.
    const agents = rows.map((row) => ({ ...row, device_id: row.device_id ?? "local" }));
    // The lead has eight children in the data; the popover lists three, working, waiting and resting mixed, and says how many more.
    rest.navigator!.siblings_shown = 3;
    const view = { ...rest.workspace_view!, open: false, tool: null, pinned: false, documents: false };
    let ui_state = { ...rest.ui_state, sidebar_lens: model.lens } as NonNullable<typeof rest.ui_state>;
    let extra: WorldSnapshot["extra"];
    if (model.lane === "documents" || model.lane === "explorer") {
      const { tab, display, document } = notes();
      Object.assign(view, { open: true, documents: true, pinned: true, layout: { root: { area: { id: "v1", active: display.id, displays: [display] } }, active_area: "v1", limits: { areas: 4, depth: 3, displays: 32 }, display_count: 1 } });
      extra = { editor: { tabs: [tab], active_tab_id: tab.id }, documents: { [tab.id]: document } };
      if (model.lane === "explorer") {
        Object.assign(view, { tool: "explorer" });
        ui_state = { ...ui_state, expanded_paths: Object.keys(listings()), selected_path: NOTES_PATH };
        extra.listings = listings();
        extra.changes = changes();
        rest.navigator!.changes_root_path = ROOT;
        // Both columns side by side, as the lane's Expand draws them: at this width they would otherwise stack.
        extra.laneExpanded = true;
      }
    } else if (model.lane === "mission") {
      Object.assign(view, { open: true, pinned: true, detail_width: 520 });
      ui_state = { ...ui_state, detail: { items: [{ kind: "mission", mission: MISSION }], shown: true } };
    }
    // The fourth pane's agent works in another worktree; the tab lists it so its pane is drawn.
    const checkouts = rest.navigator!.workspaces!.find((one) => one.id === "hide")!.checkouts;
    const e1 = checkouts.flatMap((one) => one.tabs.flatMap((tab) => tab.panes)).find((pane) => pane.id === "e1");
    const main = checkouts.find((one) => one.id === "hide:main")!.tabs[0]!;
    if (e1 && !main.panes.some((pane) => pane.id === "e1")) main.panes.push({ ...e1, identity_label: "Task detail API", cwd: "/work/billing-app" });
    // The agent that opened the file waits in another worktree too; pressing its 볼 것 row brings its pane forward.
    const q1 = checkouts.flatMap((one) => one.tabs.flatMap((tab) => tab.panes)).find((pane) => pane.id === "q1");
    if (q1 && !main.panes.some((pane) => pane.id === "q1")) main.panes.push({ ...q1, identity_label: "Write the release notes", cwd: "/work/billing-app" });
    // Another computer's tile on the device rail: connected, with nothing that waits on the learner there.
    const home = rest.navigator!.devices![0]!;
    const quiet = rest.navigator!.workspaces![0]!.agent_scope;
    rest.navigator!.devices = [home, { ...home, id: "mini", label: "Mac mini", kind: "remote", state: "connected", agent_count: 2, agent_scope: quiet }];
    const layouts = (rest.pane_layouts ?? []).map((one) => ({ ...one, focused_pane_id: model.focus ?? one.focused_pane_id, root: model.solo ? solo(model.solo) : PANES }));
    const headers = { a1: { working: true, notice: null, band: null }, a2: { working: true, notice: null, band: null }, e1: { working: true, notice: null, band: null } };
    return {
      rest: { ...rest, ui_state, workspace_view: view, pane_layouts: layouts, terminal: { ...rest.terminal, headers }, status: { ...rest.status, remote: [{ target_id: "mini", state: "connected" }] } as never, missions: { items: [tourMission(nowMs, agents)] } },
      agents, focused: model.focus, usage, extra,
    };
  },
  panes: () => Object.keys(TERMINALS),
  screen: (_model, paneId, cols) => TERMINALS[paneId]!.replaceAll(RULE, "─".repeat(cols)),
};
