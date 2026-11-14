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
import { missionFixture, withMissionAsk } from "./gallery/MissionScene";
import { OverviewGraph } from "./OverviewGraph";
import { AddProjectGraphNode, ClusterGraphNode, OperatorGraphNode, ProjectGraphNode, StartSlotGraphNode, type AddProjectNode, type ClusterNode, type OperatorNode, type ProjectNode, type StartSlotNode } from "./overviewGraphNodes";
import { ReactFlowProvider, type NodeProps } from "@xyflow/react";
import type { BoardProject } from "./projectBoard";
import { GRAPH_CHOICES_DEFAULTS } from "./overviewGraphModel";
import type { Checkout, OverviewGraphChoices, SnapshotRest, Workspace } from "./snapshot";
import { useShellStore } from "./store";
import { useReplyFields } from "./agentReply";
import { useUiStore } from "./ui";
import { useStartPanel } from "./startDraft";
import type { DispatchFn } from "./ws";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
// Layout failure is a supported surface with the same session preview actions.
vi.mock("./overviewGraphLayout", () => ({
  useOverviewGraphLayout: () => ({ phase: "failed", size: null, points: new Map(), map: null, reset: () => {} }),
  useGraphTheme: () => "light",
}));
// The map's node budget is 400; what these tests claim holds for a project past any budget, and filling
// 400 nodes made the overflow case take up to 5.8 s on the Windows runner (hide-private issue 145).
// Every project takes its name, its start circle and one access slot after the map's two-node Add project
// place, so (BUDGET - 2) / 3 projects fill the map and one more is left out.
const { BUDGET } = vi.hoisted(() => ({ BUDGET: 14 }));
vi.mock("./dependencyLayout", async (importOriginal) => ({ ...await importOriginal<typeof import("./dependencyLayout")>(), LAYOUT_NODE_LIMIT: BUDGET }));
const OUTSIDE = (BUDGET - 2) / 3;
const OUTSIDE_PROJECT = `project-${String(OUTSIDE).padStart(3, "0")}`;
const savedShell = useShellStore.getState();
const savedUi = useUiStore.getState();
afterEach(() => {
  useShellStore.setState(savedShell, true);
  useUiStore.setState(savedUi, true);
  useReplyFields.setState({ fields: {} });
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
  const projects = overflow ? Array.from({ length: OUTSIDE + 1 }, (_, index): BoardProject => {
    const id = `project-${String(index).padStart(3, "0")}`;
    const paneId = `pane-${index}`;
    const row = { ...agent, id: paneId, pane_id: paneId, identity_label: `Session ${index}`, session_id: `native-${index}` };
    return { ...project, agents: [row], workspace: { ...workspace, id, label: id, checkouts: [{ ...checkout, workspace_id: id }], agent_scope: { ...emptyScope(), members: [{ pane_id: paneId, project_id: id, checkout_id: checkout.id }] } } };
  }) : [project];
  const rest = { navigator: { devices: [{ id: "local", kind: "local" }, { id: "mini", kind: "remote" }], workspaces: [workspace], focused_device_id: front }, status: { remote: [{ target_id: "mini", state: "connected", session: { workspaces: [], agents: device === "mini" ? [agent] : [] } }] }, overview_graph: { available: true, limit: 2000, retained_count: 0, truncated: false, retention_ms: 86400000, letters: [], sessions: [{ device_id: device, pane_id: pane, session_digest: "confirmed-current", launch_model: null, launch_effort: null }] } } as unknown as SnapshotRest;
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
    // The filter's counts say they are the map's sessions.
    expect(container.querySelector(".og-filter-scope")?.textContent).toBe("Sessions on the map");
    // B4/D09: a matching confirmed entry highlights without opening a panel.
    expect(container.querySelector("[data-graph-preview]")).toBeNull();
    expect(container.querySelector("[data-graph-session-list] > button")?.getAttribute("aria-pressed")).toBe(String(!overflow && front === device));
    if (overflow) {
      expect(container.querySelector(".og-notice")?.textContent).toContain(`Up to ${BUDGET} visible nodes`);
      const search = container.querySelector<HTMLInputElement>("[data-graph-session-list] input")!;
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(search, `Session ${OUTSIDE}`);
        search.dispatchEvent(new Event("input", { bubbles: true }));
      });
      await act(async () => container.querySelector<HTMLButtonElement>("[data-graph-session-list] > button")!.click());
      expect(useUiStore.getState().overview.projectId).toBe(OUTSIDE_PROJECT);
      expect(container.querySelector("[data-graph-preview]")?.textContent).toContain(`Session ${OUTSIDE}`);
      expect(container.querySelector("[data-graph-session-list] > button")?.getAttribute("aria-pressed")).toBe("true");
      expect(events).toHaveLength(0);
      await act(async () => useUiStore.getState().setOverview({ projectId: null }));
      await act(async () => useUiStore.getState().setOverview({ projectId: OUTSIDE_PROJECT }));
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

// PRD mission-node-simple B14, review 5 P2-2: a mission no row carries is
// listed on the Overview with its title, reason and choices even when the
// map draws no session at all.
it("lists a mission waiting on the operator when no session is drawn", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  initializeInterfaceI18n("en");
  const base = { navigator: { devices: [{ id: "local", kind: "local", agent_scope: emptyScope() }], workspaces: [], focused_device_id: "local" }, status: { remote: [] }, overview_graph: { available: true, limit: 2000, retained_count: 0, truncated: false, retention_ms: 86400000, letters: [], sessions: [] } } as unknown as SnapshotRest;
  const rest = withMissionAsk(base, "local", Date.UTC(2026, 10, 1));
  useShellStore.setState({ rest, agents: [], focusedPaneId: null, connection: "live", linkSummaries: null });
  useUiStore.getState().setOverview({ projectId: null, panel: null });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => root.render(<TooltipProvider><OverviewGraph projects={[]} deviceId="local" actions={actions} /></TooltipProvider>));
    // With no session awake the map's place is New agent's, and the waiting mission stays under it.
    expect(container.querySelector("[data-graph-welcome]")).not.toBeNull();
    const item = container.querySelector<HTMLElement>('[data-graph-mission-asks] [data-graph-mission-ask="mission-1"]');
    expect(item?.textContent).toContain("이슈 정리");
    // No Conductor holds it, so its raise stands behind the safety net, which the item reads.
    expect(item?.textContent).toContain("No Conductor conducts this mission · 1 stuck");
    expect([...item!.querySelectorAll("[data-mission-ask-choices] button")].map((button) => button.textContent)).toEqual(["Resume command", "Take a look"]);
    expect(container.querySelector("[data-graph-mission-asks] h3")?.textContent).toBe("Waiting on you 1");
  } finally { await act(async () => root.unmount()); }
});

