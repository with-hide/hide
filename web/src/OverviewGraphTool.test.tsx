// @vitest-environment jsdom
// PRD hide-external-control B40: a tool's map is the tool's work only. Its
// centre is the tool, by the bot's owl, where a device's map draws
// the operator with a `+`; the missions it lists wait on the tool, not the
// operator; and nothing on it starts the operator's work.
import { readFileSync } from "node:fs";
import { ReactFlowProvider } from "@xyflow/react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { emptyScope } from "../test/legacyAgentScope";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { createActions } from "./actions";
import type { ToolCell } from "./agentScope";
import { InToolCell } from "./components/tool-mark";
import { TooltipProvider } from "./components/ui/tooltip";
import { missionFixture } from "./gallery/MissionScene";
import { initializeInterfaceI18n } from "./i18n/instance";
import { OverviewGraph } from "./OverviewGraph";
import { sameGraphScope, type GraphAgent } from "./overviewGraphModel";
import { SessionGraphNode } from "./overviewGraphNodes";
import type { BoardProject } from "./projectBoard";
import type { AgentRow, Checkout, SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { toolProjects } from "./toolCells";

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
    centre: centre && picture ? { id: centre.dataset.graphCenter, glyph: picture.querySelector("[data-bot-face]") ? "owl" : picture.textContent, name: picture.getAttribute("aria-label"), start: centre.querySelector("[data-graph-operator-start]") !== null } : null,
    asks: [...container.querySelectorAll<HTMLElement>("[data-graph-mission-ask]")].map((item) => item.dataset.graphMissionAsk),
    heading: container.querySelector(".og-waiting h3")?.firstChild?.textContent?.trim() ?? null,
    starts: container.querySelectorAll("[data-graph-project-row-start], [data-graph-welcome-action='start_agent'], [data-graph-empty-action='start_agent']").length,
    reserved: layout.starts.at(-1) ?? null,
  };
  await act(async () => root.unmount());
  return seen;
}

it("centres a device's map on the operator with its + and a tool's map on the bot's owl with none", async () => {
  expect((await drawn(null, true)).centre).toEqual({ id: "operator", glyph: "You", name: "You", start: true });
  expect((await drawn(TOOL, true)).centre).toEqual({ id: TOOL.connection, glyph: "owl", name: "nightly · Bot", start: false });
});

it("lists the missions waiting on the tool in its map, and the operator's only in the device's", async () => {
  for (const working of [true, false]) {
    expect(await drawn(null, working).then(({ asks, heading }) => ({ asks, heading })), `device, drawn ${working}`).toEqual({ asks: ["mission-operator"], heading: "Waiting on you" });
    expect(await drawn(TOOL, working).then(({ asks, heading }) => ({ asks, heading })), `tool, drawn ${working}`).toEqual({ asks: ["mission-tool"], heading: "Bot's turn" });
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

// Together check, issues 1 and 4. A tool's agent that waits on a person stands in the operator's own
// Overview; its card in `Waiting on you` and its node's line lead with the tool's name, as its sidebar row does.
// In the tool's own cell the filter says the cell's words and counts what the cell's sidebar counts.
const NIGHTLY = { connection: TOOL.connection, name: "nightly", owns: true };
const WAITING_ON_PERSON = legacyAgentRow({ id: "h", pane_id: "h", identity_label: "helper", agent_kind: "claude", session_id: "native-h", group: "needs_you", activity: "stopped", detail: "Sign-in · github.com/login", tool: NIGHTLY, human_turn: { id: "agent-3.1", kind: "auth", place: "github.com/login", pane_id: "h", title: "helper", since_unix_ms: 1, own: true } });
const ASKING_TOOL = legacyAgentRow({ id: "q", pane_id: "q", identity_label: "asker", agent_kind: "claude", session_id: "native-q", group: "seen", activity: "stopped", mark: "waiting", demand: "question", tool: NIGHTLY });

async function map(agents: AgentRow[], tool: ToolCell | null) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  initializeInterfaceI18n("en");
  const row: Checkout = { id: "alpha-main", workspace_id: "alpha", label: "main", path: "/fixture/alpha", branch: "main", purpose: null, is_worktree: false, exists: true, has_panes: true, pull_request: null, tabs: [], active_tab_id: null, next_tab_label: "Tab 1", strip: [], agent_scope: emptyScope() };
  const members = agents.map((agent) => ({ pane_id: agent.pane_id, project_id: "alpha", checkout_id: row.id }));
  const device: BoardProject[] = [{ agents, device: null, workspace: { id: "alpha", label: "alpha", device_id: "local", path: "/fixture/alpha", registered: true, temporary: false, pinned: false, checkouts: [row], inactive_checkouts: { expanded: false, checkout_ids: [] }, agent_scope: { ...emptyScope(), members } } }];
  const projects = tool ? toolProjects(device, tool) : device;
  const rest = { navigator: { devices: [{ id: "local", kind: "local", agent_scope: emptyScope() }], workspaces: projects.map((project) => project.workspace), focused_device_id: "local", tools: tool ? [tool] : [] }, status: { remote: [] }, missions: { items: [], failed: false }, overview_graph: { available: true, limit: 2000, retained_count: 0, truncated: false, retention_ms: 86400000, letters: [], sessions: agents.map((agent) => ({ device_id: "local", pane_id: agent.pane_id, session_digest: `digest-${agent.pane_id}` })) } } as unknown as SnapshotRest;
  useShellStore.setState({ rest, agents, focusedPaneId: null, connection: "live", linkSummaries: null });
  useUiStore.getState().setOverview({ projectId: null, panel: null });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><OverviewGraph projects={projects} deviceId="local" tool={tool} actions={createActions(() => true)} /></TooltipProvider>));
  const seen = {
    asks: [...container.querySelectorAll<HTMLElement>("[data-graph-ask]")].map((ask) => ({ pane: ask.dataset.graphAsk, line: ask.querySelector("p")?.textContent ?? null })),
    filter: [...container.querySelectorAll<HTMLElement>("[data-focus]")].map((chip) => chip.textContent?.replace(/\s+/g, " ").trim()),
    heading: container.querySelector(".og-waiting h3")?.firstChild?.textContent?.trim() ?? null,
  };
  await act(async () => root.unmount());
  return seen;
}

