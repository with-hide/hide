// @vitest-environment jsdom
// The mission screen (PRD mission-screen B2-B13, B22, D-16): what it shows of
// the device's missions in each state of the record, where a press takes it,
// and the one event that tells the core which mission the detail shows.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { initializeInterfaceI18n } from "./i18n/instance";
import { MissionScreen } from "./MissionScreen";
import type { AgentRow, Mission, SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { MISSION_ENTRY, useUiStore, type MissionPlace } from "./ui";
import type { DispatchFn } from "./ws";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
const savedShell = useShellStore.getState();
const savedUi = useUiStore.getState();
afterEach(() => {
  useShellStore.setState(savedShell, true);
  useUiStore.setState(savedUi, true);
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

function mission(id: string, extra: Partial<Mission> = {}): Mission {
  return { id, title: `Mission ${id}`, workspace_id: "w", project: "/repo", checkout_path: "/repo", device_id: "mac", status: "running", scope: "pr", cap: 4, exempt: false, strength: null, conductor: { pane_id: "p1", name: "conductor", agent_kind: "claude", session: null, present: true, takes_answers: true }, nodes: [], merge_order: [], landing_order: [], stack_waiting: 0, check: null, github: null, decision: null, question: null, answer: null, answer_failed: null, delay: null, report: null, created_at_unix_ms: 1, started_at_unix_ms: Date.now() - 180_000, closed_at_unix_ms: null, running: 0, stale: false, github_stale: false, github_read_at_unix_ms: null, ...extra };
}

const MISSIONS = [
  mission("mission-1", { decision: "Which order?" }),
  mission("mission-2", { created_at_unix_ms: 2 }),
  mission("mission-3", { workspace_id: "w2", project: "/other" }),
  mission("mission-9", { device_id: "mini" }),
];

type Setup = { missions?: { items: Mission[]; failed?: string } | null; place?: Partial<MissionPlace>; remote?: unknown[]; agents?: AgentRow[] };

async function mount({ missions = { items: MISSIONS }, place = {}, remote = [], agents = [] }: Setup = {}) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  initializeInterfaceI18n("en");
  useShellStore.setState({
    connection: "live",
    agents,
    rest: {
      ...(missions ? { missions } : {}),
      navigator: {
        devices: [{ id: "mac", kind: "local", label: "This Mac" }, { id: "mini", kind: "remote", label: "mini" }],
        focused_device_id: "mac",
        workspaces: [{ id: "w", label: "herdr-ide", device_id: "mac", checkouts: [] }, { id: "w2", label: "atorni-ax", device_id: "mac", checkouts: [] }],
      },
      status: { remote },
      overview_graph: { sessions: [] },
    } as unknown as SnapshotRest,
  });
  useUiStore.setState({ screen: { kind: "missions" }, missions: { ...MISSION_ENTRY, ...place } });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><MissionScreen actions={actions} /></TooltipProvider>));
  const q = (selector: string) => container.querySelector<HTMLElement>(selector);
  const all = (selector: string) => Array.from(container.querySelectorAll<HTMLElement>(selector));
  const press = async (element: HTMLElement | null) => {
    if (!element) throw new Error("nothing to press");
    await act(async () => element.click());
  };
  const shown = () => events.filter((event) => event.kind === "mission_shown").map((event) => (event.payload as { mission: string | null }).mission);
  return { container, events, q, all, press, shown, actions, unmount: async () => { await act(async () => root.unmount()); } };
}