// Review 6 P2-1 of PRD mission-node-simple: a map narrowed to one project
// lists and counts that project's missions only, on an empty map and beside
// drawn sessions; the whole map keeps every one the device's scope names.
it("lists only the chosen project's missions waiting on the operator, on an empty and a drawn map", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  initializeInterfaceI18n("en");
  const { mission } = missionFixture("no-conductor", Date.UTC(2026, 10, 1));
  const missions = [{ ...mission, id: "mission-a", title: "알파 미션", workspace_id: "alpha" }, { ...mission, id: "mission-b", title: "베타 미션", workspace_id: "beta" }];
  const board = (id: string, label: string, agents: ReturnType<typeof legacyAgentRow>[]): BoardProject => {
    const row: Checkout = { id: `${id}-main`, workspace_id: id, label: "main", path: `/fixture/${id}`, branch: "main", purpose: null, is_worktree: false, exists: true, has_panes: agents.length > 0, pull_request: null, tabs: [], active_tab_id: null, next_tab_label: "Tab 1", strip: [], agent_scope: emptyScope() };
    return { agents, device: null, workspace: { id, label, device_id: "local", path: `/fixture/${id}`, registered: true, temporary: false, pinned: false, checkouts: [row], inactive_checkouts: { expanded: false, checkout_ids: [] }, agent_scope: { ...emptyScope(), members: agents.map((agent) => ({ pane_id: agent.pane_id, project_id: id, checkout_id: row.id })) } } };
  };
  const shown = async (drawn: boolean, projectId: string | null) => {
    const agents = drawn ? [legacyAgentRow({ id: "w", pane_id: "w", identity_label: "작업", agent_kind: "claude", session_id: "native-w", group: "working", activity: "working" }), legacyAgentRow({ id: "v", pane_id: "v", identity_label: "다른 작업", agent_kind: "claude", session_id: "native-v", group: "working", activity: "working" })] : [];
    const projects = [board("alpha", "알파", agents.slice(0, 1)), board("beta", "베타", agents.slice(1))];
    const scope = { ...emptyScope(), mission_asks: ["mission-a", "mission-b"] };
    const rest = { navigator: { devices: [{ id: "local", kind: "local", agent_scope: scope }], workspaces: projects.map((project) => project.workspace), focused_device_id: "local" }, status: { remote: [] }, missions: { items: missions, failed: false }, overview_graph: { available: true, limit: 2000, retained_count: 0, truncated: false, retention_ms: 86400000, letters: [], sessions: agents.map((agent) => ({ device_id: "local", pane_id: agent.pane_id, session_digest: `digest-${agent.pane_id}` })) } } as unknown as SnapshotRest;
    useShellStore.setState({ rest, agents, focusedPaneId: null, connection: "live", linkSummaries: null });
    useUiStore.getState().setOverview({ projectId, panel: null });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    await act(async () => root.render(<TooltipProvider><OverviewGraph projects={projects} deviceId="local" actions={createActions(() => true)} /></TooltipProvider>));
    const listed = [...container.querySelectorAll<HTMLElement>("[data-graph-mission-ask]")].map((item) => item.dataset.graphMissionAsk);
    const count = container.querySelector(drawn ? '[data-graph-projects] .og-waiting h3 span' : "[data-graph-mission-asks] h3 span")?.textContent ?? null;
    await act(async () => root.unmount());
    container.remove();
    return { listed, count };
  };
  for (const drawn of [false, true]) {
    expect(await shown(drawn, null), `whole map, drawn ${drawn}`).toEqual({ listed: ["mission-a", "mission-b"], count: "2" });
    expect(await shown(drawn, "alpha"), `project A, drawn ${drawn}`).toEqual({ listed: ["mission-a"], count: "1" });
    expect(await shown(drawn, "beta"), `project B, drawn ${drawn}`).toEqual({ listed: ["mission-b"], count: "1" });
  }
});