/** A session node of the map, drawn alone. */
async function node(agent: AgentRow, inCell = false) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  initializeInterfaceI18n("en");
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const row = { agent, id: "session:h", projectId: "project:alpha", deviceId: "local", digest: null, launchModel: null, launchEffort: null, activity: "turn", device: null, project: { label: "alpha" }, checkout: { label: "main" } } as unknown as GraphAgent;
  const props = { id: "session:h", data: { row, root: true, branch: null, choose: () => {}, highlight: () => {}, team: null, current: false, context: false, muted: false, highlighted: false }, selected: false } as unknown as Parameters<typeof SessionGraphNode>[0];
  await act(async () => root.render(<TooltipProvider><InToolCell.Provider value={inCell}><ReactFlowProvider><SessionGraphNode {...props} /></ReactFlowProvider></InToolCell.Provider></TooltipProvider>));
  const line = container.querySelectorAll<HTMLElement>(".og-session-line");
  const seen = line.length === 1 ? { text: line[0]!.textContent, asking: line[0]!.classList.contains("og-asking") } : { lines: line.length };
  await act(async () => root.unmount());
  return seen;
}

it("leads the operator's Waiting on you card and the node's line with the tool's name, as the sidebar row does", async () => {
  const { asks } = await map([WAITING_ON_PERSON], null);
  expect(asks).toHaveLength(1);
  expect(asks[0]!.line).toMatch(/^nightly·/);
  expect(await node(WAITING_ON_PERSON)).toEqual({ text: "nightly·Sign-in · github.com/login", asking: true });
  // The operator's own row names no tool, and in the tool's own cell the line stands alone, as the same one line
  // with its chip and its asking colour (review of pull request 2586: the cell's lines lost both).
  expect(await node({ ...WAITING_ON_PERSON, tool: undefined })).toEqual({ text: "Sign-in · github.com/login", asking: true });
  expect(await node(WAITING_ON_PERSON, true)).toEqual({ text: "Sign-in · github.com/login", asking: true });
});

it("words and counts a tool's map filter as the tool's cell does: its turn is what waits on the tool", async () => {
  const cell: ToolCell = { ...TOOL, mission_asks: [], devices: [{ device_id: "local", members: [], tasks: { needs_you: 1, recency: "", cards: [
    { root: "q", state: "needs_you", members: ["q"], checkouts: ["alpha-main"], recency: "" },
    { root: "h", state: "waiting", members: ["h"], checkouts: ["alpha-main"], recency: "" },
  ] } }] };
  cell.devices[0]!.members = ["q", "h"].map((pane_id) => ({ pane_id, project_id: "alpha", checkout_id: "alpha-main" }));
  const seen = await map([ASKING_TOOL, WAITING_ON_PERSON], cell);
  expect(seen.filter).toEqual(["All 2", "Bot's turn 1", "Working 1", "Unread results 0"]);
  expect(seen.heading).toBe("Bot's turn");
  expect(seen.asks.map((ask) => ask.pane)).toEqual(["q"]);
});

// Issue 2597, PRD hide-external-control D-46: nothing in a tool's cell is the operator's turn, so its map draws no
// warning colour: the filter chip's turn count, the project rows' counts, the edges and the asks all use it through
// one variable, which the tool's map sets to the grey of the cell's count; a device's map keeps its own.
it("draws a tool's map with no warning colour and leaves a device's map as it was", async () => {
  const css = readFileSync(`${process.cwd()}/src/overviewGraph.css`, "utf8");
  const rule = css.split("\n").find((line) => line.startsWith(".overview-graph[data-graph-tool]")) ?? "";
  expect(rule, "the tool's map redefines the warning colour").toContain("--warning: var(--muted-foreground)");
  expect(rule, "and the alias Tailwind's text-warning reads").toContain("--color-warning: var(--muted-foreground)");
  expect(css.split("\n").filter((line) => /--warning\s*:/.test(line)).map((line) => line.split("{")[0]!.trim()), "no other rule recolours it").toEqual([".overview-graph[data-graph-tool]"]);
  const hosts = async (tool: ToolCell | null) => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
    initializeInterfaceI18n("en");
    useShellStore.setState({ rest: { navigator: { devices: [{ id: "local", kind: "local", agent_scope: emptyScope() }], workspaces: [], focused_device_id: "local", tools: tool ? [tool] : [] }, status: { remote: [] }, missions: { items: [], failed: false } } as unknown as SnapshotRest, agents: [], focusedPaneId: null, connection: "live", linkSummaries: null });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    await act(async () => root.render(<TooltipProvider><OverviewGraph projects={[]} deviceId="local" tool={tool} actions={createActions(() => true)} /></TooltipProvider>));
    const found = { marked: container.querySelector("[data-overview-graph]")?.hasAttribute("data-graph-tool"), warningClass: /text-warning|bg-warning|border-warning/.test(container.innerHTML) };
    await act(async () => root.unmount());
    return found;
  };
  expect(await hosts(TOOL)).toEqual({ marked: true, warningClass: false });
  expect(await hosts(null)).toMatchObject({ marked: false });
});
