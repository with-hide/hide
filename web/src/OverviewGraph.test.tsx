// @vitest-environment jsdom
// B5: opening a session leaves either Overview surface and uses device-aware navigation.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { emptyScope } from "../test/legacyAgentScope";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { initializeInterfaceI18n } from "./i18n/instance";
import { OverviewGraph } from "./OverviewGraph";
import type { BoardProject } from "./projectBoard";
import type { Checkout, SnapshotRest, Workspace } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import type { DispatchFn } from "./ws";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
// Layout failure is a supported surface with the same session preview actions.
vi.mock("./overviewGraphLayout", () => ({
  useOverviewGraphLayout: () => ({ key: "fixture", phase: "failed", placement: null }),
  useGraphReducedMotion: () => true,
  useGraphTheme: () => "light",
}));
const savedShell = useShellStore.getState();
const savedUi = useUiStore.getState();
afterEach(() => {
  useShellStore.setState(savedShell, true);
  useUiStore.setState(savedUi, true);
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

it.each([
  { front: "local", device: "local", modal: false }, { front: "local", device: "local", modal: true },
  { front: "local", device: "mini", modal: false }, { front: "local", device: "mini", modal: true },
  { front: "mini", device: "local", modal: false }, { front: "mini", device: "local", modal: true },
  { front: "mini", device: "mini", modal: false }, { front: "mini", device: "mini", modal: true },
])("opens a $device session from Overview with $front in front (modal=$modal)", async ({ front, device, modal }) => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  initializeInterfaceI18n("en");
  const pane = device === "local" ? "p1" : "remote:mini:pane:p1";
  const agent = legacyAgentRow({ id: pane, pane_id: pane, device_id: device, identity_label: "Current work", agent_kind: "claude", session_id: "native-current", group: "working", activity: "working" });
  const checkout: Checkout = { id: "checkout", workspace_id: "project", label: "main", path: "/fixture/project", branch: "main", purpose: null, is_worktree: false, exists: true, has_panes: true, pull_request: null, tabs: [], active_tab_id: null, next_tab_label: "Tab 1", strip: [], agent_scope: emptyScope() };
  const workspace: Workspace = { id: "project", label: "Project", device_id: device, path: "/fixture/project", registered: true, temporary: false, pinned: false, checkouts: [checkout], inactive_checkouts: { expanded: false, checkout_ids: [] }, agent_scope: { ...emptyScope(), members: [{ pane_id: pane, project_id: "project", checkout_id: checkout.id }] } };
  const project: BoardProject = { workspace, agents: [agent], device: device === "local" ? null : "Mini" };
  const rest = { navigator: { devices: [{ id: "local", kind: "local" }, { id: "mini", kind: "remote" }], workspaces: [workspace], focused_device_id: front }, status: { remote: [{ target_id: "mini", state: "connected" }] } } as unknown as SnapshotRest;
  useShellStore.setState({ rest, agents: [agent], connection: "live", linkSummaries: null });
  useUiStore.getState().setOverview({ projectId: null, panel: null });
  useUiStore.setState({ overviewOpen: modal, opening: null, screen: modal ? { kind: "workspace" } : { kind: "main", deviceId: front } });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => root.render(<TooltipProvider><OverviewGraph projects={[project]} deviceId={device} actions={actions} /></TooltipProvider>));
    await act(async () => container.querySelector<HTMLButtonElement>("[data-graph-session-list] button")!.click());
    const open = [...container.querySelectorAll<HTMLButtonElement>("[data-graph-preview] button")].find((button) => button.textContent === "Open session")!;
    expect(open).toBeDefined();
    await act(async () => open.click());
    expect(useUiStore.getState().overviewOpen).toBe(false);
    expect(useUiStore.getState().opening?.target).toEqual({ paneId: pane });
    expect(events).toHaveLength(1);
    expect(events[0]).toMatchObject(device === "local"
      ? { kind: "focus_pane", payload: { pane_id: pane, origin: "operator", focus_device: front !== device } }
      : { kind: "remote_control", payload: { target_id: "mini", action: "focus_pane", pane_id: pane } });
    expect(Boolean((events[0]!.payload as { focus_device?: boolean }).focus_device)).toBe(front !== device);
  } finally { await act(async () => root.unmount()); }
});
