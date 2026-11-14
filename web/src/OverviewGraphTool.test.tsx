// @vitest-environment jsdom
// PRD hide-external-control B40: a tool's map is the tool's work only. Its
// centre is the tool, by its rail tile's monogram, where a device's map draws
// the operator with a `+`; the missions it lists wait on the tool, not the
// operator; and nothing on it starts the operator's work.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { emptyScope } from "../test/legacyAgentScope";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { createActions } from "./actions";
import type { ToolCell } from "./agentScope";
import { TooltipProvider } from "./components/ui/tooltip";
import { missionFixture } from "./gallery/MissionScene";
import { initializeInterfaceI18n } from "./i18n/instance";
import { OverviewGraph } from "./OverviewGraph";
import { sameGraphScope } from "./overviewGraphModel";
import type { BoardProject } from "./projectBoard";
import type { Checkout, SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
// The force layout needs a real browser; a placed map with no session points still draws its centre. Whether
// the layout makes room for the start circles and Add project is what the map tells it.
const layout = vi.hoisted(() => ({ starts: [] as boolean[] }));
vi.mock("./overviewGraphLayout", () => ({
  useOverviewGraphLayout: (...args: unknown[]) => {
    layout.starts.push(args[4] !== false);
    return { phase: "ready", size: { root: { width: 160, height: 40, orb: 40 }, child: { width: 120, height: 32, orb: 32 }, operator: 64, label: { width: 320, height: 16 }, gap: 8 }, points: new Map(), map: null, reset: () => {}, scale: 1 };
  },
  useGraphTheme: () => "light",
}));

const savedShell = useShellStore.getState();
const savedUi = useUiStore.getState();
afterEach(() => {
  useShellStore.setState(savedShell, true);
  useUiStore.setState(savedUi, true);
  vi.unstubAllGlobals();
  document.body.replaceChildren();
  layout.starts = [];
});

const TOOL: ToolCell = { connection: "c0ffee00c0ffee00", name: "nightly", turn: 1, devices: [], mission_asks: ["mission-tool"] };

async function drawn(tool: ToolCell | null, working: boolean) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  initializeInterfaceI18n("en");
  const agents = working ? [legacyAgentRow({ id: "w", pane_id: "w", identity_label: "work", agent_kind: "claude", session_id: "native-w", group: "working", activity: "working" })] : [];
  const row: Checkout = { id: "alpha-main", workspace_id: "alpha", label: "main", path: "/fixture/alpha", branch: "main", purpose: null, is_worktree: false, exists: true, has_panes: working, pull_request: null, tabs: [], active_tab_id: null, next_tab_label: "Tab 1", strip: [], agent_scope: emptyScope() };
  const projects: BoardProject[] = [{ agents, device: null, workspace: { id: "alpha", label: "alpha", device_id: "local", path: "/fixture/alpha", registered: true, temporary: false, pinned: false, checkouts: [row], inactive_checkouts: { expanded: false, checkout_ids: [] }, agent_scope: { ...emptyScope(), members: agents.map((agent) => ({ pane_id: agent.pane_id, project_id: "alpha", checkout_id: row.id })) } } }];
  // The device's own mission asks the operator; the tool's asks the tool.
  const { mission } = missionFixture("no-conductor", Date.UTC(2026, 10, 1));
  const missions = [{ ...mission, id: "mission-operator", title: "operator's", workspace_id: "alpha" }, { ...mission, id: "mission-tool", title: "tool's", workspace_id: "alpha" }];
  const rest = { navigator: { devices: [{ id: "local", kind: "local", agent_scope: { ...emptyScope(), mission_asks: ["mission-operator"] } }], workspaces: projects.map((project) => project.workspace), focused_device_id: "local", tools: tool ? [tool] : [] }, status: { remote: [] }, missions: { items: missions, failed: false }, overview_graph: { available: true, limit: 2000, retained_count: 0, truncated: false, retention_ms: 86400000, letters: [], sessions: agents.map((agent) => ({ device_id: "local", pane_id: agent.pane_id, session_digest: `digest-${agent.pane_id}` })) } } as unknown as SnapshotRest;
  useShellStore.setState({ rest, agents, focusedPaneId: null, connection: "live", linkSummaries: null });
  useUiStore.getState().setOverview({ projectId: null, panel: null });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><OverviewGraph projects={projects} deviceId="local" tool={tool} actions={createActions(() => true)} /></TooltipProvider>));
  const centre = container.querySelector<HTMLElement>("[data-graph-center]");
  const picture = centre?.querySelector<HTMLElement>('[role="img"]');
  const seen = {
    centre: centre && picture ? { id: centre.dataset.graphCenter, glyph: picture.textContent, name: picture.getAttribute("aria-label"), start: centre.querySelector("[data-graph-operator-start]") !== null } : null,
    asks: [...container.querySelectorAll<HTMLElement>("[data-graph-mission-ask]")].map((item) => item.dataset.graphMissionAsk),
    heading: container.querySelector(".og-waiting h3")?.firstChild?.textContent?.trim() ?? null,
    starts: container.querySelectorAll("[data-graph-project-row-start], [data-graph-welcome-action='start_agent'], [data-graph-empty-action='start_agent']").length,
    reserved: layout.starts.at(-1) ?? null,
  };
  await act(async () => root.unmount());
  return seen;
}

it("centres a device's map on the operator with its + and a tool's map on the tool's letter with none", async () => {
  expect((await drawn(null, true)).centre).toEqual({ id: "operator", glyph: "You", name: "You", start: true });
  expect((await drawn(TOOL, true)).centre).toEqual({ id: TOOL.connection, glyph: "N", name: "nightly · External tool", start: false });
});

it("lists the missions waiting on the tool in its map, and the operator's only in the device's", async () => {
  for (const working of [true, false]) {
    expect(await drawn(null, working).then(({ asks, heading }) => ({ asks, heading })), `device, drawn ${working}`).toEqual({ asks: ["mission-operator"], heading: "Waiting on you" });
    expect(await drawn(TOOL, working).then(({ asks, heading }) => ({ asks, heading })), `tool, drawn ${working}`).toEqual({ asks: ["mission-tool"], heading: "Tool's turn" });
  }
});

it("starts nothing from a tool's map: no project +, no New agent, no start circles or Add project place", async () => {
  for (const working of [true, false]) {
    const device = await drawn(null, working);
    expect([device.starts, device.reserved], `device, drawn ${working}`).toEqual([1, true]);
    const tool = await drawn(TOOL, working);
    expect([tool.starts, tool.reserved], `tool, drawn ${working}`).toEqual([0, false]);
  }
});

it("keeps a tool's map places apart from the device's map of the same machine", () => {
  const device = { kind: "all" as const, device_id: "local" };
  const tool = { ...device, tool: TOOL.connection };
  expect([sameGraphScope(device, device), sameGraphScope(tool, tool), sameGraphScope(device, tool), sameGraphScope(tool, { ...device, tool: "beef0000beef0000" })]).toEqual([true, true, false, false]);
});
