import { emptyScope } from "../test/legacyAgentScope";
// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { StartIssueDialog } from "./IssueDialogs";
import type { BackgroundAi, IssueDetail, Task, Workspace } from "./snapshot";
import { useShellStore } from "./store";
import type { DispatchFn } from "./ws";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

const task = { key: "github:7", source: "github", id: "#7", url: null, title: "Fix the sidebar", open: true } as Task;
const workspace = { agent_scope: emptyScope(), id: "ws-1", path: "/repo", is_git: true, device_id: "local", branches: ["main"], default_branch: "main", checkouts: [] } as unknown as Workspace;
const claude = { id: "claude", label: "Claude Code", agent: "claude-code", state: "ready", headline: "", message: null, installed: true, selectable: true, retry_at_ms: null, model: "", models: [], models_fixed: false, cli_default: false, models_unavailable_reason: null };
const ai = (over: Partial<BackgroundAi>): BackgroundAi => ({ enabled: true, provider: "claude", chosen: true, providers: [claude], fallback: [], refusal: null, unavailable_reason: null, ...over }) as BackgroundAi;

afterEach(() => {
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
});

async function open(background: BackgroundAi, context: Partial<Pick<IssueDetail, "workspace_id" | "device_id">> = {}) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const saved = useShellStore.getState();
  await act(async () => {
    useShellStore.setState({
      connection: "live",
      rest: { status: { background_ai: background }, issue_work: { detail: { workspace_id: workspace.id, device_id: workspace.device_id, ...context, task_key: task.key, phase: "ready", body: "Body" } } },
    } as never);
    root.render(
      <TooltipProvider>
        <StartIssueDialog actions={actions} workspace={workspace} task={task} onClose={() => {}} />
      </TooltipProvider>,
    );
  });
  const q = (selector: string) => document.body.querySelector(selector) as HTMLElement | null;
  return { events, q, unmount: async () => { await act(async () => root.unmount()); useShellStore.setState(saved, true); } };
}

const kinds = (events: Parameters<DispatchFn>[0][]) => events.map((event) => event.kind);

it("asks the AI to name the worktree and says so while it works, when Hide AI can answer", async () => {
  const { events, q, unmount } = await open(ai({}));
  expect(kinds(events)).toContain("worktree_name_suggest");
  expect(q("[data-name-state]")?.getAttribute("data-name-state")).toBe("working");
  await unmount();
});

it.each([
  ["Hide AI is off", ai({ enabled: false })],
  ["no agent is chosen", ai({ provider: null, chosen: false })],
  ["the chosen agent cannot answer", ai({ providers: [{ ...claude, state: "needs_login", selectable: false }] })],
])("neither asks nor waits for a name when %s", async (_name, background) => {
  const { events, q, unmount } = await open(background);
  expect(kinds(events)).not.toContain("worktree_name_suggest");
  expect(q("[data-name-state]")).toBeNull();
  await unmount();
});

it.each([
  { workspace_id: "another-project", device_id: workspace.device_id },
  { workspace_id: workspace.id, device_id: "another-node" },
])("waits for this project's body before naming when the same key belongs to $workspace_id on $device_id", async (context) => {
  const { events, unmount } = await open(ai({}), context);
  try {
    expect(kinds(events)).toContain("issue_detail_request");
    expect(kinds(events)).not.toContain("worktree_name_suggest");
    await act(async () => {
      useShellStore.setState((state) => ({ rest: { ...state.rest!, issue_work: { ...state.rest!.issue_work!, detail: { ...state.rest!.issue_work!.detail!, workspace_id: workspace.id, device_id: workspace.device_id, body: "Correct device's body" } } } }));
    });
    const suggest = events.find((event) => event.kind === "worktree_name_suggest");
    expect(suggest?.payload).toMatchObject({ body: "Correct device's body" });
  } finally {
    await unmount();
  }
});
