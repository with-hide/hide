// @vitest-environment jsdom
// PRD hide-external-control B40: the map's centre is the operator on a device's
// map and the tool, by its rail tile's monogram, on an external tool's map.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { emptyScope } from "../test/legacyAgentScope";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { createActions } from "./actions";
import type { ToolCell } from "./agentScope";
import { TooltipProvider } from "./components/ui/tooltip";
import { initializeInterfaceI18n } from "./i18n/instance";
import { OverviewGraph } from "./OverviewGraph";
import type { BoardProject } from "./projectBoard";
import type { Checkout, SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
// The force layout needs a real browser; a placed map with no session points still draws its centre.
vi.mock("./overviewGraphLayout", () => ({
  useOverviewGraphLayout: () => ({ phase: "ready", size: { root: { width: 160, height: 40, orb: 40 }, child: { width: 120, height: 32, orb: 32 }, operator: 64, label: { width: 320, height: 16 }, gap: 8 }, points: new Map(), map: null, reset: () => {}, scale: 1 }),
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

async function centre(tool: ToolCell | null) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  initializeInterfaceI18n("en");
  const agents = [legacyAgentRow({ id: "w", pane_id: "w", identity_label: "work", agent_kind: "claude", session_id: "native-w", group: "working", activity: "working" })];
  const row: Checkout = { id: "alpha-main", workspace_id: "alpha", label: "main", path: "/fixture/alpha", branch: "main", purpose: null, is_worktree: false, exists: true, has_panes: true, pull_request: null, tabs: [], active_tab_id: null, next_tab_label: "Tab 1", strip: [], agent_scope: emptyScope() };
  const projects: BoardProject[] = [{ agents, device: null, workspace: { id: "alpha", label: "alpha", device_id: "local", path: "/fixture/alpha", registered: true, temporary: false, pinned: false, checkouts: [row], inactive_checkouts: { expanded: false, checkout_ids: [] }, agent_scope: { ...emptyScope(), members: [{ pane_id: "w", project_id: "alpha", checkout_id: row.id }] } } }];
  const rest = { navigator: { devices: [{ id: "local", kind: "local", agent_scope: emptyScope() }], workspaces: projects.map((project) => project.workspace), focused_device_id: "local", tools: tool ? [tool] : [] }, status: { remote: [] }, overview_graph: { available: true, limit: 2000, retained_count: 0, truncated: false, retention_ms: 86400000, letters: [], sessions: [{ device_id: "local", pane_id: "w", session_digest: "digest-w" }] } } as unknown as SnapshotRest;
  useShellStore.setState({ rest, agents, focusedPaneId: null, connection: "live", linkSummaries: null });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><OverviewGraph projects={projects} deviceId="local" tool={tool} actions={createActions(() => true)} /></TooltipProvider>));
  const node = container.querySelector<HTMLElement>("[data-graph-center]");
  const drawn = node ? { center: node.dataset.graphCenter, glyph: node.textContent, name: node.getAttribute("aria-label") } : null;
  await act(async () => root.unmount());
  return drawn;
}

it("centres a device's map on the operator and a tool's map on the tool, by its tile's monogram", async () => {
  expect(await centre(null)).toEqual({ center: "operator", glyph: "You", name: "You" });
  const tool: ToolCell = { connection: "c0ffee00c0ffee00", name: "nightly", turn: 0, devices: [] };
  expect(await centre(tool)).toEqual({ center: "c0ffee00c0ffee00", glyph: "N", name: "nightly · External tool" });
});
