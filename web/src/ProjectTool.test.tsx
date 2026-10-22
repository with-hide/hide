// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import type { ProjectStatus } from "./agentScope";
import { TooltipProvider } from "./components/ui/tooltip";
import { ProjectTool } from "./ProjectTool";
import { todoView } from "./projectStatus";
import { nextSidebarLens, type ProjectMemory, type SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { ToolRail } from "./Tools";
import { useUiStore } from "./ui";
import { emptyScope } from "../test/legacyAgentScope";
import type { DispatchFn } from "./ws";

// The Project tool (PRD lane-two-columns B26-B39): the front checkout `c`
// belongs to project `p`, whose scope carries the core's 현황.

const status = (overrides: Partial<ProjectStatus> = {}): ProjectStatus => ({
  todo: [], todo_total: 0,
  prs: { open: 0, mergeable: 0, failed: 0, review: 0, checking: 0 },
  git: { worktrees: 1, changed: 0, unpushed: 0, removable: 0, dots: [{ checkout_id: "c", state: "clean" }] },
  ...overrides,
});

function snapshot(projectStatus: ProjectStatus | undefined, detail: { kind: "worktrees"; workspace_id: string } | null = null): SnapshotRest {
  return {
    navigator: {
      focused_device_id: "local", focused_checkout_id: "c", focused_workspace_id: "p",
      devices: [{ id: "local", label: "This Mac", kind: "local", state: "local" }],
      workspaces: [{ id: "p", label: "herdr-ide", path: "/p", device_id: "local", is_git: true, agent_scope: { ...emptyScope(), status: projectStatus }, checkouts: [{ id: "c", workspace_id: "p", path: "/p", label: "main", tabs: [], active_tab_id: null, strip: [] }] }],
    },
    workspace_view: { device_id: "local", path: "/p", open: true, documents: false, tool: "project", pinned: true, document_width: null, detail_width: null, explorer_width: null, panel_width: null, lane_called: 0, lane_calls: 0, lane_called_side: "tool" },
    ui_state: { left_sidebar_visible: true, sidebar_lens: "projects", detail: detail ? { items: [detail], shown: true } : { items: [], shown: false } },
  } as unknown as SnapshotRest;
}

const memory = (questions: ProjectMemory["questions"]): ProjectMemory => ({
  workspace_id: "p", unavailable: null, loading: false, enabled: true, capacity_reached: false, evicted_cards: 0, evicted_through_unix_ms: null,
  rows: [{ number: 3, title: "card 3", kind: "rule", human_source_count: 1, provided_session_count: 0, pull_count: 0, updated_at_unix_ms: Date.now(), verifying: false, promotion: null }],
  questions, analysis: "idle", consent_required: false, usable_cards: 1, checking_card: null, check_resume_at_unix_ms: null, forgotten: null, pending_action: null, expanded: null, search: null,
});

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

  async function render(rest: SnapshotRest, project: ProjectMemory | null = null) {
    const actions = createActions((event) => { events.push(event); return true; });
    await act(async () => {
      useShellStore.setState({ connection: "live", rest, memory: { project }, usage: null });
      useUiStore.setState({ screen: { kind: "workspace" }, laneShown: true, projectTab: "status", sidebarProject: null, drawnLane: { mode: "docked", view: rest.ui_state?.detail?.shown ? "detail" : "none", tool: "project", stacked: false } });
      root.render(<TooltipProvider><ToolRail actions={actions} /><ProjectTool actions={actions} /></TooltipProvider>);
    });
    return actions;
  }
  const sent = (kind: string) => events.filter((event) => event.kind === kind).map((event) => event.payload);
  const press = async (selector: string) => {
    const element = container.querySelector<HTMLElement>(selector);
    if (!element) throw new Error(`missing ${selector}`);
    await act(async () => element.click());
  };

  it("counts the to-do on the rail and shows no number at 0 (B38)", async () => {
    await render(snapshot(status({ todo: [{ kind: "pr", number: 7 }], todo_total: 1 })));
    expect(container.querySelector('[data-lane-tool="project"] [data-lane-todo-count]')?.textContent).toBe("1");
    await render(snapshot(status()));
    expect(container.querySelector('[data-lane-tool="project"] [data-lane-todo-count]')).toBeNull();
    expect(container.querySelector('[data-project-todo="empty"]')).not.toBeNull();
  });

  it("holds the tiles' numbers empty until the core answers, and shows 0 as 0 (B33)", async () => {
    await render(snapshot(undefined));
    expect(container.querySelector('[data-project-tile="git"] [data-tile-value]')?.getAttribute("data-tile-value")).toBe("pending");
    expect(container.querySelector('[data-project-tile="cost"] [data-tile-value]')?.getAttribute("data-tile-value")).toBe("pending");
    expect(container.querySelector('[data-project-tile="sessions"] [data-tile-value]')?.textContent).toBe("0");
  });

  it("sends the sessions and PRs tiles to the sidebar narrowed to the project (B27, B29)", async () => {
    await render(snapshot(status()));
    await press('[data-project-tile-press="sessions"]');
    expect(sent("sidebar_lens_set").at(-1)).toEqual({ lens: "tasks" });
    expect(useUiStore.getState().sidebarProject).toEqual({ deviceId: "local", projectId: "p" });
  });

  it("opens the worktree table in the viewing column and folds it when pressed again (B30, B32)", async () => {
    await render(snapshot(status()));
    await press('[data-project-tile-press="git"]');
    expect(sent("detail_open")).toEqual([{ item: { kind: "worktrees", workspace_id: "p" } }]);
    await render(snapshot(status(), { kind: "worktrees", workspace_id: "p" }));
    expect(container.querySelector('[data-project-tile="git"] [data-tile-target]')?.getAttribute("data-tile-target")).toBe("close");
    await press('[data-project-tile-press="git"]');
    expect(sent("detail_show")).toEqual([{ shown: false }]);
  });

  it("dots the 기억 tab while Memory has something to handle (B26)", async () => {
    await render(snapshot(status()), memory([]));
    expect(container.querySelector('[data-project-tab-dot="memory"]')).toBeNull();
    await render(snapshot(status()), memory([{ kind: "failed", number: 9, title: "card 9", start_failure: null }]));
    expect(container.querySelector('[data-project-tab-dot="memory"]')).not.toBeNull();
    expect(container.querySelector("[data-project-memory-handle]")?.textContent).toContain("1");
  });
});