// PRD overview-main-map B11-B16: the side column lists projects with their
// states and what waits on the operator; a chosen session shows its team and
// its counterparts by count only; hiding a project ends a selection inside it.
it("lists projects, hides one, and shows a chosen session's team and counterparts without message text", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  initializeInterfaceI18n("en");
  const checkout = (workspace: string): Checkout => ({ id: `${workspace}-main`, workspace_id: workspace, label: "main", path: `/fixture/${workspace}`, branch: "feat/map", purpose: null, is_worktree: false, exists: true, has_panes: true, pull_request: null, tabs: [], active_tab_id: null, next_tab_label: "Tab 1", strip: [], agent_scope: emptyScope() });
  const root = legacyAgentRow({ id: "r", pane_id: "r", identity_label: "배포 확인", agent_kind: "claude", session_id: "native-r", group: "needs_you", demand: "question", mark: "turn" });
  root.state.ask = { verb: "answer", what: "배포 범위를 확인해 주세요", more: 0 };
  const child = legacyAgentRow({ id: "c", pane_id: "c", identity_label: "검증 자식", agent_kind: "codex", session_id: "native-c", group: "working", activity: "working", delegated: true, lineage_parent_pane_id: "r" });
  const other = legacyAgentRow({ id: "o", pane_id: "o", identity_label: "다른 일", agent_kind: "claude", session_id: "native-o", group: "working", activity: "working", profile: "work" });
  const board = (id: string, label: string, agents: typeof root[]): BoardProject => {
    const row = checkout(id);
    return { agents, device: null, workspace: { id, label, device_id: "local", path: `/fixture/${id}`, registered: true, temporary: false, pinned: false, checkouts: [row], inactive_checkouts: { expanded: false, checkout_ids: [] }, agent_scope: { ...emptyScope(), members: agents.map((agent) => ({ pane_id: agent.pane_id, project_id: id, checkout_id: row.id })) } } };
  };
  const projects = [board("alpha", "알파", [root, child]), board("beta", "베타", [other])];
  const party = (pane: string) => ({ device_id: "local", pane_id: pane, session: `digest-${pane}` });
  const rest = { navigator: { devices: [{ id: "local", kind: "local" }], workspaces: projects.map((project) => project.workspace), focused_device_id: "local" }, status: {}, overview_graph: {
    available: true, limit: 2000, retained_count: 4, truncated: false, retention_ms: 86400000,
    letters: [{ id: "l1", sender: party("r"), recipient: party("o"), kind: "request", at_unix_ms: 10 }, { id: "l2", sender: party("o"), recipient: party("r"), kind: "report", at_unix_ms: 20 }, { id: "l3", sender: party("o"), recipient: party("r"), kind: "progress", at_unix_ms: 30 }, { id: "l4", sender: party("c"), recipient: party("r"), kind: "progress", at_unix_ms: 40 }],
    sessions: ["r", "c", "o"].map((pane) => ({ device_id: "local", pane_id: pane, session_digest: `digest-${pane}`, launch_model: pane === "r" ? "opus" : null, launch_effort: pane === "r" ? "high" : null })),
  } } as unknown as SnapshotRest;
  useShellStore.setState({ rest, agents: [root, child, other], focusedPaneId: null, connection: "live", linkSummaries: null });
  useUiStore.getState().setOverview({ projectId: null, panel: null });
  useUiStore.setState({ overviewOpen: false, opening: null, screen: { kind: "main", deviceId: "local" } });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const reactRoot = createRoot(container);
  const button = (name: string, within: ParentNode = container) => [...within.querySelectorAll<HTMLButtonElement>("button")].find((one) => one.textContent?.trim() === name || one.getAttribute("aria-label") === name);
  try {
    await act(async () => reactRoot.render(<TooltipProvider><OverviewGraph projects={projects} deviceId="local" actions={actions} /></TooltipProvider>));
    const column = () => container.querySelector("[data-graph-projects]")!;
    // B11: each project with its session count and nonzero states.
    const alpha = column().querySelector('[data-graph-project-row="alpha"]')!;
    expect(alpha.textContent).toContain("알파");
    expect(alpha.querySelector('[data-state="turn"]')?.textContent).toContain("1");
    expect(alpha.querySelector('[data-state="going"]')?.textContent).toContain("1");
    expect(alpha.querySelector(".og-count")?.textContent).toBe("2");
    expect(column().querySelector('[data-graph-project-row="beta"] [data-state="turn"]')).toBeNull();
    // B12: the root whose turn it is, its question, and a way to answer in its pane.
    const ask = column().querySelector('[data-graph-ask="r"]')!;
    expect(ask.textContent).toContain("배포 범위를 확인해 주세요");
    await act(async () => button("Go answer", ask)!.click());
    expect(events.at(-1)).toMatchObject({ kind: "focus_pane", payload: { pane_id: "r" } });
    // B15/B16: choosing the root shows its team and counterparts as counts.
    await act(async () => ask.querySelector<HTMLButtonElement>(".og-ask-who")!.click());
    const preview = () => container.querySelector("[data-graph-preview]");
    expect(preview()?.textContent).toContain("Asks you");
    expect(preview()?.textContent).toContain("feat/map");
    expect(preview()?.textContent).toContain("Launched with · opus · high");
    const team = preview()!.querySelector(".og-team-list")!;
    expect(team.textContent).toContain("검증 자식");
    const talks = [...preview()!.querySelectorAll(".og-talks > div")].map((row) => row.textContent);
    expect(talks).toEqual(expect.arrayContaining([expect.stringContaining("Sent 1 · Received 2"), expect.stringContaining("Received 1")]));
    // PRD agent-profiles B17: a counterpart whose logo wears a profile badge says its agent and profile on hover and to assistive technology; a default row keeps its hover.
    const counterpart = [...preview()!.querySelectorAll<HTMLElement>(".og-talks > div")].find((row) => row.textContent?.includes("다른 일"))!;
    expect(counterpart.title).toBe("Claude Code · work\nLetters 3");
    expect(counterpart.querySelector(".sr-only")?.textContent).toBe(", Claude Code · work");
    expect(ask.querySelector(".og-ask-who")!.hasAttribute("title")).toBe(false);
    expect(preview()!.textContent).not.toContain("request");
    // A team row chooses that descendant.
    await act(async () => team.querySelector<HTMLButtonElement>("button")!.click());
    expect(preview()?.querySelector(".og-details-head")?.textContent).toContain("검증 자식");
    // Escape closes the panel first (the shell's keyboard pops the top layer); the session stays highlighted.
    await act(async () => useUiStore.getState().escapeLayers.at(-1)!());
    expect(preview()).toBeNull();
    const listed = (name: string) => [...container.querySelectorAll<HTMLButtonElement>("[data-graph-session-list] > button")].find((row) => row.textContent?.includes(name))!;
    expect(listed("검증 자식").getAttribute("aria-pressed")).toBe("true");
    expect(listed("다른 일").title).toBe("Claude Code · work");
    expect(listed("검증 자식").hasAttribute("title")).toBe(false);
    // PRD overview-graph-remembered B7, B8: the row holds its name, states and count, with no Fold or Hide words; an eye hides it.
    expect(button("Fold 알파 into one circle")).toBeUndefined();
    expect([...alpha.querySelectorAll("button")].map((one) => one.textContent?.trim())).not.toContain("Hide");
    const eye = () => column().querySelector<HTMLButtonElement>('[data-graph-project-row="alpha"] .og-eye')!;
    expect(eye().getAttribute("aria-label")).toBe("Hide 알파 from the graph");
    expect(eye().getAttribute("aria-pressed")).toBe("false");
    await act(async () => eye().click());
    expect(events.at(-1)).toEqual({ schema_version: 2, kind: "overview_graph_change", payload: { change: { kind: "project_hidden", project: { device_id: "local", workspace_id: "alpha" }, hidden: true } } });
    // The core's answer is what the map draws (D-05).
    const answer = (overview_graph: Partial<OverviewGraphChoices>) => act(async () => useShellStore.setState({ rest: { ...rest, ui_state: { overview_graph: { ...GRAPH_CHOICES_DEFAULTS, ...overview_graph } } } as unknown as SnapshotRest }));
    await answer({ hidden_projects: [{ device_id: "local", workspace_id: "alpha" }] });
    // B14: hiding the highlighted session's project clears it; the row stays, dimmed, with its counts.
    expect(listed("검증 자식").getAttribute("aria-pressed")).toBe("false");
    const hidden = column().querySelector('[data-graph-project-row="alpha"]')!;
    expect(hidden.getAttribute("data-hidden")).toBe("true");
    expect(hidden.querySelector('[data-state="turn"]')?.textContent).toContain("1");
    // B9: the hidden row's eye says how to bring it back, and its name does not move the camera.
    expect(eye().getAttribute("aria-label")).toBe("Show 알파 on the graph again");
    expect(eye().getAttribute("aria-pressed")).toBe("true");
    expect(button("Locate 알파")!.disabled).toBe(true);
    // D-21/B27: a hidden project's question still waits for the operator, marked hidden.
    expect(column().querySelector('[data-graph-ask="r"] .og-hidden-mark')?.textContent).toBe("Hidden on the graph");
    // B1: closing and opening Overview again keeps it hidden, because the core holds it.
    await act(async () => reactRoot.render(<TooltipProvider><div /></TooltipProvider>));
    await act(async () => reactRoot.render(<TooltipProvider><OverviewGraph projects={projects} deviceId="local" actions={actions} /></TooltipProvider>));
    expect(column().querySelector('[data-graph-project-row="alpha"]')?.getAttribute("data-hidden")).toBe("true");
    await act(async () => eye().click());
    expect(events.at(-1)).toMatchObject({ kind: "overview_graph_change", payload: { change: { kind: "project_hidden", hidden: false } } });
    // Choosing a hidden project's waiting root shows the project again and keeps the choice open across the core's answer.
    const sent = events.length;
    await act(async () => column().querySelector<HTMLButtonElement>('[data-graph-ask="r"] .og-ask-who')!.click());
    expect(events.slice(sent).filter((event) => event.kind === "overview_graph_change")).toEqual([{ schema_version: 2, kind: "overview_graph_change", payload: { change: { kind: "project_hidden", project: { device_id: "local", workspace_id: "alpha" }, hidden: false } } }]);
    expect(preview()?.querySelector(".og-details-head")?.textContent).toContain("배포 확인");
    await answer({});
    expect(preview()?.querySelector(".og-details-head")?.textContent).toContain("배포 확인");
    await act(async () => useUiStore.getState().escapeLayers.at(-1)!());
    // B6: a filter is one event, and the one the core holds is the one shown after reopening.
    await act(async () => container.querySelector<HTMLButtonElement>('[data-focus="turn"]')!.click());
    expect(events.at(-1)).toMatchObject({ payload: { change: { kind: "focus", focus: "turn" } } });
    expect(events.at(-1)?.payload).not.toHaveProperty("change.include_resting");
    await answer({ focus: "turn" });
    expect(container.querySelector('[data-focus="turn"]')?.getAttribute("aria-pressed")).toBe("true");
  } finally { await act(async () => reactRoot.unmount()); }
});

