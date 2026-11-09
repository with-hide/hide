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
import { checkoutKey } from "./startTargets";
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
      focused_device_id: remote,
      workspaces: [{ id: "local-project", label: "local", path: "/local/p", device_id: "core-machine", registered: true, temporary: false, is_git: false, checkouts: [{ id: "local-checkout", workspace_id: "local-project", label: "local", path: "/local/p", exists: true, is_primary: true }] }],
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
    await act(async () => useStartPanel.getState().setTarget(checkoutKey("core-machine", "/local/p")));
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

it("offers Add a project in place of Start when the device has no project, and opens it on that device (PRD retire-hide-home B1)", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  const shell = useShellStore.getState();
  const ui = useUiStore.getState();
  const panel = useStartPanel.getState();
  const snapshot = {
    navigator: { focused_device_id: "core", devices: [{ id: "core", kind: "local", label: "This Mac", state: "local" }], workspaces: [] },
    ui_state: { agent_start: { kind: "claude", models: {} } },
    status: {},
  } as unknown as SnapshotRest;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const q = (selector: string) => document.querySelector<HTMLElement>(selector);
  try {
    await act(async () => {
      useShellStore.setState({ rest: snapshot, connection: "live" });
      useUiStore.setState({ screen: { kind: "main" }, overlay: "none", workspaceDialog: null, addProjectDevice: null });
      useStartPanel.getState().open(false);
      root.render(<TooltipProvider><StartPanelHost actions={createActions(() => true)} /></TooltipProvider>);
    });
    expect(q("[data-start-submit]")).toBeNull();
    expect(q("[data-start-add-project]")?.textContent).toBe("Add project");
    expect(q("[data-start-target]")?.getAttribute("data-start-target")).toBe("");
    await act(async () => q("[data-start-add-project]")!.click());
    expect(useUiStore.getState().overlay).toBe("add_project");
    expect(useUiStore.getState().addProjectDevice).toBe("core");
    expect(useStartPanel.getState().isOpen).toBe(false);
  } finally {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(shell, true);
    useUiStore.setState(ui, true);
    useStartPanel.setState(panel, true);
    vi.unstubAllGlobals();
  }
});