describe("the table (B2-B5)", () => {
  it("lists the front device's missions of every project in their groups, with the device and the count in the head", async () => {
    const { q, all } = await mount();
    expect(q("[data-mission-screen-view]")?.dataset.missionScreenView).toBe("table");
    expect(all("[data-mission-group]").map((group) => [group.dataset.missionGroup, group.dataset.missionGroupCount])).toEqual([["turn", "1"], ["running", "2"]]);
    expect(all("[data-mission-row]").map((row) => row.dataset.missionRow)).toEqual(["mission-1", "mission-2", "mission-3"]);
    expect(q("[data-mission-screen-count]")?.dataset.missionScreenCount).toBe("3");
    expect(q("[data-mission-screen-device-name]")?.textContent).toBe("· This computer");
    expect(q('[data-mission-row-project="w2"]')?.textContent).toBe("atorni-ax");
  });

  it("narrows the rows and the count to a project's chip, and back with 모든 프로젝트 (B3)", async () => {
    const { q, all, press } = await mount();
    await press(q('[data-mission-project-chip="w2"]'));
    expect(all("[data-mission-row]").map((row) => row.dataset.missionRow)).toEqual(["mission-3"]);
    expect(q("[data-mission-screen-count]")?.dataset.missionScreenCount).toBe("1");
    await press(q('[data-mission-project-chip="all"]'));
    expect(all("[data-mission-row]")).toHaveLength(3);
  });

  it("says how to start one with none on the device, and offers every project when a chip leaves none (B7)", async () => {
    const empty = await mount({ missions: { items: [mission("m", { device_id: "mini" })] } });
    expect(empty.q("[data-mission-screen-state]")?.dataset.missionScreenState).toBe("empty");
    expect(empty.container.textContent).toContain("/hide-mission");
    await empty.unmount();
    // The chip's project lost its last mission: the table reads as every project again rather than an empty page.
    const gone = await mount({ place: { projectId: "w-gone" } });
    expect(gone.all("[data-mission-row]")).toHaveLength(3);
  });

  it("draws quiet rows and no count before the missions arrive, and one muted line and no count when they cannot be read (B8)", async () => {
    const loading = await mount({ missions: null });
    expect(loading.q("[data-mission-screen-state]")?.dataset.missionScreenState).toBe("loading");
    expect(loading.q("[data-mission-screen-count]")).toBeNull();
    await loading.unmount();
    const failed = await mount({ missions: { items: [], failed: "book: unreadable" } });
    expect(failed.q("[data-mission-screen-state]")?.textContent).toBe("Couldn't read the missions");
    expect(failed.q("[data-mission-screen-count]")).toBeNull();
    expect(failed.container.textContent).not.toContain("unreadable");
  });

  it("marks a device that does not answer, dims its rows and holds 이어받기 (B9)", async () => {
    const orphan = mission("mission-9", { device_id: "mini", conductor: { pane_id: "p1", name: "c", agent_kind: "claude", session: null, present: false, takes_answers: true } });
    const { q } = await mount({ missions: { items: [orphan] }, place: { deviceId: "mini" }, remote: [{ target_id: "mini", state: "not_connected", session: null }] });
    expect(q("[data-mission-screen-offline]")).not.toBeNull();
    expect(q('[data-mission-row="mission-9"]')?.dataset.missionRowStale).toBe("true");
    expect(q("[data-mission-takeover]")).toHaveProperty("disabled", true);
  });
});