// Issue 2513: the waiting item and the session detail each answer the agent
// without opening its session, as one `pane_reply`; 답하러 가기 and 세션 열기
// stay, and a refused or unsent reply keeps its words with the reason.
it("answers a waiting agent from its item and from the session detail, keeping refused words with their reason", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  initializeInterfaceI18n("en");
  const checkout: Checkout = { id: "alpha-main", workspace_id: "alpha", label: "main", path: "/fixture/alpha", branch: "main", purpose: null, is_worktree: false, exists: true, has_panes: true, pull_request: null, tabs: [], active_tab_id: null, next_tab_label: "Tab 1", strip: [], agent_scope: emptyScope() };
  const asking = legacyAgentRow({ id: "r", pane_id: "r", identity_label: "배포 확인", agent_kind: "claude", session_id: "native-r", group: "needs_you", demand: "question", mark: "turn" });
  asking.state.ask = { verb: "answer", what: "배포 범위를 확인해 주세요", more: 0 };
  const projects: BoardProject[] = [{ agents: [asking], device: null, workspace: { id: "alpha", label: "알파", device_id: "local", path: "/fixture/alpha", registered: true, temporary: false, pinned: false, checkouts: [checkout], inactive_checkouts: { expanded: false, checkout_ids: [] }, agent_scope: { ...emptyScope(), members: [{ pane_id: "r", project_id: "alpha", checkout_id: checkout.id }] } } }];
  const rest = { navigator: { devices: [{ id: "local", kind: "local" }], workspaces: projects.map((project) => project.workspace), focused_device_id: "local" }, status: {}, overview_graph: { available: true, limit: 2000, retained_count: 0, truncated: false, retention_ms: 86400000, letters: [], sessions: [{ device_id: "local", pane_id: "r", session_digest: "digest-r", launch_model: null, launch_effort: null }] } } as unknown as SnapshotRest;
  useShellStore.setState({ rest, agents: [asking], focusedPaneId: null, connection: "live", linkSummaries: null, paneReplies: null });
  useUiStore.getState().setOverview({ projectId: null, panel: null });
  useUiStore.setState({ overviewOpen: false, opening: null, screen: { kind: "main", deviceId: "local" } });
  const events: Parameters<DispatchFn>[0][] = [];
  let online = true;
  const actions = createActions((event) => { if (!online) return false; events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const reactRoot = createRoot(container);
  const type = (input: HTMLInputElement, value: string) => act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  const enter = (input: HTMLInputElement) => act(async () => { input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
  const answer = (state: "sending" | "sent" | "refused", reason?: string) => act(async () => {
    const sent = events.at(-1)!.payload as { request_id: string };
    useShellStore.setState({ paneReplies: { receipts: [{ request_id: sent.request_id, pane_id: "r", state, ...(reason ? { reason } : {}) }] } });
  });
  try {
    await act(async () => reactRoot.render(<TooltipProvider><OverviewGraph projects={projects} deviceId="local" actions={actions} /></TooltipProvider>));
    const item = container.querySelector('[data-graph-ask="r"]')!;
    const field = () => container.querySelector<HTMLElement>('[data-agent-reply="r"]')!;
    const input = () => field().querySelector<HTMLInputElement>("input")!;
    expect([...item.querySelectorAll("button")].some((button) => button.textContent === "Go answer")).toBe(true);
    expect(input().getAttribute("aria-label")).toBe("Reply to 배포 확인");

    // Enter sends one event with the words; nothing opens.
    await type(input(), "프로덕션까지 해 주세요");
    await enter(input());
    expect(events).toHaveLength(1);
    // It names the session the map drew there, which the core checks the pane still runs.
    expect(events[0]).toMatchObject({ schema_version: 2, kind: "pane_reply", payload: { pane_id: "r", text: "프로덕션까지 해 주세요", session: "digest-r" } });
    expect((events[0]!.payload as { request_id: string }).request_id).toMatch(/^[0-9a-f-]{36}$/);
    expect(useUiStore.getState().opening).toBeNull();
    await answer("sending");
    expect(field().getAttribute("data-agent-reply-state")).toBe("sending");
    await enter(input());
    expect(events).toHaveLength(1);

    // A refusal keeps the words and says why, and the same field sends again.
    await answer("refused", "asleep");
    expect(input().value).toBe("프로덕션까지 해 주세요");
    expect(field().querySelector('[role="alert"]')?.textContent).toBe("The session is asleep: wake it to send");
    expect(input().getAttribute("aria-invalid")).toBe("true");
    // Editing the words clears the reason.
    await type(input(), "프로덕션까지 해 주세요!");
    expect(field().querySelector('[role="alert"]')).toBeNull();
    await type(input(), "프로덕션까지 해 주세요");

    // A reply that never left the page is refused as offline, its words kept.
    online = false;
    await enter(input());
    expect(events).toHaveLength(1);
    expect(field().querySelector('[role="alert"]')?.textContent).toBe("Hide is not connected: nothing was sent");
    expect(input().value).toBe("프로덕션까지 해 주세요");
    online = true;

    // The session detail shares the field and keeps 세션 열기; its send empties the field once the pane took it.
    await act(async () => item.querySelector<HTMLButtonElement>(".og-ask-who")!.click());
    const preview = container.querySelector("[data-graph-preview]")!;
    expect([...preview.querySelectorAll("button")].some((button) => button.textContent === "Open session")).toBe(true);
    const detail = preview.querySelector<HTMLElement>('[data-agent-reply="r"]')!;
    expect(detail.querySelector("input")!.value).toBe("프로덕션까지 해 주세요");
    await act(async () => detail.querySelector<HTMLButtonElement>('button[aria-label="Send"]')!.click());
    expect(events).toHaveLength(2);
    expect(events[1]).toMatchObject({ kind: "pane_reply", payload: { pane_id: "r", text: "프로덕션까지 해 주세요" } });
    await answer("sent");
    expect(detail.querySelector("input")!.value).toBe("");
    expect(detail.querySelector('[role="alert"]')).toBeNull();
    expect(detail.querySelector<HTMLButtonElement>('button[aria-label="Send"]')!.disabled).toBe(true);
  } finally { await act(async () => reactRoot.unmount()); }
});

// PRD pane-open-in-overview B4-B7, B10, B12: a pane's Show in Overview opens Work on the
// pane's device with its session chosen, opening what hid it; any other opening only highlights.
it("opens Work with a pane's session chosen, showing its hidden project and folded team, and only once", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  initializeInterfaceI18n("en");
  const checkout = (workspace: string): Checkout => ({ id: `${workspace}-main`, workspace_id: workspace, label: "main", path: `/fixture/${workspace}`, branch: "feat/map", purpose: null, is_worktree: false, exists: true, has_panes: true, pull_request: null, tabs: [], active_tab_id: null, next_tab_label: "Tab 1", strip: [], agent_scope: emptyScope() });
  const root = legacyAgentRow({ id: "r", pane_id: "r", identity_label: "배포 확인", agent_kind: "claude", session_id: "native-r", group: "working", activity: "working" });
  const child = legacyAgentRow({ id: "c", pane_id: "c", identity_label: "검증 자식", agent_kind: "codex", session_id: "native-c", group: "working", activity: "working", delegated: true, lineage_parent_pane_id: "r" });
  const fresh = legacyAgentRow({ id: "n", pane_id: "n", identity_label: "새 세션", agent_kind: "claude", session_id: "native-n", group: "working", activity: "working" });
  const row = checkout("alpha");
  const projects: BoardProject[] = [{ agents: [root, child], device: null, workspace: { id: "alpha", label: "알파", device_id: "local", path: "/fixture/alpha", registered: true, temporary: false, pinned: false, checkouts: [row], inactive_checkouts: { expanded: false, checkout_ids: [] }, agent_scope: { ...emptyScope(), members: ["r", "c"].map((pane_id) => ({ pane_id, project_id: "alpha", checkout_id: row.id })) } } }];
  const alpha = { device_id: "local", workspace_id: "alpha" };
  const rest = { navigator: { devices: [{ id: "local", kind: "local" }], workspaces: projects.map((project) => project.workspace), focused_device_id: "local" }, status: {},
    overview_graph: { available: true, limit: 2000, retained_count: 0, truncated: false, retention_ms: 86400000, letters: [], sessions: ["r", "c"].map((pane) => ({ device_id: "local", pane_id: pane, session_digest: `digest-${pane}`, launch_model: null, launch_effort: null })) },
    ui_state: { overview_graph: { ...GRAPH_CHOICES_DEFAULTS, hidden_projects: [alpha], folded_projects: [alpha], team_folds: [{ session: { device_id: "local", pane_id: "r", session: "digest-r" }, folded: true }] } },
  } as unknown as SnapshotRest;
  // `n` runs in a pane the map has no project for yet.
  useShellStore.setState({ rest, agents: [root, child, fresh], focusedPaneId: null, connection: "live", linkSummaries: null, diagnostics: [] });
  useUiStore.setState({ overviewOpen: false, opening: null, screen: { kind: "main", deviceId: "local" }, mainView: "projects" });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const reactRoot = createRoot(container);
  const preview = () => container.querySelector("[data-graph-preview]");
  const graph = () => <TooltipProvider><OverviewGraph projects={projects} deviceId={useUiStore.getState().overview.deviceId!} actions={actions} /></TooltipProvider>;
  try {
    actions.openPaneInOverview("c");
    // B4: Work, whichever tab was last, on the pane's device.
    expect(useUiStore.getState().mainView).toBe("work");
    expect(useUiStore.getState().overview.deviceId).toBe("local");
    await act(async () => reactRoot.render(graph()));
    // B5: the session is chosen and its details are open.
    expect(preview()?.querySelector(".og-details-head")?.textContent).toContain("검증 자식");
    // B7: the hidden project shows again, it unfolds, and so does the team folded under its root.
    expect(events.filter((event) => event.kind === "overview_graph_change").map((event) => (event.payload as { change: unknown }).change)).toEqual([
      { kind: "project_hidden", project: alpha, hidden: false },
      { kind: "project_folded", project: alpha, folded: false },
      { kind: "team_folded", session: { device_id: "local", pane_id: "r", session: "digest-r" }, folded: false },
    ]);
    // The core's answer does not end the choice.
    await act(async () => useShellStore.setState({ rest: { ...rest, ui_state: { overview_graph: GRAPH_CHOICES_DEFAULTS } } as unknown as SnapshotRest }));
    expect(preview()?.querySelector(".og-details-head")?.textContent).toContain("검증 자식");
    // B6: Escape closes the details, back to the projects and what waits.
    await act(async () => useUiStore.getState().escapeLayers.at(-1)!());
    expect(preview()).toBeNull();
    expect(container.querySelector("[data-graph-projects]")).not.toBeNull();
    // B12: another opening of the Overview chooses nothing, even right after the button.
    await act(async () => reactRoot.render(<TooltipProvider><div /></TooltipProvider>));
    actions.openPaneInOverview("c");
    actions.openOverview("local");
    await act(async () => reactRoot.render(graph()));
    expect(preview()).toBeNull();
    // B10: a session the map does not know opens Work with nothing chosen, said only in the log.
    await act(async () => reactRoot.render(<TooltipProvider><div /></TooltipProvider>));
    actions.openPaneInOverview("n");
    await act(async () => reactRoot.render(graph()));
    expect(preview()).toBeNull();
    expect(useUiStore.getState().overviewReveal).toBeNull();
    expect(useShellStore.getState().diagnostics.at(-1)).toContain("pane n");
  } finally { await act(async () => reactRoot.unmount()); }
});

// PRD pane-open-in-overview B7-B9 for a sleeping pane: the map it opens draws the chosen session though resting
// sessions are left out, so its quiet team unfolds and a project of sleepers alone is no budget overflow.
it("opens a sleeping pane's session drawn on the whole map, its quiet team unfolded", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  initializeInterfaceI18n("en");
  const board = (id: string, agents: ReturnType<typeof legacyAgentRow>[]): BoardProject => {
    const row: Checkout = { id: `${id}-main`, workspace_id: id, label: "main", path: `/fixture/${id}`, branch: "main", purpose: null, is_worktree: false, exists: true, has_panes: true, pull_request: null, tabs: [], active_tab_id: null, next_tab_label: "Tab 1", strip: [], agent_scope: emptyScope() };
    return { agents, device: null, workspace: { id, label: id, device_id: "local", path: `/fixture/${id}`, registered: true, temporary: false, pinned: false, checkouts: [row], inactive_checkouts: { expanded: false, checkout_ids: [] }, agent_scope: { ...emptyScope(), members: agents.map((agent) => ({ pane_id: agent.pane_id, project_id: id, checkout_id: row.id })) } } };
  };
  const root = legacyAgentRow({ id: "r", pane_id: "r", identity_label: "부모", agent_kind: "claude", session_id: "native-r", group: "working", activity: "working" });
  const sleeper = legacyAgentRow({ id: "s", pane_id: "s", identity_label: "잠든 자식", agent_kind: "claude", session_id: "native-s", mark: "asleep", status_code: "sleeping", delegated: true, lineage_parent_pane_id: "r" });
  const alone = legacyAgentRow({ id: "z", pane_id: "z", identity_label: "혼자 잠듦", agent_kind: "claude", session_id: "native-z", mark: "asleep", status_code: "sleeping" });
  const projects = [board("alpha", [root, sleeper]), board("beta", [alone])];
  const rest = { navigator: { devices: [{ id: "local", kind: "local" }], workspaces: projects.map((project) => project.workspace), focused_device_id: "local" }, status: {},
    overview_graph: { available: true, limit: 2000, retained_count: 0, truncated: false, retention_ms: 86400000, letters: [], sessions: ["r", "s", "z"].map((pane) => ({ device_id: "local", pane_id: pane, session_digest: `digest-${pane}`, launch_model: null, launch_effort: null })) } } as unknown as SnapshotRest;
  useShellStore.setState({ rest, agents: [root, sleeper, alone], focusedPaneId: null, connection: "live", linkSummaries: null });
  useUiStore.setState({ overviewOpen: false, opening: null, screen: { kind: "main", deviceId: "local" } });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const reactRoot = createRoot(container);
  const head = () => container.querySelector("[data-graph-preview] .og-details-head")?.textContent;
  const open = async (pane: string) => {
    await act(async () => reactRoot.render(<TooltipProvider><div /></TooltipProvider>));
    actions.openPaneInOverview(pane);
    await act(async () => reactRoot.render(<TooltipProvider><OverviewGraph projects={projects} deviceId="local" actions={actions} /></TooltipProvider>));
  };
  try {
    await open("s");
    expect(head()).toContain("잠든 자식");
    expect(events.map((event) => (event.payload as { change: unknown }).change)).toEqual([{ kind: "team_folded", session: { device_id: "local", pane_id: "r", session: "digest-r" }, folded: false }]);
    await open("z");
    expect(head()).toContain("혼자 잠듦");
    expect(useUiStore.getState().overview.projectId).toBeNull();
  } finally { await act(async () => reactRoot.unmount()); }
});

// PRD pane-open-in-overview B9: a pane whose project the graph budget leaves out opens that project's scope with its session chosen.
it("opens the project scope for a pane outside the graph budget", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  initializeInterfaceI18n("en");
  const projects = Array.from({ length: OUTSIDE + 1 }, (_, index): BoardProject => {
    const id = `project-${String(index).padStart(3, "0")}`;
    const pane = `pane-${index}`;
    const checkout: Checkout = { id: `${id}-main`, workspace_id: id, label: "main", path: `/fixture/${id}`, branch: "main", purpose: null, is_worktree: false, exists: true, has_panes: true, pull_request: null, tabs: [], active_tab_id: null, next_tab_label: "Tab 1", strip: [], agent_scope: emptyScope() };
    const agent = legacyAgentRow({ id: pane, pane_id: pane, identity_label: `Session ${index}`, agent_kind: "claude", session_id: `native-${index}`, group: "working", activity: "working" });
    return { agents: [agent], device: null, workspace: { id, label: id, device_id: "local", path: `/fixture/${id}`, registered: true, temporary: false, pinned: false, checkouts: [checkout], inactive_checkouts: { expanded: false, checkout_ids: [] }, agent_scope: { ...emptyScope(), members: [{ pane_id: pane, project_id: id, checkout_id: checkout.id }] } } };
  });
  const rest = { navigator: { devices: [{ id: "local", kind: "local" }], workspaces: projects.map((project) => project.workspace), focused_device_id: "local" }, status: {},
    overview_graph: { available: true, limit: 2000, retained_count: 0, truncated: false, retention_ms: 86400000, letters: [], sessions: projects.map((project) => ({ device_id: "local", pane_id: project.agents[0]!.pane_id, session_digest: `digest-${project.agents[0]!.pane_id}`, launch_model: null, launch_effort: null })) } } as unknown as SnapshotRest;
  useShellStore.setState({ rest, agents: projects.flatMap((project) => project.agents), focusedPaneId: null, connection: "live", linkSummaries: null });
  useUiStore.setState({ overviewOpen: false, opening: null, screen: { kind: "main", deviceId: "local" } });
  const actions = createActions(() => true);
  const container = document.createElement("div");
  document.body.append(container);
  const reactRoot = createRoot(container);
  try {
    actions.openPaneInOverview(`pane-${OUTSIDE}`);
    await act(async () => reactRoot.render(<TooltipProvider><OverviewGraph projects={projects} deviceId="local" actions={actions} /></TooltipProvider>));
    expect(useUiStore.getState().overview.projectId).toBe(OUTSIDE_PROJECT);
    expect(container.querySelector("[data-graph-preview] .og-details-head")?.textContent).toContain(`Session ${OUTSIDE}`);
  } finally { await act(async () => reactRoot.unmount()); }
});

