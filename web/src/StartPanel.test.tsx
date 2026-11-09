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

const profileFixture = {
  listed: (id: string, name: string) => ({ id, kind: "claude-code", name, account: `/home/me/accounts/${name}`, program: null, env_keys: [], version: 1, install: "own", folder: null, hook: null, skill: null, startable: true }),
  project: (id: string) => ({ id, label: id, path: `/local/${id}`, device_id: "core-machine", registered: true, temporary: false, is_git: false, checkouts: [{ id: `${id}-main`, workspace_id: id, label: "main", path: `/local/${id}`, exists: true, is_primary: true }] }),
};

/** A core machine with projects `blog` and `docs`, the given profiles and the core's automatic choice per project. */
function profileSnapshot(profiles: unknown, automatic: Record<string, string>, projectProfile: string | null = null) {
  return {
    navigator: {
      focused_device_id: "core-machine",
      workspaces: [profileFixture.project("blog"), profileFixture.project("docs")],
      devices: [{ id: "core-machine", kind: "local", label: "Core machine", state: "local", kit: { busy: false, components: [], agents: [{ id: "claude-code", label: "Claude Code", availability: "available", enabled: true, chosen: true }], profiles } }],
    },
    ui_state: {
      agent_start: { kind: "claude", models: {} },
      project_settings: projectProfile ? { blog: { version: 1, device_id: "core-machine", root: "/local/blog", agent_profiles: { "claude-code": projectProfile } } } : {},
    },
    status: { start_profiles: Object.fromEntries(Object.entries(automatic).map(([id, profile]) => [id, { device_id: "core-machine", root: `/local/${id}`, profiles: { "claude-code": profile } }])) },
  } as unknown as SnapshotRest;
}

const weeklyLimits = (used: number) => ({ state: "reading", freshness: "fresh", observed_at_ms: Date.now(), value: { windows: [{ kind: "weekly", scope: { kind: "all" }, outcome: { state: "open", used_percent: used, resets_at_s: Date.now() / 1000 + 3600 } }] } });