describe("a mission's detail (B10, B11, B22, D-16)", () => {
  it("opens beside the list from a row, tells the core which mission it shows, and ✕ returns to the table and tells it none", async () => {
    const { q, press, shown } = await mount();
    // The table names no mission, and says nothing that would take another window's detail its events.
    expect(shown()).toEqual([]);
    await press(q('[data-mission-row="mission-2"] [data-slot="row-overlay-button"]'));
    expect(q("[data-mission-screen-view]")?.dataset.missionScreenView).toBe("detail");
    expect(q("[data-mission-screen-detail]")?.dataset.missionScreenDetail).toBe("mission-2");
    expect(q('[data-mission-list-row="mission-2"]')?.getAttribute("aria-current")).toBe("true");
    expect(shown()).toEqual(["mission-2"]);
    await press(q("[data-mission-screen-close]"));
    expect(q("[data-mission-screen-view]")?.dataset.missionScreenView).toBe("table");
    expect(shown()).toEqual(["mission-2", null]);
  });

  it("names its mission again when the connection comes back, since a dispatch while it was down was dropped (D-16)", async () => {
    const { shown } = await mount({ place: { selected: "mission-1" } });
    act(() => useShellStore.setState({ connection: "reconnecting" }));
    act(() => useShellStore.setState({ connection: "live" }));
    expect(shown()).toEqual(["mission-1", "mission-1"]);
  });

  it("steps to the next and previous mission in the list's order with ↓ and ↑ (B11)", async () => {
    const { q, shown } = await mount({ place: { selected: "mission-1" } });
    const list = q("[data-mission-list]")!;
    await act(async () => list.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true })));
    expect(q("[data-mission-screen-detail]")?.dataset.missionScreenDetail).toBe("mission-2");
    await act(async () => list.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true })));
    await act(async () => list.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true })));
    expect(q("[data-mission-screen-detail]")?.dataset.missionScreenDetail).toBe("mission-1");
    expect(shown().at(-1)).toBe("mission-1");
  });

  it("returns to the table without a word when the mission it shows is gone (B22)", async () => {
    const { q, shown } = await mount({ place: { selected: "mission-2" } });
    await act(async () => useShellStore.setState((state) => ({ rest: { ...state.rest!, missions: { items: MISSIONS.filter((row) => row.id !== "mission-2") } } })));
    expect(q("[data-mission-screen-view]")?.dataset.missionScreenView).toBe("table");
    expect(shown().at(-1)).toBeNull();
  });

  it("tells the core it shows none once the screen goes (D-16)", async () => {
    const { unmount, shown } = await mount({ place: { selected: "mission-1" } });
    await unmount();
    expect(shown().at(-1)).toBeNull();
  });
});

describe("a project chip while a mission shows (B10)", () => {
  it("returns to the chip's table when the chip leaves the mission out, and keeps the detail when it holds it", async () => {
    const { q, press } = await mount({ place: { selected: "mission-1" } });
    await press(q('[data-mission-project-chip="w"]'));
    expect(q("[data-mission-screen-detail]")?.dataset.missionScreenDetail).toBe("mission-1");
    await press(q('[data-mission-project-chip="all"]'));
    expect(q("[data-mission-screen-detail]")?.dataset.missionScreenDetail).toBe("mission-1");
    await press(q('[data-mission-project-chip="w2"]'));
    expect(q("[data-mission-screen-view]")?.dataset.missionScreenView).toBe("table");
    expect(q("[data-mission-screen-count]")?.dataset.missionScreenCount).toBe("1");
  });
});

describe("what the detail opens in the 보기 slot (D-09)", () => {
  it("leaves the screen for the Workspace in front, where the slot draws a node's plan or a worker's record", async () => {
    const { actions, events } = await mount({ place: { selected: "mission-1" } });
    act(() => useShellStore.setState((state) => ({
      rest: { ...state.rest!, workspace_view: {}, navigator: { ...state.rest!.navigator!, focused_checkout_id: "c1", workspaces: [{ id: "w", label: "herdr-ide", device_id: "mac", checkouts: [{ id: "c1", path: "/repo" }] }] } } as unknown as SnapshotRest,
    })));
    const session = { pane_id: "w1:p4", device_id: "mac" } as never;
    // A node's `크게 ›` keeps the plan in the slot the map was pressed in.
    act(() => actions.openPlan(session, false));
    expect(useUiStore.getState().screen).toEqual({ kind: "workspace" });
    expect(events.filter((event) => event.kind === "detail_open").map((event) => (event.payload as { item: { kind: string } }).item.kind)).toEqual(["plan"]);
    act(() => useUiStore.setState({ screen: { kind: "missions" } }));
    act(() => actions.openChildRecord(session));
    expect(useUiStore.getState().screen).toEqual({ kind: "workspace" });
    expect(events.filter((event) => event.kind === "detail_open").map((event) => (event.payload as { item: { kind: string } }).item.kind)).toEqual(["plan", "record"]);
  });
});