// The side column's search finds sessions as well as projects: a found session is shown on the map
// as a pane's Show in Overview shows it, opening its hidden project and the folded project and team it is in.
it("finds a session from the side column's search and shows it, opening what hid it", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  initializeInterfaceI18n("en");
  const checkout: Checkout = { id: "alpha-main", workspace_id: "alpha", label: "main", path: "/fixture/alpha", branch: "feat/map", purpose: null, is_worktree: false, exists: true, has_panes: true, pull_request: null, tabs: [], active_tab_id: null, next_tab_label: "Tab 1", strip: [], agent_scope: emptyScope() };
  const root = legacyAgentRow({ id: "r", pane_id: "r", identity_label: "배포 확인", agent_kind: "claude", session_id: "native-r", group: "working", activity: "working" });
  const child = legacyAgentRow({ id: "c", pane_id: "c", identity_label: "검증 자식", agent_kind: "codex", session_id: "native-c", group: "working", activity: "working", delegated: true, lineage_parent_pane_id: "r" });
  const projects: BoardProject[] = [{ agents: [root, child], device: null, workspace: { id: "alpha", label: "알파", device_id: "local", path: "/fixture/alpha", registered: true, temporary: false, pinned: false, checkouts: [checkout], inactive_checkouts: { expanded: false, checkout_ids: [] }, agent_scope: { ...emptyScope(), members: [root, child].map((agent) => ({ pane_id: agent.pane_id, project_id: "alpha", checkout_id: checkout.id })) } } }];
  const alpha = { device_id: "local", workspace_id: "alpha" };
  const rest = { navigator: { devices: [{ id: "local", kind: "local" }], workspaces: projects.map((project) => project.workspace), focused_device_id: "local" }, status: {},
    overview_graph: { available: true, limit: 2000, retained_count: 0, truncated: false, retention_ms: 86400000, letters: [], sessions: ["r", "c"].map((pane) => ({ device_id: "local", pane_id: pane, session_digest: `digest-${pane}`, launch_model: null, launch_effort: null })) },
    ui_state: { overview_graph: { ...GRAPH_CHOICES_DEFAULTS, hidden_projects: [alpha], folded_projects: [alpha], team_folds: [{ session: { device_id: "local", pane_id: "r", session: "digest-r" }, folded: true }] } },
  } as unknown as SnapshotRest;
  useShellStore.setState({ rest, agents: [root, child], focusedPaneId: null, connection: "live", linkSummaries: null });
  useUiStore.getState().setOverview({ projectId: null, panel: null });
  useUiStore.setState({ overviewOpen: false, opening: null, screen: { kind: "main", deviceId: "local" } });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const reactRoot = createRoot(container);
  const search = () => container.querySelector<HTMLInputElement>('[data-graph-projects] input[aria-label="Find project or session"]')!;
  const type = (value: string) => act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(search(), value);
    search().dispatchEvent(new Event("input", { bubbles: true }));
  });
  const found = () => [...container.querySelectorAll("[data-graph-found-session]")].map((row) => row.getAttribute("data-graph-found-session"));
  const head = () => container.querySelector("[data-graph-preview] .og-details-head")?.textContent;
  try {
    await act(async () => reactRoot.render(<TooltipProvider><OverviewGraph projects={projects} deviceId="local" actions={actions} /></TooltipProvider>));
    // A session is found by its own title, and its hidden project is said beside it.
    await type("검증");
    expect(found()).toEqual(["c"]);
    expect(container.querySelector('[data-graph-found-session="c"] .og-hidden-mark')?.textContent).toBe("Hidden on the graph");
    expect(container.querySelector("[data-graph-project-row]")).toBeNull();
    // A project's name finds the project, not each of its sessions again.
    await type("알파");
    expect(found()).toEqual([]);
    expect(container.querySelector('[data-graph-project-row="alpha"]')).not.toBeNull();
    await type("없는 이름");
    expect(container.querySelector(".og-no-match")?.textContent).toBe("Nothing matches");
    // Choosing the found session opens what hides it, one change each, and its details.
    await type("검증");
    await act(async () => container.querySelector<HTMLButtonElement>('[data-graph-found-session="c"]')!.click());
    expect(head()).toContain("검증 자식");
    expect(events.map((event) => (event.payload as { change: unknown }).change)).toEqual([
      { kind: "project_hidden", project: alpha, hidden: false },
      { kind: "project_folded", project: alpha, folded: false },
      { kind: "team_folded", session: { device_id: "local", pane_id: "r", session: "digest-r" }, folded: false },
    ]);
    // Escape goes back to the column with the search as it was; Enter takes the first found session.
    await act(async () => useUiStore.getState().escapeLayers.at(-1)!());
    expect(search().value).toBe("검증");
    await type("배포");
    await act(async () => search().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    expect(head()).toContain("배포 확인");
  } finally { await act(async () => reactRoot.unmount()); }
});