it("shows the profile the core would pick, the project's first, and sends a profile only when the operator picks one (agent-profiles B1, B14)", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  Element.prototype.scrollIntoView = () => {};
  const shell = useShellStore.getState();
  const ui = useUiStore.getState();
  const panel = useStartPanel.getState();
  const events: { kind: string; payload: Record<string, unknown> }[] = [];
  const listed = (id: string, name: string) => ({ id, kind: "claude-code", name, account: `/home/me/accounts/${name}`, program: null, env_keys: [], version: 1, install: "own", folder: null, hook: null, skill: null, startable: true });
  const snapshot = (profiles: unknown, projectProfile: string | null, automaticProfile: string) => ({
    navigator: {
      focused_device_id: "core-machine",
      workspaces: [{ id: "blog", label: "blog", path: "/local/blog", device_id: "core-machine", registered: true, temporary: false, is_git: false, checkouts: [{ id: "blog-main", workspace_id: "blog", label: "main", path: "/local/blog", exists: true, is_primary: true }] }],
      devices: [{ id: "core-machine", kind: "local", label: "Core machine", state: "local", kit: { busy: false, components: [], agents: [{ id: "claude-code", label: "Claude Code", availability: "available", enabled: true, chosen: true }], profiles } }],
    },
    ui_state: {
      agent_start: { kind: "claude", models: {} },
      project_settings: projectProfile ? { blog: { version: 1, device_id: "core-machine", root: "/local/blog", agent_profiles: { "claude-code": projectProfile } } } : {},
    },
    status: { start_profiles: { blog: { device_id: "core-machine", root: "/local/blog", profiles: { "claude-code": automaticProfile } } } },
  }) as unknown as SnapshotRest;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const q = (selector: string) => document.body.querySelector<HTMLElement>(selector);
  const started = () => events.filter((event) => event.kind === "agent_start_in_checkout").map((event) => event.payload);
  const render = async (rest: SnapshotRest) => {
    await act(async () => {
      useShellStore.setState({ rest, connection: "live" });
      useUiStore.setState({ screen: { kind: "main" }, overlay: "none", workspaceDialog: null });
      useStartPanel.setState(panel, true);
      useStartPanel.getState().open(false, { projectId: "blog", kind: "claude" });
      root.render(<TooltipProvider><StartPanelHost actions={createActions((event) => { events.push(event as never); return true; })} /></TooltipProvider>);
    });
  };
  try {
    // No profile on the machine: no menu, and the start names none.
    await render(snapshot({ state: "listed", version: 1, profiles: [], defaults: {} }, null, "default"));
    expect(q("[data-profile-pick]")).toBeNull();
    await act(async () => q("[data-start-submit]")!.click());
    expect(started().at(-1)).not.toHaveProperty("profile");
    await act(async () => root.render(<></>));

    const profiles = { state: "listed", version: 2, profiles: [listed("p-work", "work"), listed("p-side", "side")], defaults: { "claude-code": "p-side" } };
    await render(snapshot(profiles, "p-work", "p-work"));
    // The project's choice is shown chosen and sent as no choice at all.
    expect(q("[data-profile-pick]")?.dataset.profilePick).toBe("p-work");
    await act(async () => q("[data-start-submit]")!.click());
    expect(started().at(-1)).not.toHaveProperty("profile");
    await act(async () => root.render(<></>));

    // A project that names the built-in profile opens on it, though the agent's default is side.
    await render(snapshot(profiles, "default", "default"));
    expect(q("[data-profile-pick]")?.dataset.profilePick).toBe("default");
    await act(async () => root.render(<></>));

    await render(snapshot(profiles, null, "p-side"));
    expect(q("[data-profile-pick]")?.dataset.profilePick).toBe("p-side");
    const trigger = q("[data-profile-pick]")!;
    trigger.focus();
    await act(async () => { trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
    expect([...document.querySelectorAll("[data-profile-pick-option]")].map((option) => option.getAttribute("data-profile-pick-option"))).toEqual(["default", "p-work", "p-side"]);
    const option = q('[data-profile-pick-option="p-work"]')!;
    option.focus();
    await act(async () => { option.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
    expect(q("[data-profile-pick]")?.dataset.profilePick).toBe("p-work");
    await act(async () => q("[data-start-submit]")!.click());
    expect(started().at(-1)).toMatchObject({ checkout_path: "/local/blog", provider: "claude", profile: "p-work" });
  } finally {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(shell, true);
    useUiStore.setState(ui, true);
    useStartPanel.setState(panel, true);
    vi.unstubAllGlobals();
  }
});

it("shows the core's choice for another project, and opens again on the core's choice rather than the last pick (agent-profiles I-1.8)", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  Element.prototype.scrollIntoView = () => {};
  const shell = useShellStore.getState();
  const ui = useUiStore.getState();
  const panel = useStartPanel.getState();
  const listed = (id: string, name: string) => ({ id, kind: "claude-code", name, account: `/home/me/accounts/${name}`, program: null, env_keys: [], version: 1, install: "own", folder: null, hook: null, skill: null, startable: true });
  const project = (id: string) => ({ id, label: id, path: `/local/${id}`, device_id: "core-machine", registered: true, temporary: false, is_git: false, checkouts: [{ id: `${id}-main`, workspace_id: id, label: "main", path: `/local/${id}`, exists: true, is_primary: true }] });
  const rest = {
    navigator: {
      focused_device_id: "core-machine",
      workspaces: [project("blog"), project("docs")],
      devices: [{ id: "core-machine", kind: "local", label: "Core machine", state: "local", kit: { busy: false, components: [], agents: [{ id: "claude-code", label: "Claude Code", availability: "available", enabled: true, chosen: true }], profiles: { state: "listed", version: 2, profiles: [listed("p-work", "work"), listed("p-side", "side")], defaults: {} } } }],
    },
    ui_state: { agent_start: { kind: "claude", models: {} } },
    status: {
      start_profiles: {
        blog: { device_id: "core-machine", root: "/local/blog", profiles: { "claude-code": "p-side" } },
        docs: { device_id: "core-machine", root: "/local/docs", profiles: { "claude-code": "default" } },
      },
    },
  } as unknown as SnapshotRest;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const q = (selector: string) => document.body.querySelector<HTMLElement>(selector);
  const open = async () => {
    await act(async () => {
      useShellStore.setState({ rest, connection: "live" });
      useUiStore.setState({ screen: { kind: "main" }, overlay: "none", workspaceDialog: null });
      useStartPanel.setState(panel, true);
      useStartPanel.getState().open(false, { projectId: "blog", kind: "claude" });
      root.render(<TooltipProvider><StartPanelHost actions={createActions(() => true)} /></TooltipProvider>);
    });
  };
  try {
    await open();
    const trigger = q("[data-profile-pick]")!;
    trigger.focus();
    await act(async () => { trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
    const option = q('[data-profile-pick-option="p-work"]')!;
    option.focus();
    await act(async () => { option.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
    expect(q("[data-profile-pick]")?.dataset.profilePick).toBe("p-work");
    // Another project shows what the core would pick there, not the earlier pick.
    await act(async () => useStartPanel.getState().setTarget(checkoutKey("core-machine", "/local/docs")));
    expect(q("[data-profile-pick]")?.dataset.profilePick).toBe("default");
    // A panel opened again starts from the core's choice, not the last pick.
    await act(async () => root.render(<></>));
    await open();
    expect(q("[data-profile-pick]")?.dataset.profilePick).toBe("p-side");
  } finally {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(shell, true);
    useUiStore.setState(ui, true);
    useStartPanel.setState(panel, true);
    vi.unstubAllGlobals();
  }
});
