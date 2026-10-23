// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterAll, expect, it, vi } from "vitest";
import { emptyScope } from "../test/legacyAgentScope";
import { createActions } from "./actions";
import { BacklogItemPanel, OverviewBacklog } from "./BacklogTab";
import { TooltipProvider } from "./components/ui/tooltip";
import type { BoardProject } from "./projectBoard";
import type { BackgroundAi, BacklogOrigin, IssueDetail, SnapshotRest, Task, Workspace } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { WorkspaceDialogs } from "./WorkspaceDialogs";
import type { DispatchFn } from "./ws";

// xterm probes canvas on import; no terminal is part of this flow.
const canvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => canvas.restore());

const key = "local:/repo#1";
const bodyA = "Private memo body from device A";
const bodyB = "Private memo body from device B";
const lateA = "Late private memo body from device A";
const requestA = "Private original request from device A";
const requestB = "Private original request from device B";
const responseA = "Private original answer from device A";
const responseB = "Private original answer from device B";
const task = (title: string): Task => ({ key, source: "local", id: "L-1", url: null, title, open: true, labels: [], author: null, assignees: [], created_at_unix_ms: null, updated_at_unix_ms: null });
const project = (id: string, device: string, title: string): Workspace & { tasks: NonNullable<Workspace["tasks"]> } => ({
  id, device_id: device, path: "/repo", label: id, is_git: true, branches: ["main"], default_branch: "main",
  registered: true, temporary: false, pinned: false, inactive_checkouts: { expanded: false, checkout_ids: [] },
  checkouts: [], agent_scope: emptyScope(), tasks: { source: null, tasks: [task(title)], overflow: false },
});
const a = project("project-a", "device-a", "Memo on A");
const b = project("project-b", "device-b", "Memo on B");
const projects: BoardProject[] = [a, b].map((workspace) => ({ workspace, agents: [], device: null }));
const answer = (workspace: Workspace, body: string): IssueDetail => ({
  workspace_id: workspace.id, device_id: workspace.device_id, task_key: key, phase: "ready", body,
  failure: null, labels: [], author: null, created_at_unix_ms: null, assignees: [], comment_count: null, comments: [],
});
const origin = (workspace: Workspace, request: string, response: string): BacklogOrigin => ({
  workspace_id: workspace.id, device_id: workspace.device_id, task_key: key, phase: "ready",
  session: null, session_title: null, missed_text: null, flow_name: null, request, answer: response,
});
const background = {
  enabled: true, provider: "claude", chosen: true, fallback: [], refusal: null, unavailable_reason: null,
  providers: [{ id: "claude", label: "Claude Code", agent: "claude-code", state: "ready", headline: "", message: null, installed: true, selectable: true, retry_at_ms: null, model: "", models: [], models_fixed: false, cli_default: false, models_unavailable_reason: null }],
} as BackgroundAi;