// An Overview with no session awake: nothing to filter or arrange, so its one
// next step, New agent, takes the map's place (UI_BEHAVIOR, Overview Work).
it("with no session awake, offers New agent in the one project in scope and keeps sleeping sessions one press away", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  initializeInterfaceI18n("en");
  const checkout: Checkout = { id: "checkout", workspace_id: "project", label: "main", path: "/fixture/project", branch: "main", purpose: null, is_worktree: false, exists: true, has_panes: true, pull_request: null, tabs: [], active_tab_id: null, next_tab_label: null } as unknown as Checkout;
  const workspace = { id: "project", label: "Project", device_id: "local", path: "/fixture/project", registered: true, temporary: false, pinned: false, checkouts: [checkout], inactive_checkouts: { expanded: false, checkout_ids: [] }, agent_scope: emptyScope() } as unknown as Workspace;
  const sleeping = legacyAgentRow({ id: "p1", pane_id: "p1", device_id: "local", identity_label: "Slept work", mark: "asleep", status_code: "sleeping", activity: "stopped", agent_kind: "claude", session_id: "native-slept" });
  const graph = (include_resting: boolean) => ({ ...GRAPH_CHOICES_DEFAULTS, include_resting });
  const draw = (agents: typeof sleeping[], include_resting = false) => {
    const rest = { navigator: { devices: [{ id: "local", kind: "local" }], workspaces: [{ ...workspace, agent_scope: { ...emptyScope(), members: agents.map((row) => ({ pane_id: row.pane_id, project_id: "project", checkout_id: "checkout" })) } }], focused_device_id: "local" }, status: { remote: [] }, ui_state: { overview_graph: graph(include_resting) }, overview_graph: { available: true, limit: 2000, retained_count: 0, truncated: false, retention_ms: 86400000, letters: [], sessions: [] } } as unknown as SnapshotRest;
    useShellStore.setState({ rest, agents, focusedPaneId: null, connection: "live", linkSummaries: null });
    return [{ workspace: rest.navigator!.workspaces![0]!, agents, device: null }] as BoardProject[];
  };
  useUiStore.getState().setOverview({ projectId: null, panel: null });
  useStartPanel.setState({ isOpen: false, preset: null });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const render = (projects: BoardProject[]) => act(async () => root.render(<TooltipProvider><OverviewGraph projects={projects} deviceId="local" actions={actions} /></TooltipProvider>));
  try {
    await render(draw([]));
    const welcome = container.querySelector("[data-graph-welcome]")!;
    expect(welcome.querySelector("h2")?.textContent).toBe("Hand an agent some work");
    expect(welcome.querySelector("p")?.textContent).toBe("Starts in Project");
    expect(container.querySelector(".og-filters")).toBeNull();
    expect(welcome.querySelector('[data-graph-welcome-action="show_resting"]')).toBeNull();
    await act(async () => welcome.querySelector<HTMLButtonElement>('[data-graph-welcome-action="start_agent"]')!.click());
    expect(useStartPanel.getState()).toMatchObject({ isOpen: true, preset: { projectId: "project" } });

    // Only a sleeping session: the welcome stays, and one press shows every session.
    await render(draw([sleeping]));
    const show = container.querySelector<HTMLButtonElement>('[data-graph-welcome] [data-graph-welcome-action="show_resting"]')!;
    expect(show.textContent).toBe("Show sleeping sessions (1)");
    await act(async () => show.click());
    expect(events.at(-1)).toMatchObject({ kind: "overview_graph_change", payload: { change: { kind: "focus", focus: "all", include_resting: true } } });

    // Once the core keeps sleeping sessions on the map, the map draws them.
    await render(draw([sleeping], true));
    expect(container.querySelector("[data-graph-welcome]")).toBeNull();
    expect(container.querySelector(".og-filters")).not.toBeNull();

    // Two projects and none chosen: the start panel picks the place, so no project is named.
    await render([...draw([]), { workspace: { ...workspace, id: "other", label: "Other" }, agents: [], device: null } as BoardProject]);
    expect(container.querySelector("[data-graph-welcome] p")).toBeNull();
  } finally { await act(async () => root.unmount()); }
});

