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
  { front: "local", device: "local", modal: false, overflow: true },
  ...(["checking", "unconfirmed", "confirmed"] as const).map((connection) => ({ front: "local", device: "local", modal: false, connection })),
])("opens a $device session from Overview with $front in front (modal=$modal, overflow=$overflow)", async ({ front, device, modal, overflow = false, connection }: { front: string; device: string; modal: boolean; overflow?: boolean; connection?: "checking" | "unconfirmed" | "confirmed" }) => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  initializeInterfaceI18n("en");
  const pane = device === "local" ? "p1" : "remote:mini:pane:p1";
  const agent = legacyAgentRow({ id: pane, pane_id: pane, device_id: device, identity_label: "Current work", record_connection: connection, record_summary: connection ? "Retained conversation summary" : undefined, agent_kind: "claude", session_id: "native-current", group: "working", activity: "working" });
  const checkout: Checkout = { id: "checkout", workspace_id: "project", label: "main", path: "/fixture/project", branch: "main", purpose: null, is_worktree: false, exists: true, has_panes: true, pull_request: null, tabs: [], active_tab_id: null, next_tab_label: "Tab 1", strip: [], agent_scope: emptyScope() };
  const workspace: Workspace = { id: "project", label: "Project", device_id: device, path: "/fixture/project", registered: true, temporary: false, pinned: false, checkouts: [checkout], inactive_checkouts: { expanded: false, checkout_ids: [] }, agent_scope: { ...emptyScope(), members: [{ pane_id: pane, project_id: "project", checkout_id: checkout.id }] } };
  const project: BoardProject = { workspace, agents: [agent], device: device === "local" ? null : "Mini" };
  const projects = overflow ? Array.from({ length: 401 }, (_, index): BoardProject => {
    const id = `project-${String(index).padStart(3, "0")}`;
    const paneId = `pane-${index}`;
    const row = { ...agent, id: paneId, pane_id: paneId, identity_label: `Session ${index}`, session_id: `native-${index}` };
    return { ...project, agents: [row], workspace: { ...workspace, id, label: id, checkouts: [{ ...checkout, workspace_id: id }], agent_scope: { ...emptyScope(), members: [{ pane_id: paneId, project_id: id, checkout_id: checkout.id }] } } };
  }) : [project];
  const rest = { navigator: { devices: [{ id: "local", kind: "local" }, { id: "mini", kind: "remote" }], workspaces: [workspace], focused_device_id: front }, status: { remote: [{ target_id: "mini", state: "connected" }] }, overview_graph: { available: true, limit: 2000, retained_count: 0, truncated: false, retention_ms: 86400000, letters: [], sessions: [{ device_id: device, pane_id: pane, session_digest: "confirmed-current", launch_model: null }] } } as unknown as SnapshotRest;
  useShellStore.setState({ rest, agents: projects.flatMap((project) => project.agents), focusedPaneId: overflow ? null : pane, connection: "live", linkSummaries: null });
  useUiStore.getState().setOverview({ projectId: null, panel: null });
  useUiStore.setState({ overviewOpen: modal, opening: null, screen: modal ? { kind: "workspace" } : { kind: "main", deviceId: front } });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => root.render(<TooltipProvider><OverviewGraph projects={projects} deviceId={device} actions={actions} /></TooltipProvider>));
    if (connection) {
      const row = container.querySelector("[data-graph-session-list] > button")!;
      expect(row.getAttribute("aria-label")).toContain("Retained conversation summary");
      expect(row.querySelector("[data-record-connection]")?.getAttribute("data-record-connection") ?? null).toBe(connection === "confirmed" ? null : connection);
    }
    // B4/D09: a matching confirmed entry highlights without opening a panel.
    expect(container.querySelector("[data-graph-preview]")).toBeNull();
    expect(container.querySelector("[data-graph-session-list] > button")?.getAttribute("aria-pressed")).toBe(String(!overflow && front === device));
    if (overflow) {
      const search = container.querySelector<HTMLInputElement>("[data-graph-session-list] input")!;
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(search, "Session 400");
        search.dispatchEvent(new Event("input", { bubbles: true }));
      });
      await act(async () => container.querySelector<HTMLButtonElement>("[data-graph-session-list] > button")!.click());
      expect(useUiStore.getState().overview.projectId).toBe("project-400");
      expect(container.querySelector("[data-graph-preview]")?.textContent).toContain("Session 400");
      expect(container.querySelector("[data-graph-session-list] > button")?.getAttribute("aria-pressed")).toBe("true");
      expect(events).toHaveLength(0);
      await act(async () => useUiStore.getState().setOverview({ projectId: null }));
      await act(async () => useUiStore.getState().setOverview({ projectId: "project-400" }));
      expect(container.querySelector("[data-graph-preview]")).toBeNull();
      return;
    }
    await act(async () => container.querySelector<HTMLButtonElement>("[data-graph-session-list] button")!.click());
    if (connection) expect(container.querySelector("[data-graph-preview] [data-record-connection]")?.getAttribute("data-record-connection") ?? null).toBe(connection === "confirmed" ? null : connection);
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
