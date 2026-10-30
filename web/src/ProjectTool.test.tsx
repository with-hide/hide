// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import type { ProjectStatus, ProjectStray } from "./agentScope";
import { TooltipProvider } from "./components/ui/tooltip";
import { DetailPanel } from "./DetailPanel";
import { ProjectTool } from "./ProjectTool";
import { durationWords, filterCounts, strayParts, strayView, taskIn } from "./projectStatus";
import { nextSidebarLens, type ProjectMemory, type ProjectSuggestions, type SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { ToolRail } from "./Tools";
import { useUiStore } from "./ui";
import { emptyScope } from "../test/legacyAgentScope";
import { suggestionsProject } from "../test/suggestionsProject";
import type { DispatchFn } from "./ws";
import type { ProjectCost, TaskCost, UsageSnapshot } from "./generated/hided-ws";

// The Project tool as the results column (PRD project-panel-results): the
// front checkout `c` belongs to project `p`, whose scope carries the core's
// counts and 흘린 것, and the usage frame its week and tasks.

const stray = (overrides: Partial<ProjectStray> = {}): ProjectStray => ({ checkout_id: "w", branch: "fix-emails", name: "Retry failed emails", changed_files: 4, unpushed: 0, ...overrides });

const status = (overrides: Partial<ProjectStatus> = {}): ProjectStatus => ({
  prs: { open: 0, mergeable: 0, failed: 0, review: 0, checking: 0 },
  git: { worktrees: 1, changed: 0, unpushed: 0, removable: 0, dots: [{ checkout_id: "c", state: "clean" }] },
  strays: [], strays_total: 0,
  ...overrides,
});

const task = (id: string, result: TaskCost["result"], extra: Partial<TaskCost> = {}): TaskCost => ({
  id, kind: "task", root: { agent: "claude", session_id: `s-${id}` }, micro_usd: 1_000_000, tokens: 1_000, unpriced: false,
  started_at_unix_ms: 0, ended_at_unix_ms: 0, elapsed_ms: 3_600_000, worked_ms: 1_800_000,
  sessions: [{ agent: "claude", session_id: `s-${id}`, depth: 0, micro_usd: 1_000_000, tokens: 1_000, worked_ms: 1_800_000, unpriced: false }], sessions_total: 1, result, ...extra,
});

const cost = (overrides: Partial<ProjectCost> = {}): ProjectCost => ({
  workspace_id: "p", micro_usd: 3_000_000, tokens: 3_000, days: [], models: [], agents: [], unpriced_agents: [], partial: false, waiting: false, stale: false, limited: false,
  tasks: [], tasks_total: 0,
  week: { merged_prs: 0, open_prs: 0, missions: 0, mission_reopened: 0, merged_by_day: [], per_pr: null, no_result_tasks: 0, no_result_micro_usd: 0, no_result_unpriced: false },
  ...overrides,
});

function snapshot(projectStatus: ProjectStatus | undefined, detail: { kind: "worktrees" | "cost"; workspace_id: string } | null = null): SnapshotRest {
  return {
    navigator: {
      focused_device_id: "local", focused_checkout_id: "c", focused_workspace_id: "p",
      devices: [{ id: "local", label: "This Mac", kind: "local", state: "local" }],
      workspaces: [{ id: "p", label: "herdr-ide", path: "/p", device_id: "local", is_git: true, agent_scope: { ...emptyScope(), status: projectStatus }, checkouts: [{ id: "c", workspace_id: "p", path: "/p", label: "main", tabs: [], active_tab_id: null, strip: [], agent_scope: { ...emptyScope(), git_mark: "clean" } }, { id: "w", workspace_id: "p", path: "/w", label: "fix-emails", branch: "fix-emails", tabs: [], active_tab_id: null, strip: [], agent_scope: { ...emptyScope(), git_mark: "changed" } }] }],
    },
    workspace_view: { device_id: "local", path: "/p", open: true, documents: false, tool: "project", pinned: true, document_width: null, detail_width: null, explorer_width: null, panel_width: null, lane_called: 0, lane_calls: 0, lane_called_side: "tool" },
    ui_state: { left_sidebar_visible: true, sidebar_lens: "projects", detail: detail ? { items: [detail], shown: true } : { items: [], shown: false } },
  } as unknown as SnapshotRest;
}

describe("the Project tool", () => {
  let container: HTMLDivElement;
  let root: Root;
  let saved: ReturnType<typeof useShellStore.getState>;
  let savedUi: ReturnType<typeof useUiStore.getState>;
  let events: Parameters<DispatchFn>[0][];

  beforeEach(() => {
    saved = useShellStore.getState();
    savedUi = useUiStore.getState();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    events = [];
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(saved, true);
    useUiStore.setState(savedUi, true);
    vi.unstubAllGlobals();
  });

  async function render(rest: SnapshotRest, project: ProjectMemory | null = null, usage: UsageSnapshot | null = null, suggestions: ProjectSuggestions | null = null) {
    const actions = createActions((event) => { events.push(event); return true; });
    await act(async () => {
      useShellStore.setState({ connection: "live", rest, memory: { project }, suggestions: { project: suggestions }, usage });
      useUiStore.setState({ screen: { kind: "workspace" }, drawnLane: { mode: "docked", view: rest.ui_state?.detail?.shown ? "detail" : "none", tool: "project", stacked: false } });
      root.render(<TooltipProvider><ToolRail actions={actions} /><ProjectTool actions={actions} />{rest.ui_state?.detail?.shown ? <DetailPanel actions={actions} /> : null}</TooltipProvider>);
    });
    return actions;
  }
  const sent = (kind: string) => events.filter((event) => event.kind === kind).map((event) => event.payload);
  const press = async (selector: string) => {
    const element = container.querySelector<HTMLElement>(selector);
    if (!element) throw new Error(`missing ${selector}`);
    await act(async () => element.click());
  };
  const node = (extra: object): UsageSnapshot => ({ nodes: [{ node: "local", agents: [], history: null, ...extra }] });

  it("counts 흘린 것 on the rail and shows no number at 0", async () => {
    await render(snapshot(status({ strays: [stray()], strays_total: 1 })));
    expect(container.querySelector('[data-lane-tool="project"] [data-lane-todo-count]')?.textContent).toBe("1");
    expect(container.querySelector('[data-lane-tool="project"]')?.getAttribute("aria-description")).toBe("1 left behind");
    await render(snapshot(status()));
    expect(container.querySelector('[data-lane-tool="project"] [data-lane-todo-count]')).toBeNull();
  });

  it("counts what Hide AI asks on its icon, apart from the Project's 흘린 것", async () => {
    await render(snapshot(status()), null, null, suggestionsProject({ counts: { suggestions: 2, rail: 3 } }));
    expect(container.querySelector('[data-lane-tool="memory"] [data-lane-todo-count]')?.textContent).toBe("3");
    expect(container.querySelector('[data-lane-tool="memory"]')?.getAttribute("aria-description")).toBe("Suggestions 2 · Memory 1");
    await render(snapshot(status()), null, null, suggestionsProject({ enabled: false }));
    expect(container.querySelector('[data-lane-tool="memory"] [data-lane-todo-count]')).toBeNull();
    await render(snapshot(status()), null, null, suggestionsProject({ workspace_id: "other", counts: { suggestions: 2, rail: 3 } }));
    expect(container.querySelector('[data-lane-tool="memory"] [data-lane-todo-count]')).toBeNull();
  });

  it("holds a count empty until the core answers and draws 0 as 0, with no tile and no 할 일 (B1, B5)", async () => {
    await render(snapshot(undefined));
    expect(container.querySelector('[data-project-count="worktrees"] [data-count-pending]')).not.toBeNull();
    expect(container.querySelector('[data-project-count="sessions"] [data-count-value]')?.textContent).toBe("0");
    expect(container.querySelector("[data-project-tiles], [data-project-todo]")).toBeNull();
  });

  it("sends the session count to the sidebar narrowed to the project, and the worktree count opens and folds the table (B2)", async () => {
    await render(snapshot(status()));
    await press('[data-project-count="sessions"]');
    expect(sent("sidebar_project_set").at(-1)).toEqual({ project: { device_id: "local", workspace_id: "p" }, lens: "tasks" });
    await press('[data-project-count="worktrees"]');
    expect(sent("detail_open")).toEqual([{ item: { kind: "worktrees", workspace_id: "p" } }]);
    await render(snapshot(status(), { kind: "worktrees", workspace_id: "p" }));
    expect(container.querySelector('[data-project-count="worktrees"]')?.getAttribute("aria-pressed")).toBe("true");
    await press('[data-project-count="worktrees"]');
    expect(sent("detail_show")).toEqual([{ shown: false }]);
  });

  it("draws no 흘린 것 part when nothing is left, and five rows with +N 더 past that (B8)", async () => {
    await render(snapshot(status()));
    expect(container.querySelector("[data-project-strays]")).toBeNull();
    const rows = Array.from({ length: 7 }, (_, index) => stray({ checkout_id: `w${index}` }));
    await render(snapshot(status({ strays: rows, strays_total: 9 })));
    expect(container.querySelectorAll("[data-stray-row]")).toHaveLength(5);
    expect(container.querySelector("[data-project-strays-more]")?.getAttribute("data-project-strays-more")).toBe("4");
  });

  it("names what a row left and opens its evidence, or the failing pull request's card for CI alone (B6, B9)", async () => {
    await render(snapshot(status({ strays: [stray({ unpushed: 3, failed_pr: 1815 })], strays_total: 1 })));
    expect(container.querySelector('[data-stray-row="w"]')?.textContent).toContain(`Uncommitted changes in 4 files · Unpushed commits 3 · #${1815} CI failed`);
    await press('[data-stray-open="w"]');
    expect(sent("detail_open").at(-1)).toEqual({ item: { kind: "remains", workspace_id: "p", checkout_id: "w" } });
    await render(snapshot(status({ strays: [stray({ changed_files: 0, failed_pr: 1815 })], strays_total: 1 })));
    await press('[data-stray-open="w"]');
    expect(sent("pr_open").at(-1)).toEqual({ workspace_id: "p", pr_number: 1815, checkout_id: "w" });
  });

  it("hands a row off once, showing progress and no 놔두기 while it sends (B11, B12)", async () => {
    await render(snapshot(status({ strays: [stray()], strays_total: 1 })));
    await press('[data-stray-hand-off="w"]');
    await press('[data-stray-hand-off="w"]');
    const handOffs = sent("stray_hand_off") as { workspace_id: string; checkout_id: string; request_id: string }[];
    expect(handOffs).toHaveLength(1);
    expect(handOffs[0]).toMatchObject({ workspace_id: "p", checkout_id: "w" });
    expect(container.querySelector('[data-stray-row="w"] [role="status"]')).not.toBeNull();
    expect(container.querySelector("[data-stray-dismiss]")).toBeNull();
  });

  it("keeps the hand-off off once sent while the row stands, and back on after it failed (B12)", async () => {
    const rows = snapshot(status({ strays: [stray()], strays_total: 1 }));
    await render(rows);
    await press('[data-stray-hand-off="w"]');
    const first = (sent("stray_hand_off") as { request_id: string }[])[0]!.request_id;
    const receipt = (state: "sent" | "failed") => ({ ...rows, status: { stray_hand_offs: [{ request_id: first, checkout_id: "w", state }] } }) as SnapshotRest;
    await render(receipt("sent"));
    expect(container.querySelector('[data-stray-row="w"] [role="status"]')).toBeNull();
    await press('[data-stray-hand-off="w"]');
    expect(sent("stray_hand_off")).toHaveLength(1);
    // A receipt the core let go after `sent` still reads sent.
    await render(rows);
    await press('[data-stray-hand-off="w"]');
    expect(sent("stray_hand_off")).toHaveLength(1);
    await render(receipt("failed"));
    await press('[data-stray-hand-off="w"]');
    expect(sent("stray_hand_off")).toHaveLength(2);
  });

  it("asks the core to keep 놔두기, and its 되돌리기 to take it back (B15)", async () => {
    await render(snapshot(status({ strays: [stray()], strays_total: 1 })));
    await press('[data-stray-dismiss="w"]');
    expect(sent("stray_dismiss")).toEqual([{ workspace_id: "p", checkout_id: "w" }]);
    // The core takes the row out; the 되돌리기 line stays a moment.
    await render(snapshot(status()));
    await press('[data-stray-restore="w"]');
    expect(sent("stray_restore")).toEqual([{ workspace_id: "p", checkout_id: "w" }]);
  });

  it("draws no week before the core's first build, $0.00 after a read with nothing, and a dash when no build succeeded (B23)", async () => {
    await render(snapshot(status()), null, node({}));
    expect(container.querySelector('[data-project-week="pending"]')).not.toBeNull();
    await render(snapshot(status()), null, node({ projects: [] }));
    expect(container.querySelector("[data-week-cost]")?.textContent).toBe("$0.00");
    await render(snapshot(status()), null, node({ projects_failed: true }));
    expect(container.querySelector('[data-project-week="failed"]')).not.toBeNull();
  });

  it("opens the 작업 비용 table on 결과 없음 from the week's line, and on the costly task from its row (B22, B25)", async () => {
    const tasks = [task("a", { kind: "pr", number: 7, state: "merged" }), task("b", { kind: "none" })];
    await render(snapshot(status()), null, node({ projects: [cost({ tasks, tasks_total: 2, week: { ...cost().week, no_result_tasks: 1, no_result_micro_usd: 1_000_000 } })] }));
    await press("[data-week-no-result]");
    expect(useUiStore.getState().taskTable).toEqual({ filter: "none", task: null });
    expect(sent("detail_open").at(-1)).toEqual({ item: { kind: "cost", workspace_id: "p" } });
    await press('[data-costly-task="a"] button');
    expect(useUiStore.getState().taskTable).toEqual({ filter: "all", task: "a" });
  });

  it("lists the tasks under a filter with each filter's count, the 결과 없음 rows saying their last words (B27, B30, B32)", async () => {
    const tasks = [task("a", { kind: "pr", number: 7, state: "merged" }), task("b", { kind: "none" }, { last_line: "Gave up on the flaky test" }), task("c", { kind: "working" })];
    await render(snapshot(status(), { kind: "cost", workspace_id: "p" }), null, node({ projects: [cost({ tasks, tasks_total: 5 })], hide_ai_week_micro_usd: 420_000 }));
    expect(container.querySelector('[data-task-filter="merged"]')?.textContent).toContain("1");
    expect(container.querySelectorAll("[data-task-row]")).toHaveLength(3);
    expect(container.querySelector("[data-task-over-cap]")?.getAttribute("data-task-over-cap")).toBe("2");
    expect(container.querySelector("[data-cost-hide-ai]")?.textContent).toBe("Hide AI used $0.42");
    await act(async () => useUiStore.getState().showTaskTable("none", null));
    expect(container.querySelectorAll("[data-task-row]")).toHaveLength(1);
    expect(container.querySelector('[data-task-row="b"]')?.textContent).toContain("Gave up on the flaky test");
    await press('[data-task-conversation="s-b"]');
    expect(sent("detail_open").at(-1)).toEqual({ item: { kind: "record", session: { device_id: "local", agent: "claude", session_id: "s-b" } } });
    await act(async () => useUiStore.getState().showTaskTable("mission", null));
    expect(container.querySelector('[data-task-table-empty="mission"]')).not.toBeNull();
  });

  it("opens a task onto its sessions under their parents (B29)", async () => {
    const children = [{ agent: "claude", session_id: "s-a", depth: 0, micro_usd: 1, tokens: 1, worked_ms: 1, unpriced: false }, { agent: "codex", session_id: "s-child", depth: 1, micro_usd: 0, tokens: 1, worked_ms: 1, unpriced: true }];
    await render(snapshot(status(), { kind: "cost", workspace_id: "p" }), null, node({ projects: [cost({ tasks: [task("a", { kind: "commit" }, { sessions: children, sessions_total: 2 })], tasks_total: 1 })] }));
    expect(container.querySelector("[data-task-sessions]")).toBeNull();
    await press('[data-task-row="a"] button');
    expect([...container.querySelectorAll("[data-task-session]")].map((row) => row.getAttribute("data-task-depth"))).toEqual(["0", "1"]);
    expect(container.querySelector('[data-task-session="s-child"]')?.textContent).toContain("No price");
  });
});

describe("the results' rules", () => {
  it("cuts 흘린 것 at five and counts the rest, every row once opened (B8)", () => {
    const rows = Array.from({ length: 8 }, (_, index) => stray({ checkout_id: `w${index}` }));
    expect(strayView(rows, 70, false)).toMatchObject({ more: 65 });
    expect(strayView(rows, 8, true)).toMatchObject({ more: 0 });
  });

  it("says what a worktree left in its order: changes, commits, then the failed pull request (B6)", () => {
    expect(strayParts(stray({ changed_files: 0, unpushed: 2, failed_pr: 9 }))).toEqual([{ kind: "unpushed", count: 2 }, { kind: "ci", number: 9 }]);
  });

  it("puts a task under exactly the filters its result names (B30)", () => {
    const tasks = [task("m", { kind: "pr", number: 1, state: "merged" }), task("o", { kind: "pr", number: 2, state: "open" }), task("x", { kind: "pr", number: 3, state: "closed" }), task("w", { kind: "working" }), task("l", { kind: "changes" }), task("k", { kind: "commit" }), task("n", { kind: "none" }), task("q", { kind: "commit" }, { kind: "mission" })];
    expect(filterCounts(tasks)).toEqual({ all: 8, merged: 1, going: 3, mission: 1, none: 1 });
    expect(taskIn(task("x", { kind: "pr", number: 3, state: "closed" }), "none")).toBe(false);
  });

  it("words a span in minutes, then hours to one decimal, then days", () => {
    expect([durationWords("en", 25 * 60_000), durationWords("en", 3.44 * 3_600_000), durationWords("en", 72 * 3_600_000)]).toEqual(["25m", "3.4h", "3d"]);
  });
});

describe("⇧⌘B (B41)", () => {
  it("turns the sidebar through 프로젝트, 태스크 and PR", () => {
    expect([nextSidebarLens("projects"), nextSidebarLens("tasks"), nextSidebarLens("prs"), nextSidebarLens(undefined)]).toEqual(["tasks", "prs", "projects", "tasks"]);
  });
});
