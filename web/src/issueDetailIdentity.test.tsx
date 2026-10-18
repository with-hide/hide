// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterAll, expect, it, vi } from "vitest";
import { emptyScope } from "../test/legacyAgentScope";
import { createActions } from "./actions";
import { BacklogItemPanel } from "./BacklogTab";
import { TooltipProvider } from "./components/ui/tooltip";
import type { BoardProject } from "./projectBoard";
import type { BackgroundAi, IssueDetail, SnapshotRest, Task, Workspace } from "./snapshot";
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
const task = (title: string): Task => ({ key, source: "local", id: "L-1", url: null, title, open: true, labels: [], author: null, assignees: [], created_at_unix_ms: null, updated_at_unix_ms: null });
const project = (id: string, device: string, title: string): Workspace => ({
  id, device_id: device, path: "/repo", label: id, is_git: true, branches: ["main"], default_branch: "main",
  checkouts: [], agent_scope: emptyScope(), tasks: { source: null, tasks: [task(title)], overflow: false },
} as Workspace);
const a = project("project-a", "device-a", "Memo on A");
const b = project("project-b", "device-b", "Memo on B");
const projects: BoardProject[] = [a, b].map((workspace) => ({ workspace, agents: [], device: null }));
const answer = (workspace: Workspace, body: string): IssueDetail => ({
  workspace_id: workspace.id, device_id: workspace.device_id, task_key: key, phase: "ready", body,
  message: null, labels: [], author: null, created_at_unix_ms: null, assignees: [], comment_count: null, comments: [],
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
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const prompt = () => document.querySelector<HTMLTextAreaElement>("[data-start-prompt]")!.value;
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
  try {
    await act(async () => {
      useUiStore.getState().setWorkspaceDialog(null);
      useShellStore.setState({ connection: "live", rest: {
        navigator: { workspaces: [a, b] }, status: { background_ai: background },
        issue_work: { detail: answer(a, bodyA), name: null, origins: [{ task_key: key, phase: "ready", session: null, session_title: null, missed_text: null, task_name: null, request: null, answer: null }] },
      } as SnapshotRest });
      render(a);
    });
    expect(panelText()).toContain(bodyA);
    await act(async () => container.querySelector<HTMLButtonElement>("[data-item-start]")!.click());
    expect(prompt()).toContain(bodyA);
    expect(suggestions().map((event) => event.payload.body)).toEqual([bodyA]);

    // Switch the actual dialog host while A's ready slot is still global.
    await act(async () => {
      useUiStore.getState().setWorkspaceDialog({ kind: "start_issue", workspaceId: b.id, taskKey: key });
      render(b);
    });
    expect(events.filter((event) => event.kind === "issue_detail_request").map((event) => event.payload)).toContainEqual({ workspace_id: b.id, task_key: key });
    expect(panelText()).not.toContain(bodyA);
    expect(prompt()).not.toContain(bodyA);
    expect(prompt()).not.toContain(bodyB);
    expect(suggestions().map((event) => event.payload.body)).toEqual([bodyA]);

    await deliver(answer(a, lateA));
    expect(panelText()).not.toContain(lateA);
    expect(prompt()).not.toContain(lateA);
    expect(suggestions().map((event) => event.payload.body)).toEqual([bodyA]);

    await deliver(answer(b, bodyB));
    expect(panelText()).toContain(bodyB);
    expect(panelText()).not.toContain(bodyA);
    expect(prompt()).toContain(bodyB);
    expect(prompt()).not.toContain(bodyA);
    expect(suggestions().map((event) => event.payload.body)).toEqual([bodyA, bodyB]);

    await deliver(answer(a, lateA));
    expect(panelText()).not.toContain(lateA);
    expect(prompt()).toContain(bodyB);
    expect(prompt()).not.toContain(lateA);
    expect(suggestions().map((event) => event.payload.body)).toEqual([bodyA, bodyB]);
    await act(async () => document.querySelector<HTMLButtonElement>("[data-start-submit]")!.click());
    const start = events.find((event) => event.kind === "create_worktree");
    expect(start?.payload).toMatchObject({ device_id: b.device_id, repository_root: "/repo", task_key: key, prompt: expect.stringContaining(bodyB) });
    expect(start?.payload.prompt).not.toContain(bodyA);
    expect(start?.payload.prompt).not.toContain(lateA);
  } finally {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(shell, true);
    useUiStore.setState(ui, true);
    vi.unstubAllGlobals();
  }
});
