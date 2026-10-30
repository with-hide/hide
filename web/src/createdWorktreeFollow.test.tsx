// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { emptyScope } from "../test/legacyAgentScope";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { followStartedPane } from "./opening";
import type { Checkout, SnapshotRest, Workspace } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { WorkspaceDialogs, WorkspaceNotices } from "./WorkspaceDialogs";
import type { DispatchFn } from "./ws";

// A worktree created on this machine comes forward
// only through its start's follow in the core, which a newer visit cancels.
// The page must not send a focus of its own when the pane is listed later.

const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => browserCanvas.restore());

function checkout(id: string, path: string, pane: string, isWorktree: boolean): Checkout {
  return {
    agent_scope: emptyScope(), id, workspace_id: "project:example", label: id, path, branch: id, purpose: null,
    is_worktree: isWorktree, exists: true, has_panes: true, pull_request: null, active_tab_id: `tab:${id}`, strip: [], next_tab_label: "2",
    tabs: [{ id: `tab:${id}`, label: "1", panes: [{ id: pane }] }], worktree: null,
  } as unknown as Checkout;
}
function project(device: string, checkouts: Checkout[]): Workspace {
  return {
    agent_scope: emptyScope(), id: "project:example", label: "Example", path: "/projects/example", device_id: device,
    registered: true, temporary: false, pinned: false, checkouts, branches: ["main"], inactive_checkouts: { expanded: false, checkout_ids: [] },
  } as unknown as Workspace;
}
const main = checkout("checkout:main", "/projects/example", "pane-main", false);
const created = checkout("checkout:next", "/projects/example-next", "pane-next", true);
function localRest(listed: boolean, operation: unknown = null): SnapshotRest {
  return {
    navigator: { devices: [{ id: "local", kind: "local" }], focused_device_id: "local", workspaces: [project("local", listed ? [main, created] : [main])] },
    focused: { pane_id: "pane-main" },
    status: { remote: [] },
    task_operation: operation,
  } as unknown as SnapshotRest;
}
function remoteRest(listed: boolean): SnapshotRest {
  const pane = "remote:mini:pane:p9";
  return {
    navigator: { devices: [{ id: "local", kind: "local" }, { id: "mini", kind: "remote" }], focused_device_id: "local", workspaces: [project("local", [main])] },
    focused: { pane_id: "pane-main" },
    status: { remote: [{ target_id: "mini", state: "connected", session: { agents: [], workspaces: listed ? [project("mini", [checkout("checkout:mini", "/home/me/example-next", pane, true)])] : [] } }] },
  } as unknown as SnapshotRest;
}

describe("a created worktree's follow", () => {
  let container: HTMLDivElement;
  let root: Root;
  let shell: ReturnType<typeof useShellStore.getState>;
  let ui: ReturnType<typeof useUiStore.getState>;
  let events: Parameters<DispatchFn>[0][];
  let actions: ReturnType<typeof createActions>;

  beforeEach(() => {
    shell = useShellStore.getState();
    ui = useUiStore.getState();
    vi.stubGlobal("ResizeObserver", class BrowserResizeObserver { observe() {} unobserve() {} disconnect() {} });
    events = [];
    actions = createActions((event) => { events.push(event); return true; });
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(shell, true);
    useUiStore.setState(ui, true);
    vi.unstubAllGlobals();
  });
  const focusesOf = (pane: string) => events.filter((event) => event.kind === "focus_pane" && (event.payload as { pane_id?: string }).pane_id === pane);

  it("leaves a local worktree to the core when the operator visits elsewhere before it is listed", async () => {
    useShellStore.setState({ connection: "live", rest: localRest(false) });
    useUiStore.setState({ screen: { kind: "workspace" }, workspaceDialog: { kind: "new_worktree", workspaceId: "project:example" } });
    await act(async () => root.render(<TooltipProvider><WorkspaceDialogs actions={actions} /><WorkspaceNotices actions={actions} /></TooltipProvider>));

    const field = document.querySelector<HTMLInputElement>("[data-worktree-branch]")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(field, "feature/next");
      field.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => document.querySelector<HTMLButtonElement>("[data-worktree-create]")!.click());
    const create = events.find((event) => event.kind === "create_worktree")!;
    const requestId = (create.payload as { request_id?: string }).request_id;
    expect(requestId).toMatch(/^start-/);
    // The page only presents what the core selects for this start.
    expect(useUiStore.getState().opening?.target).toEqual({ startRequestId: requestId, startKind: "worktree_create" });

    // The creation lands before the catalog lists its pane.
    const ready = { id: 1, kind: "worktree_create", phase: "ready", request_id: requestId, device_id: null, repository_root: "/projects/example", branch: "feature/next", base_branch: "main", path: "/projects/example-next", pane_id: "pane-next", agent_kind: null, agent_phase: null, agent_reason: null, agent_line: null, failure: null };
    await act(async () => useShellStore.setState({ rest: localRest(false, ready) }));
    expect(document.querySelector("[data-new-worktree]")).toBeNull();
    expect(useUiStore.getState().focusWhenListed).toBeNull();

    // A newer visit, then the late listing: the core cancelled its follow,
    // and the page sends nothing that would undo the visit.
    await act(async () => actions.openAgent("pane-main"));
    await act(async () => useShellStore.setState({ rest: localRest(true, ready) }));
    expect(focusesOf("pane-main")).toHaveLength(1);
    expect(focusesOf("pane-next")).toHaveLength(0);
    expect(useUiStore.getState().focusWhenListed).toBeNull();
  });

  it("hands a pull request's start on this machine to the core's follow by its request id", () => {
    useShellStore.setState({ connection: "live", rest: remoteRest(false) });
    actions.delegatePullRequest({ id: "project:example", device_id: "local" }, 7, "claude", null, "Fix it", "worktree_create");
    const requestId = (events[0]!.payload as { request_id?: string }).request_id;
    expect(requestId).toMatch(/^start-/);
    expect(useUiStore.getState().opening?.target).toEqual({ startRequestId: requestId, startKind: "worktree_create" });
    useUiStore.setState({ opening: null });
    actions.delegatePullRequest({ id: "project:example", device_id: "mini" }, 7, "claude", null, "Fix it", "agent_start");
    expect((events[1]!.payload as { request_id?: string }).request_id).toBeUndefined();
    expect(useUiStore.getState().opening).toBeNull();
  });

  it("opens a device's created worktree once its pane is listed, through that device", async () => {
    useShellStore.setState({ connection: "live", rest: remoteRest(false) });
    await act(async () => root.render(<WorkspaceNotices actions={actions} />));

    actions.createWorktree({ deviceId: "mini", repositoryRoot: "/home/me/example", branch: "feature/next", baseBranch: null, agentKind: null, model: null, purpose: null });
    expect((events[0]!.payload as { request_id?: string }).request_id).toBeUndefined();
    expect(useUiStore.getState().opening).toBeNull();

    await act(async () => followStartedPane("remote:mini:pane:p9"));
    expect(useUiStore.getState().focusWhenListed).toBe("remote:mini:pane:p9");
    await act(async () => useShellStore.setState({ rest: remoteRest(true) }));
    expect(useUiStore.getState().focusWhenListed).toBeNull();
    expect(events.filter((event) => event.kind === "remote_control")).toEqual([
      expect.objectContaining({ payload: expect.objectContaining({ target_id: "mini", action: "focus_pane", pane_id: "remote:mini:pane:p9" }) }),
    ]);
  });
});