describe("the 할 일 cut (B37)", () => {
  const rows = (count: number): ProjectStatus["todo"] => Array.from({ length: count }, (_, index) => (index < 2 ? { kind: "ask", pane_id: `a${index}` } : index < 4 ? { kind: "revisit", pane_id: `r${index}` } : { kind: "pr", number: index }));

  it("shows five in the core's order with the rest counted, every row once opened", () => {
    const status: ProjectStatus = { todo: rows(8), todo_total: 8, prs: { open: 0, mergeable: 0, failed: 0, review: 0, checking: 0 }, git: { worktrees: 0, changed: 0, unpushed: 0, removable: 0, dots: [] } };
    const cut = todoView(status, false);
    expect([cut.asks.length, cut.revisit.length, cut.prs.length, cut.more]).toEqual([2, 2, 1, 3]);
    const all = todoView(status, true);
    expect([all.prs.length, all.more]).toEqual([4, 0]);
  });

  it("counts rows the core left off past its cap", () => {
    const status: ProjectStatus = { todo: rows(3), todo_total: 70, prs: { open: 0, mergeable: 0, failed: 0, review: 0, checking: 0 }, git: { worktrees: 0, changed: 0, unpushed: 0, removable: 0, dots: [] } };
    expect(todoView(status, false).more).toBe(67);
  });
});

describe("⇧⌘B (B41)", () => {
  it("turns the sidebar through 프로젝트, 태스크 and PR", () => {
    expect([nextSidebarLens("projects"), nextSidebarLens("tasks"), nextSidebarLens("prs"), nextSidebarLens(undefined)]).toEqual(["tasks", "prs", "projects", "tasks"]);
  });
});
