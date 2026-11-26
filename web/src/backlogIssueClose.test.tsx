// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterAll, expect, it, vi } from "vitest";
import { emptyScope } from "../test/legacyAgentScope";
import { createActions } from "./actions";
import { BacklogItemPanel } from "./BacklogTab";
import { TooltipProvider } from "./components/ui/tooltip";
import type { BoardProject } from "./projectBoard";
import type { SnapshotRest, Task, Workspace } from "./snapshot";
import { useShellStore } from "./store";
import type { DispatchFn } from "./ws";

// xterm probes canvas on import; no terminal is part of this flow.
const canvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => canvas.restore());

const localKey = "local:/repo#1";
const githubKey = "github:acme/repo#7";
const task = (key: string, id: string, source: Task["source"], open: boolean): Task => ({
  key, source, id, url: source === "local" ? null : "https://github.com/acme/repo/issues/7", title: `Item ${id}`, open,
  labels: [], author: null, assignees: [], created_at_unix_ms: null, updated_at_unix_ms: null,
});
const project = (tasks: Task[]): Workspace & { tasks: NonNullable<Workspace["tasks"]> } => ({
  id: "project-a", device_id: "device-a", path: "/repo", label: "repo", is_git: true, branches: ["main"], default_branch: "main",
  registered: true, temporary: false, pinned: false, inactive_checkouts: { expanded: false, checkout_ids: [] },
  checkouts: [], agent_scope: emptyScope(), tasks: { source: null, tasks, overflow: false },
});

// Closing is the one way a Local issue leaves the open list (with-hide/hide issue 2947): the panel
// holds the control, it sends the core's `issue_set_open`, and a GitHub issue has none.
it("closes and reopens a Local issue from its panel and offers no control on a GitHub issue", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const show = async (workspace: Workspace, taskKey: string) => {
    const projects: BoardProject[] = [{ workspace, agents: [], device: null }];
    useShellStore.setState({ connection: "live", rest: { navigator: { workspaces: [workspace] }, issue_work: { name: null, origins: [] } } as unknown as SnapshotRest });
    await act(async () => root.render(
      <TooltipProvider><BacklogItemPanel panel={{ kind: "item", workspaceId: workspace.id, taskKey }} projects={projects} actions={actions} /></TooltipProvider>,
    ));
  };
  const sent = () => events.filter((event) => event.kind === "issue_set_open");
  const toggle = () => container.querySelector<HTMLButtonElement>("[data-item-open-toggle]");
  try {
    await show(project([task(localKey, "L-1", "local", true)]), localKey);
    expect(toggle()?.dataset.itemOpenToggle).toBe("close");
    await act(async () => toggle()!.click());
    expect(sent().at(-1)).toMatchObject({ kind: "issue_set_open", payload: { workspace_id: "project-a", task_key: localKey, open: false } });

    await show(project([task(localKey, "L-1", "local", false)]), localKey);
    expect(toggle()?.dataset.itemOpenToggle).toBe("reopen");
    await act(async () => toggle()!.click());
    expect(sent().at(-1)).toMatchObject({ kind: "issue_set_open", payload: { workspace_id: "project-a", task_key: localKey, open: true } });

    await show(project([task(githubKey, "#7", "github", true)]), githubKey);
    expect(container.querySelector("[data-backlog-item]")).not.toBeNull();
    expect(toggle()).toBeNull();
    expect(sent()).toHaveLength(2);
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
});
