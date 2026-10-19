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