async function withPanel(run: (panel: { render: (rest: SnapshotRest, project?: string) => Promise<void>; events: { kind: string; payload: Record<string, unknown> }[]; q: (selector: string) => HTMLElement | null; openItems: () => Promise<void>; pickItem: (key: string) => Promise<void>; root: ReturnType<typeof createRoot> }) => Promise<void>) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  Element.prototype.scrollIntoView = () => {};
  const shell = useShellStore.getState();
  const ui = useUiStore.getState();
  const panel = useStartPanel.getState();
  const events: { kind: string; payload: Record<string, unknown> }[] = [];
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const q = (selector: string) => document.body.querySelector<HTMLElement>(selector);
  const key = async (element: HTMLElement) => {
    element.focus();
    await act(async () => { element.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
  };
  try {
    await run({
      root,
      events,
      q,
      render: async (rest, project = "blog") => {
        await act(async () => {
          useShellStore.setState({ rest, connection: "live" });
          useUiStore.setState({ screen: { kind: "main" }, overlay: "none", workspaceDialog: null });
          useStartPanel.setState(panel, true);
          useStartPanel.getState().open(false, { projectId: project, kind: "claude" });
          root.render(<TooltipProvider><StartPanelHost actions={createActions((event) => { events.push(event as never); return true; })} /></TooltipProvider>);
        });
      },
      openItems: () => key(q("[data-agent-item], [data-agent-kind]")!),
      pickItem: (item) => key(q(`[data-agent-item-option="${item}"]`)!),
    });
  } finally {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(shell, true);
    useUiStore.setState(ui, true);
    useStartPanel.setState(panel, true);
    vi.unstubAllGlobals();
  }
}

it("lists each agent with its profiles as items, shows the core's choice chosen and starts the item shown, the built-in one included (agent-profiles-phase2 B1-B5)", async () => {
  await withPanel(async ({ render, events, q, openItems, pickItem, root }) => {
    const started = () => events.filter((event) => event.kind === "agent_start_in_checkout").map((event) => event.payload);
    // No profile on the machine: the agent names alone, no profile menu, no figure, and the start names none (B1).
    await render(profileSnapshot({ state: "listed", version: 1, profiles: [], defaults: {} }, { blog: "default" }));
    expect(q("[data-profile-pick]")).toBeNull();
    await openItems();
    expect(document.querySelectorAll("[data-slot=select-item-trailing]")).toHaveLength(0);
    expect(q('[data-agent-kind-option="claude"]')?.textContent).toBe("Claude");
    await act(async () => q("[data-start-submit]")!.click());
    expect(started().at(-1)).not.toHaveProperty("profile");
    await act(async () => root.render(<></>));

    // Profiles on the machine: one item each, the project's work chosen, no profile menu beside the agent (B2).
    const profiles = { state: "listed", version: 2, profiles: [profileFixture.listed("p-work", "work"), profileFixture.listed("p-side", "side")], defaults: { "claude-code": "p-side" } };
    await render(profileSnapshot(profiles, { blog: "p-work" }, "p-work"));
    expect(q("[data-profile-pick]")).toBeNull();
    expect(q("[data-agent-item]")?.getAttribute("data-agent-item")).toBe("claude/p-work");
    await openItems();
    const claude = [...document.querySelectorAll("[data-agent-item-option^='claude/']")].map((option) => option.getAttribute("data-agent-item-option"));
    expect(claude).toEqual(["claude/default", "claude/p-work", "claude/p-side"]);
    expect(q('[data-agent-item-option="claude/p-side"]')?.textContent).toContain("Claude Code · side");
    // Without a reading every item says it does not know (B3).
    expect(q('[data-agent-item-option="claude/p-work"]')?.textContent).toContain("—");
    // The chosen item starts as shown, though nothing was picked (D-02, B5).
    await pickItem("claude/p-work");
    await act(async () => q("[data-start-submit]")!.click());
    expect(started().at(-1)).toMatchObject({ checkout_path: "/local/blog", provider: "claude", profile: "p-work" });
    await act(async () => root.render(<></>));

    // `Claude Code` is the built-in account whatever the project preselected (B4).
    await render(profileSnapshot(profiles, { blog: "p-work" }, "p-work"));
    await openItems();
    await pickItem("claude/default");
    expect(q("[data-agent-item]")?.getAttribute("data-agent-item")).toBe("claude/default");
    await act(async () => q("[data-start-submit]")!.click());
    expect(started().at(-1)).toMatchObject({ provider: "claude", profile: "default" });
  });
});

it("shows each item's account usage left, the same figure on two items of one account, and a dash where it is not known (agent-profiles-phase2 B3, D-11)", async () => {
  await withPanel(async ({ render, q, openItems }) => {
    const profiles = { state: "listed", version: 2, profiles: [profileFixture.listed("p-work", "work"), profileFixture.listed("p-also", "also"), profileFixture.listed("p-gate", "gate")], defaults: {} };
    useShellStore.setState({
      usage: { nodes: [{ node: "core-machine", agents: [{ agent: "claude", limits: weeklyLimits(60), consumption: { state: "unsupported" }, profiles: [{ id: "p-work", limits: weeklyLimits(28) }, { id: "p-also", shared: true, limits: weeklyLimits(28) }, { id: "p-gate", limits: { state: "unavailable", cause: "no_account", last_checked_at_ms: null } }] }] }] },
    } as never);
    try {
      await render(profileSnapshot(profiles, { blog: "default" }));
      await openItems();
      const text = (item: string) => q(`[data-agent-item-option="claude/${item}"]`)?.textContent ?? "";
      expect(text("default")).toContain("40% left");
      expect(text("p-work")).toContain("72% left");
      expect(text("p-also")).toContain("72% left");
      expect(text("p-gate")).toContain("—");
    } finally {
      useShellStore.setState({ usage: null } as never);
    }
  });
});

it("shows the core's choice for another project and opens again on the core's choice rather than the last pick (agent-profiles I-1.8)", async () => {
  await withPanel(async ({ render, q, openItems, pickItem, root }) => {
    const profiles = { state: "listed", version: 2, profiles: [profileFixture.listed("p-work", "work"), profileFixture.listed("p-side", "side")], defaults: {} };
    const rest = profileSnapshot(profiles, { blog: "p-side", docs: "default" });
    await render(rest);
    await openItems();
    await pickItem("claude/p-work");
    expect(q("[data-agent-item]")?.getAttribute("data-agent-item")).toBe("claude/p-work");
    await act(async () => useStartPanel.getState().setTarget(checkoutKey("core-machine", "/local/docs")));
    expect(q("[data-agent-item]")?.getAttribute("data-agent-item")).toBe("claude/default");
    await act(async () => root.render(<></>));
    await render(rest);
    expect(q("[data-agent-item]")?.getAttribute("data-agent-item")).toBe("claude/p-side");
  });
});

it("opens a profile item on its own default model when the remembered model is not in its list, and sends the listed one as it is (agent-profiles-phase2 B5)", async () => {
  await withPanel(async ({ render, events, q, openItems, pickItem, root }) => {
    const profiles = { state: "listed", version: 2, profiles: [profileFixture.listed("p-gate", "gate")], defaults: {} };
    const rest = profileSnapshot(profiles, { blog: "p-gate" }) as unknown as { ui_state: { agent_start: unknown }; status: Record<string, unknown> };
    rest.ui_state.agent_start = { kind: "claude", models: { claude: "opus" } };
    rest.status.background_ai = { providers: [{ id: "claude", agent: "claude-code", state: "ready", selectable: true, models: ["opus", "sonnet"], profile_models: [{ profile: "p-gate", models: ["deepseek-v4", "glm-5"], model_labels: { "deepseek-v4": "DeepSeek V4" }, models_fixed: false }] }] };
    await render(rest as unknown as SnapshotRest);
    const started = () => events.filter((event) => event.kind === "agent_start_in_checkout").map((event) => event.payload);
    // The remembered `opus` is the agent's, not this item's: the item opens on its default.
    await act(async () => q("[data-start-submit]")!.click());
    expect(started().at(-1)).toMatchObject({ profile: "p-gate" });
    expect(started().at(-1)).not.toHaveProperty("model");
    await act(async () => root.render(<></>));
    await render(rest as unknown as SnapshotRest);
    // The item's own models show their labels and are sent by value.
    const model = q("[data-agent-model-kind]");
    expect(model).not.toBeNull();
    expect(model!.hasAttribute("disabled")).toBe(false);
    model!.focus();
    await act(async () => { model!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
    expect([...document.querySelectorAll("[data-agent-model-option]")].map((row) => row.getAttribute("data-agent-model-option"))).toEqual(["", "deepseek-v4", "glm-5"]);
    expect(q('[data-agent-model-option="deepseek-v4"]')?.textContent).toBe("DeepSeek V4");
    const option = q('[data-agent-model-option="deepseek-v4"]')!;
    option.focus();
    await act(async () => { option.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
    await act(async () => q("[data-start-submit]")!.click());
    expect(started().at(-1)).toMatchObject({ profile: "p-gate", model: "deepseek-v4" });
    // The built-in item takes the agent's remembered model again.
    await act(async () => root.render(<></>));
    await render(rest as unknown as SnapshotRest);
    await openItems();
    await pickItem("claude/default");
    await act(async () => q("[data-start-submit]")!.click());
    expect(started().at(-1)).toMatchObject({ profile: "default", model: "opus" });
  });
});