// S1: two device project snapshots can share a path and L-1 without sharing content.
// Observe the rendered body, editable prompt and outbound work, not hook calls.
it("keeps a same-key device switch and late answer out of the detail, Start prompt and naming request", async () => {
  const shell = useShellStore.getState();
  const ui = useUiStore.getState();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const now = vi.spyOn(Date, "now").mockReturnValue(1_700_000_000_000);
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const prompt = () => document.querySelector<HTMLTextAreaElement>("[data-start-prompt]")!.value;
  const name = () => document.querySelector<HTMLInputElement>("[data-start-name]")!.value;
  const panelText = () => container.querySelector("[data-backlog-item]")!.textContent;
  const suggestions = () => events.filter((event) => event.kind === "worktree_name_suggest");
  const render = (workspace: Workspace) => root.render(
    <TooltipProvider>
      <BacklogItemPanel panel={{ kind: "item", workspaceId: workspace.id, taskKey: key }} projects={projects} actions={actions} />
      <WorkspaceDialogs actions={actions} />
    </TooltipProvider>,
  );
  const deliver = async (detail: IssueDetail) => {
    await act(async () => {
      useShellStore.setState((state) => ({ rest: { ...state.rest!, issue_work: { ...state.rest!.issue_work!, detail } } }));
    });
  };
  const deliverOrigin = async (value: BacklogOrigin) => {
    await act(async () => {
      useShellStore.setState((state) => ({ rest: { ...state.rest!, issue_work: { ...state.rest!.issue_work!, origins: [value] } } }));
    });
  };
  const deliverName = async (requestId: string, value: string) => {
    await act(async () => {
      useShellStore.setState((state) => ({ rest: { ...state.rest!, issue_work: { ...state.rest!.issue_work!, name: { request_id: requestId, phase: "ready", name: value, message: null } } } }));
    });
  };
  try {
    await act(async () => {
      useUiStore.getState().setWorkspaceDialog(null);
      useShellStore.setState({ connection: "live", rest: {
        navigator: { workspaces: [a, b] }, status: { background_ai: background },
        issue_work: { detail: answer(a, bodyA), name: null, origins: [origin(a, requestA, responseA)] },
      } as SnapshotRest });
      render(a);
    });
    expect(panelText()).toContain(bodyA);
    await act(async () => container.querySelector<HTMLButtonElement>("[data-item-start]")!.click());
    expect(prompt()).toContain(bodyA);
    expect(prompt()).toContain(requestA);
    expect(prompt()).toContain(responseA);
    expect(suggestions().map((event) => event.payload.body)).toEqual([bodyA]);
    const aNameRequest = suggestions()[0]!.payload.request_id as string;

    // Switch the actual dialog host while A's ready slot is still global.
    await act(async () => {
      useUiStore.getState().setWorkspaceDialog({ kind: "start_issue", workspaceId: b.id, taskKey: key });
      render(b);
    });
    expect(events.filter((event) => event.kind === "issue_detail_request").map((event) => event.payload)).toContainEqual({ workspace_id: b.id, task_key: key });
    expect(events.filter((event) => event.kind === "backlog_origin").map((event) => event.payload)).toContainEqual({ workspace_id: b.id, device_id: b.device_id, task_key: key });
    expect(events.filter((event) => event.kind === "backlog_origin_close").map((event) => event.payload)).toEqual([{ workspace_id: a.id, device_id: a.device_id, task_key: key }]);
    expect(panelText()).not.toContain(bodyA);
    expect(prompt()).not.toContain(bodyA);
    expect(prompt()).not.toContain(bodyB);
    expect(prompt()).not.toContain(requestA);
    expect(prompt()).not.toContain(responseA);
    expect(suggestions().map((event) => event.payload.body)).toEqual([bodyA]);

    await deliver(answer(a, lateA));
    expect(panelText()).not.toContain(lateA);
    expect(prompt()).not.toContain(lateA);
    expect(suggestions().map((event) => event.payload.body)).toEqual([bodyA]);

    await deliver(answer(b, bodyB));
    expect(panelText()).toContain(bodyB);
    expect(panelText()).not.toContain(bodyA);
    expect(prompt()).not.toContain(bodyB);
    expect(prompt()).not.toContain(bodyA);
    expect(suggestions().map((event) => event.payload.body)).toEqual([bodyA, bodyB]);
    const bNameRequest = suggestions()[1]!.payload.request_id as string;
    expect(bNameRequest).not.toBe(aNameRequest);
    expect(name()).toBe("L-1-memo-on-b");
    await deliverName(aNameRequest, "L-1-private-name-on-a");
    expect(name()).toBe("L-1-memo-on-b");
    await deliverName(bNameRequest, "L-1-ready-on-b");
    expect(name()).toBe("L-1-ready-on-b");
    await deliverName(aNameRequest, "L-1-late-name-on-a");
    expect(name()).toBe("L-1-ready-on-b");

    await deliverOrigin(origin(a, `Late ${requestA}`, `Late ${responseA}`));
    expect(prompt()).not.toContain(requestA);
    expect(prompt()).not.toContain(responseA);
    await deliverOrigin(origin(b, requestB, responseB));
    expect(prompt()).toContain(bodyB);
    expect(prompt()).toContain(requestB);
    expect(prompt()).toContain(responseB);

    await deliverOrigin(origin(a, `Late ${requestA}`, `Late ${responseA}`));
    expect(prompt()).toContain(requestB);
    expect(prompt()).toContain(responseB);
    expect(prompt()).not.toContain(requestA);
    expect(prompt()).not.toContain(responseA);

    await deliver(answer(a, lateA));
    expect(panelText()).not.toContain(lateA);
    expect(prompt()).toContain(bodyB);
    expect(prompt()).not.toContain(lateA);
    expect(suggestions().map((event) => event.payload.body)).toEqual([bodyA, bodyB]);
    await act(async () => document.querySelector<HTMLButtonElement>("[data-start-submit]")!.click());
    const start = events.find((event) => event.kind === "create_worktree");
    expect(start?.payload).toMatchObject({ device_id: b.device_id, repository_root: "/repo", task_key: key, branch: "L-1-ready-on-b", prompt: expect.stringContaining(bodyB) });
    expect(start?.payload.prompt).not.toContain(bodyA);
    expect(start?.payload.prompt).not.toContain(lateA);
    expect(start?.payload.prompt).toContain(requestB);
    expect(start?.payload.prompt).toContain(responseB);
    expect(start?.payload.prompt).not.toContain(requestA);
    expect(start?.payload.prompt).not.toContain(responseA);
  } finally {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(shell, true);
    useUiStore.setState(ui, true);
    vi.unstubAllGlobals();
    now.mockRestore();
  }
});

