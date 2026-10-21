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
  const other = legacyAgentRow({ id: "o", pane_id: "o", identity_label: "다른 일", agent_kind: "claude", session_id: "native-o", group: "working", activity: "working" });
  const board = (id: string, label: string, agents: typeof root[]): BoardProject => {
    const row = checkout(id);
    return { agents, device: null, workspace: { id, label, device_id: "local", path: `/fixture/${id}`, registered: true, temporary: false, pinned: false, checkouts: [row], inactive_checkouts: { expanded: false, checkout_ids: [] }, agent_scope: { ...emptyScope(), members: agents.map((agent) => ({ pane_id: agent.pane_id, project_id: id, checkout_id: row.id })) } } };
  };
  const projects = [board("alpha", "알파", [root, child]), board("beta", "베타", [other])];
  const party = (pane: string) => ({ device_id: "local", pane_id: pane, session: `digest-${pane}` });
  const rest = { navigator: { devices: [{ id: "local", kind: "local" }], workspaces: projects.map((project) => project.workspace), focused_device_id: "local" }, status: {}, overview_graph: {
    available: true, limit: 2000, retained_count: 2, truncated: false, retention_ms: 86400000,
    letters: [{ id: "l1", sender: party("r"), recipient: party("o"), kind: "request", at_unix_ms: 10 }, { id: "l2", sender: party("o"), recipient: party("r"), kind: "report", at_unix_ms: 20 }, { id: "l3", sender: party("o"), recipient: party("r"), kind: "progress", at_unix_ms: 30 }],
    direct_inputs: [{ id: "d1", sender: party("c"), recipient: party("r"), at_unix_ms: 40 }],
    sessions: ["r", "c", "o"].map((pane) => ({ device_id: "local", pane_id: pane, session_digest: `digest-${pane}`, launch_model: null })),
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
    const team = preview()!.querySelector(".og-team-list")!;
    expect(team.textContent).toContain("검증 자식");
    const talks = [...preview()!.querySelectorAll(".og-talks > div")].map((row) => row.textContent);
    expect(talks).toEqual(expect.arrayContaining([expect.stringContaining("Sent 1 · Received 2"), expect.stringContaining("Received 1")]));
    expect(preview()!.textContent).not.toContain("request");
    // A team row chooses that descendant.
    await act(async () => team.querySelector<HTMLButtonElement>("button")!.click());
    expect(preview()?.querySelector(".og-details-head")?.textContent).toContain("검증 자식");
    // Escape closes the panel first (the shell's keyboard pops the top layer); the session stays highlighted.
    await act(async () => useUiStore.getState().escapeLayers.at(-1)!());
    expect(preview()).toBeNull();
    const listed = (name: string) => [...container.querySelectorAll<HTMLButtonElement>("[data-graph-session-list] > button")].find((row) => row.textContent?.includes(name))!;
    expect(listed("검증 자식").getAttribute("aria-pressed")).toBe("true");
    // B14: hiding the highlighted session's project clears it; the row stays, dimmed, with its counts.
    await act(async () => button("Hide 알파 on the map")!.click());
    expect(listed("검증 자식").getAttribute("aria-pressed")).toBe("false");
    const hidden = column().querySelector('[data-graph-project-row="alpha"]')!;
    expect(hidden.getAttribute("data-hidden")).toBe("true");
    expect(hidden.querySelector('[data-state="turn"]')?.textContent).toContain("1");
    expect(column().querySelector('[data-graph-ask="r"]')).toBeNull();
    await act(async () => button("Show 알파 on the map again")!.click());
    expect(column().querySelector('[data-graph-project-row="alpha"]')?.getAttribute("data-hidden")).toBe("false");
  } finally { await act(async () => reactRoot.unmount()); }
});