it("opens the start panel on a project from its row's +, its start circle and a folded name's +, on no project from You's +, and Add project from its place (issue 2514)", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  initializeInterfaceI18n("en");
  const checkout: Checkout = { id: "checkout", workspace_id: "project", label: "main", path: "/fixture/project", branch: "main", purpose: null, is_worktree: false, exists: true, has_panes: true, pull_request: null, tabs: [], active_tab_id: null, next_tab_label: "Tab 1", strip: [], agent_scope: emptyScope() };
  const agent = legacyAgentRow({ id: "p1", pane_id: "p1", identity_label: "Map work", agent_kind: "claude", session_id: "native-p1", group: "working", activity: "working" });
  const workspace: Workspace = { id: "project", label: "Project", device_id: "local", path: "/fixture/project", registered: true, temporary: false, pinned: false, checkouts: [checkout], inactive_checkouts: { expanded: false, checkout_ids: [] }, agent_scope: { ...emptyScope(), members: [{ pane_id: "p1", project_id: "project", checkout_id: checkout.id }] } };
  const projects: BoardProject[] = [{ workspace, agents: [agent], device: null }];
  const rest = { navigator: { devices: [{ id: "local", kind: "local" }], workspaces: [workspace], focused_device_id: "local" }, status: {}, overview_graph: { available: true, limit: 2000, retained_count: 0, truncated: false, retention_ms: 86400000, letters: [], sessions: [] } } as unknown as SnapshotRest;
  useShellStore.setState({ rest, agents: [agent], focusedPaneId: null, connection: "live", linkSummaries: null });
  useUiStore.getState().setOverview({ projectId: null, panel: null });
  useUiStore.setState({ overviewOpen: true, opening: null, screen: { kind: "workspace" } });
  const savedPanel = useStartPanel.getState();
  useStartPanel.setState({ isOpen: false, preset: null });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => root.render(<TooltipProvider><OverviewGraph projects={projects} deviceId="local" actions={actions} /></TooltipProvider>));
    const row = container.querySelector('[data-graph-project-row="project"]')!;
    // The row's own press still moves the camera and opens nothing.
    await act(async () => row.querySelector<HTMLButtonElement>('button[aria-label="Locate Project"]')!.click());
    expect(useStartPanel.getState().isOpen).toBe(false);
    // Its `+` opens the start panel on the project, asking the start to keep the Overview in front.
    const plus = row.querySelector<HTMLButtonElement>('[data-graph-project-row-start="project"]')!;
    expect(plus.getAttribute("aria-label")).toBe("Start agent in Project");
    await act(async () => plus.click());
    expect(useStartPanel.getState()).toMatchObject({ isOpen: true, preset: { projectId: "project" }, stay: true });
    expect(useUiStore.getState()).toMatchObject({ overviewOpen: true, opening: null });
    expect(events).toHaveLength(0);
    await act(async () => root.unmount());

    // On the map: an unfolded name is only the drag handle and its fold; the project's start circle and a
    // folded project's `+` beside its name start there; You's `+` starts on no project; the Add project place adds one.
    const start = vi.fn();
    const fold = vi.fn();
    const anywhere = vi.fn();
    const add = vi.fn();
    const group = { id: "project", project: { workspace } } as unknown as ProjectNode["data"]["group"];
    const summary = { id: "project", project: { workspace }, sessions: 1, turn: 0, working: 1, results: 0 } as unknown as ClusterNode["data"]["summary"];
    const map = createRoot(container);
    await act(async () => map.render(<TooltipProvider><ReactFlowProvider>
      <ProjectGraphNode {...{ data: { group, accent: "sky", current: false, emphasized: false, muted: false, fold, point: () => {} } } as unknown as NodeProps<ProjectNode>} />
      <StartSlotGraphNode {...{ data: { workspaceId: "project", label: "Start agent in Project", accent: "sky", muted: false, start } } as unknown as NodeProps<StartSlotNode>} />
      <ClusterGraphNode {...{ data: { summary, accent: "sky", orb: 40, emphasized: false, muted: false, unfold: fold, point: () => {}, start } } as unknown as NodeProps<ClusterNode>} />
      <OperatorGraphNode {...{ data: { label: "You", startLabel: "New agent", start: anywhere } } as unknown as NodeProps<OperatorNode>} />
      <AddProjectGraphNode {...{ data: { label: "Add project", add } } as unknown as NodeProps<AddProjectNode>} />
    </ReactFlowProvider></TooltipProvider>));
    expect(container.querySelector(".og-project button[aria-label='Start agent in Project']")).toBeNull();
    // A drag from a control must never end in its click: the start circle leaves the drag to the map's pan,
    // the folded name's `+` is one of the folded project's drag handles (as its circle is).
    const slot = container.querySelector<HTMLButtonElement>('[data-graph-start-slot="project"]')!;
    const folded = container.querySelector<HTMLButtonElement>('.og-cluster [data-graph-project-start="project"]')!;
    expect(slot.closest(".og-project-drag, .og-cluster-drag")).toBeNull();
    expect([slot.classList.contains("nodrag"), slot.classList.contains("nopan")]).toEqual([true, false]);
    expect([folded.classList.contains("og-cluster-drag"), folded.classList.contains("nodrag")]).toEqual([true, false]);
    for (const button of [slot, folded]) {
      expect(button.getAttribute("aria-label")).toBe("Start agent in Project");
      await act(async () => button.click());
    }
    expect(start.mock.calls).toEqual([["project"], ["project"]]);
    const operator = container.querySelector<HTMLButtonElement>("[data-graph-operator-start]")!;
    expect(operator.getAttribute("aria-label")).toBe("New agent");
    expect(operator.classList.contains("nopan")).toBe(false);
    await act(async () => operator.click());
    expect(anywhere).toHaveBeenCalledTimes(1);
    const adding = container.querySelector<HTMLButtonElement>("[data-graph-add-project]")!;
    expect(adding.textContent).toBe("Add project");
    expect(adding.classList.contains("nopan")).toBe(false);
    await act(async () => adding.click());
    expect(add).toHaveBeenCalledTimes(1);
    expect(fold).not.toHaveBeenCalled();
    await act(async () => map.unmount());

    // You's `+` through the map's own action: the panel names no project and the start keeps the Overview.
    useStartPanel.setState({ isOpen: false, preset: null, stay: false });
    actions.openStartPanel(undefined, { stay: true });
    expect(useStartPanel.getState()).toMatchObject({ isOpen: true, preset: null, stay: true });
  } finally {
    useStartPanel.setState(savedPanel, true);
  }
});