it("keeps A's missed flag after B is undone without giving it to B, while GitHub flags stay shared", async () => {
  const shell = useShellStore.getState();
  const ui = useUiStore.getState();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const github = { ...task("Shared GitHub issue"), key: "github:acme/app#7", source: "github", id: "#7" };
  const rows = [a, b].map((workspace) => ({ workspace: { ...workspace, tasks: { ...workspace.tasks, tasks: [...workspace.tasks.tasks, github] } }, agents: [], device: null }));
  const flags = (workspace: Workspace) => container.querySelector(`[data-backlog-section="${workspace.id}"] [data-backlog-row="${key}"] [data-backlog-missed]`);
  const globalFlags = () => container.querySelectorAll('[data-backlog-row="github:acme/app#7"] [data-backlog-missed]');
  const sent: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { sent.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => {
      useUiStore.getState().setOverview({ projectId: null, backlogQuery: "", backlogLabels: [], panel: null });
      useShellStore.setState({ connection: "live", rest: {
        navigator: { workspaces: rows.map((row) => row.workspace), devices: [] }, status: {},
        issue_work: { detail: null, name: null, missed_keys: [
          { task_key: key, device_id: a.device_id }, { task_key: key, device_id: b.device_id },
          { task_key: github.key, device_id: null },
        ] },
      } as SnapshotRest });
      root.render(<TooltipProvider><OverviewBacklog projects={rows} deviceId={a.device_id} actions={actions} /></TooltipProvider>);
    });
    expect(flags(a)).not.toBeNull();
    expect(flags(b)).not.toBeNull();
    expect(globalFlags()).toHaveLength(2);
    actions.undoBacklog(b.id, key);
    expect(sent.at(-1)).toMatchObject({ kind: "backlog_undo", payload: { workspace_id: b.id, task_key: key } });
    // Keep both catalog rows to exercise stale rows while the ledger flag
    // update arrives. The runtime regression also proves B's memo is deleted.
    await act(async () => {
      useShellStore.setState((state) => ({ rest: { ...state.rest!, issue_work: {
        ...state.rest!.issue_work!, missed_keys: [{ task_key: key, device_id: a.device_id }, { task_key: github.key, device_id: null }],
      } } }));
    });
    expect(flags(a)).not.toBeNull();
    expect(flags(b)).toBeNull();
    expect(globalFlags()).toHaveLength(2);
  } finally {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(shell, true);
    useUiStore.setState(ui, true);
    vi.unstubAllGlobals();
  }
});
