// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterAll, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { StartPanelHost } from "./StartPanel";
import { TooltipProvider } from "./components/ui/tooltip";
import type { SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useStartPanel } from "./startDraft";
import { checkoutKey, homeKey } from "./startTargets";
import { useUiStore } from "./ui";

const canvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => canvas.restore());

it("shows a signed-out warning only when the chosen target runs on the machine whose login was checked", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const shell = useShellStore.getState();
  const ui = useUiStore.getState();
  const panel = useStartPanel.getState();
  const remote = "herdr-fixture-named-session";
  const project = { id: "remote-project", label: "new-project", path: "/remote/new-project", device_id: remote, remote_target_id: remote, registered: true, temporary: false, is_git: false, checkouts: [{ id: "remote-checkout", workspace_id: "remote-project", label: "main", path: "/remote/new-project", exists: true, is_primary: true }] };
  const snapshot = {
    navigator: {
      focused_device_id: remote, workspaces: [],
      devices: [
        { id: "core-machine", kind: "local", label: "Core machine", state: "local" },
        { id: remote, kind: "remote", label: "Fixture named session", state: "ready", kit: { busy: false, components: [], agents: [{ id: "claude-code", availability: "available", enabled: true, chosen: true }] } },
      ],
    },
    ui_state: { agent_start: { kind: "claude", models: {} } },
    status: {
      background_ai: { providers: [{ id: "claude", agent: "claude-code", state: "needs_login", selectable: false, models: [] }] },
      remote: [{ target_id: remote, state: "connected", session: { workspaces: [project] } }],
    },
  } as unknown as SnapshotRest;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const warning = () => container.querySelector('[data-start-not-signed-in="claude"]');
  try {
    await act(async () => {
      useShellStore.setState({ rest: snapshot, connection: "live" });
      useUiStore.setState({ screen: { kind: "main" }, overlay: "none", workspaceDialog: null });
      useStartPanel.getState().open(false, { projectId: project.id, kind: "claude" });
      root.render(<TooltipProvider><StartPanelHost actions={createActions(() => true)} /></TooltipProvider>);
    });
    expect(container.querySelector("[data-start-target]")?.getAttribute("data-start-target")).toBe(checkoutKey(remote, project.path));
    expect(container.querySelector("[data-agent-kind]")?.getAttribute("data-agent-kind")).toBe("claude");
    expect(warning()).toBeNull();
    await act(async () => useStartPanel.getState().setTarget(homeKey("core-machine")));
    expect(warning()?.textContent).toContain("Not signed in");
    await act(async () => useStartPanel.getState().setTarget(checkoutKey(remote, project.path)));
    expect(warning()).toBeNull();
  } finally {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(shell, true);
    useUiStore.setState(ui, true);
    useStartPanel.setState(panel, true);
    vi.unstubAllGlobals();
  }
});

it("asks which project and which branch, offers a new branch, and holds a start whose device stopped answering", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  Element.prototype.scrollIntoView = () => {};
  const shell = useShellStore.getState();
  const ui = useUiStore.getState();
  const panel = useStartPanel.getState();
  const events: { kind: string; payload: unknown }[] = [];
  const remote = "mini";
  const checkouts = [
    { id: "c-main", workspace_id: "w-hide", label: "hide", branch: "main", path: "/hide", exists: true, is_primary: true },
    { id: "c-tree", workspace_id: "w-hide", label: "rail", branch: "fix/rail", path: "/hide.worktrees/rail", exists: true, is_worktree: true },
  ];
  const remoteProject = { id: "w-app", label: "app", path: "/srv/app", device_id: remote, remote_target_id: remote, registered: true, temporary: false, checkouts: [{ id: "m-main", workspace_id: "w-app", label: "app", branch: "main", path: "/srv/app", exists: true, is_primary: true }] };
  const snapshot = (connected: boolean) => ({
    navigator: {
      focused_device_id: "core", focused_checkout_id: "c-tree",
      devices: [{ id: "core", kind: "local", label: "This Mac", state: "local" }, { id: remote, kind: "remote", label: "mini", state: "ready" }],
      workspaces: [{ id: "w-hide", label: "hide", path: "/hide", device_id: "core", registered: true, temporary: false, is_git: true, branches: ["main", "dev"], checkouts }],
    },
    ui_state: { agent_start: { kind: "claude", models: {} } },
    status: { remote: [{ target_id: remote, state: connected ? "connected" : "not_connected", session: connected ? { workspaces: [remoteProject] } : null }] },
  }) as unknown as SnapshotRest;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const q = (selector: string) => document.querySelector<HTMLElement>(selector);
  const press = async (selector: string) => act(async () => q(selector)!.click());
  try {
    await act(async () => {
      useShellStore.setState({ rest: snapshot(true), connection: "live" });
      useUiStore.setState({ screen: { kind: "workspace" }, overlay: "none", workspaceDialog: null });
      useStartPanel.getState().open(false);
      root.render(<TooltipProvider><StartPanelHost actions={createActions((event) => { events.push(event as never); return true; })} /></TooltipProvider>);
    });
    // The checkout in front is where the start goes: its project, then its branch.
    expect(q("[data-start-project]")?.textContent).toBe("hide");
    expect(q("[data-start-branch]")?.getAttribute("data-start-branch")).toBe(checkoutKey("core", "/hide.worktrees/rail"));
    expect(q("[data-start-branch]")?.textContent).toBe("fix/rail");
    expect(q("[data-device-chip]")).toBeNull();

    await press("[data-start-branch]");
    const rows = [...document.querySelectorAll("[data-start-branch-menu] [data-start-target-option]")].map((row) => row.textContent);
    expect(rows).toEqual(["mainmain", "fix/railcurrent"]);
    expect(q("[data-start-new-branch]")).not.toBeNull();

    // A new branch takes the branch chip's place until it is named or given up.
    await press("[data-start-new-branch]");
    expect(q("[data-start-branch]")).toBeNull();
    expect(q("[data-start-submit]")).toHaveProperty("disabled", true);
    await press("[data-start-cancel-new-branch]");
    expect(q("[data-start-branch]")?.getAttribute("data-start-branch")).toBe(checkoutKey("core", "/hide"));

    // Another device's project shows that device, and a device that stops answering keeps the pick and locks the start.
    await act(async () => useStartPanel.getState().setTarget(checkoutKey(remote, "/srv/app")));
    expect(q("[data-device-chip]")?.textContent).toBe("mini");
    expect(q("[data-start-submit]")).toHaveProperty("disabled", false);
    await act(async () => useShellStore.setState({ rest: snapshot(false) }));
    expect(q("[data-start-target]")?.getAttribute("data-start-target")).toBe(checkoutKey(remote, "/srv/app"));
    expect(q("[data-start-offline]")?.textContent).toContain("mini · not connected");
    expect(q("[data-start-submit]")).toHaveProperty("disabled", true);
    await press("[data-start-reconnect]");
    expect(events.at(-1)).toMatchObject({ kind: "retry_connect", payload: { target_id: remote } });
  } finally {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(shell, true);
    useUiStore.setState(ui, true);
    useStartPanel.setState(panel, true);
    vi.unstubAllGlobals();
  }
});