describe("the turn pill (B5)", () => {
  it("words the operator's action with no `!`, draws no pill on the row that has the 이어받기 button, and opens a row on a press", async () => {
    const gone = mission("mission-4", { conductor: { pane_id: "p1", name: "conductor", agent_kind: "claude", session: null, present: false, takes_answers: true } });
    const { q, press, actions } = await mount({ missions: { items: [mission("mission-1", { decision: "Which order?" }), gone] } });
    const row = (id: string) => q(`[data-mission-row="${id}"]`)!;
    expect(row("mission-4").dataset.missionRowGroup).toBe("turn");
    expect(row("mission-4").querySelector("[data-mission-row-pill]")).toBeNull();
    expect(row("mission-4").querySelector("[data-mission-takeover]")?.textContent).toBe("Resume command");
    expect(row("mission-1").querySelector("[data-mission-row-pill]")?.textContent).toBe("Answer");
    for (const id of ["mission-1", "mission-4"]) expect(row(id).textContent).not.toContain("!");
    const open = vi.spyOn(actions, "showMissionOnScreen");
    await press(row("mission-4").querySelector<HTMLElement>('[data-slot="row-overlay-button"]'));
    expect(open).toHaveBeenCalledWith("mission-4");
    // The narrow list beside a detail marks a turn as every other row, with the mission mark and the action under the title, and no `!`.
    act(() => actions.showMissionOnScreen("mission-1"));
    const list = q('[data-mission-list-row="mission-4"]')!;
    expect(list.textContent).toContain("Resume command");
    expect(list.textContent).not.toContain("!");
    expect(q('[data-mission-list-row="mission-1"]')!.textContent).not.toContain("!");
  });
});

describe("opening a mission (B2, B13, D-04; operator decision 2026-11-09)", () => {
  it("opens a mission pressed outside the screen as a card in its Conductor's tab, leaving the screen for the Workspace", async () => {
    const { actions, events } = await mount({ place: { selected: "mission-2" } });
    act(() => useShellStore.setState((state) => ({
      rest: { ...state.rest!, workspace_view: {}, navigator: { ...state.rest!.navigator!, focused_checkout_id: "c1", workspaces: [{ id: "w", label: "herdr-ide", device_id: "mac", checkouts: [{ id: "c1", path: "/repo" }] }] } } as unknown as SnapshotRest,
    })));
    act(() => actions.openMission("mission-1"));
    expect(useUiStore.getState().screen).toEqual({ kind: "workspace" });
    expect(events.filter((event) => event.kind === "detail_open").map((event) => event.payload)).toEqual([{ item: { kind: "mission", mission: "mission-1" }, in_owner_tab: true }]);
    // The screen's own detail is left as it was, for the sidebar row to come back to.
    expect(useUiStore.getState().missions.selected).toBe("mission-2");
  });

  it("opens a row of the screen on the screen, bringing another device forward first", async () => {
    const { actions, events } = await mount();
    act(() => useUiStore.setState({ screen: { kind: "workspace" } }));
    act(() => actions.showMissionOnScreen("mission-9"));
    expect(useUiStore.getState().screen).toEqual({ kind: "missions" });
    expect(useUiStore.getState().missions).toMatchObject({ deviceId: "mini", selected: "mission-9" });
    expect(events.filter((event) => event.kind === "focus_device").map((event) => event.payload)).toEqual([{ device_id: "mini" }]);
    // The sidebar row always opens the table on the device in front, every project and 끝남 folded.
    act(() => useUiStore.getState().setMissions({ projectId: "w2", closedAll: true }));
    act(() => actions.openMissions());
    expect(useUiStore.getState().missions).toEqual(MISSION_ENTRY);
  });

  it("lets go of a chip that leaves the mission out, so the list beside it holds the mission (B10)", async () => {
    const { actions, q } = await mount({ place: { projectId: "w2" } });
    act(() => actions.showMissionOnScreen("mission-1"));
    expect(useUiStore.getState().missions.projectId).toBeNull();
    expect(q('[data-mission-list-row="mission-1"]')?.getAttribute("aria-current")).toBe("true");
    act(() => actions.showMissionOnScreen("mission-3"));
    act(() => useUiStore.getState().setMissions({ projectId: "w2" }));
    act(() => actions.showMissionOnScreen("mission-3"));
    expect(useUiStore.getState().missions.projectId).toBe("w2");
  });
});
