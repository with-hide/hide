// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { focusTerminal } from "./terminals";
import { useUiStore } from "./ui";
import { useShellStore } from "./store";
import type { AgentRow, SnapshotRest } from "./snapshot";
import type { DispatchFn } from "./ws";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

vi.mock("./terminals", async (importOriginal) => ({ ...(await importOriginal<typeof import("./terminals")>()), focusTerminal: vi.fn() }));

const saved = useShellStore.getState();
afterEach(() => useShellStore.setState(saved, true));

const MEMORY = { kind: "memory_promote", target: "m72" };

function open(agents: Partial<AgentRow>[]) {
  const rest = { navigator: { devices: [{ id: "mac", kind: "local" }], workspaces: [{ id: "ws-1", device_id: "mac", path: "/repo", checkouts: [] }], focused_device_id: "mac" }, status: { remote: [] } } as unknown as SnapshotRest;
  useShellStore.setState({ rest, agents: agents.map((agent) => ({ state: { asking: false }, ...agent })) as AgentRow[], connection: "live" });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => (events.push(event), true));
  return { events, actions };
}

// PRD hide-ai-work-home B11, hide-ai-suggestions B11: a row its home owns opens the home, any other opens its pane.
it("opens the Hide AI tool's 제안 tab on the attempt's row for a row its home owns, and the pane for every other row", () => {
  const { events, actions } = open([
    { pane_id: "w1:p1", hide_ai: { task: MEMORY, home: "suggestions", workspace_id: "ws-1", turn: { kind: "pr_review", pr_number: 924 } } },
    { pane_id: "w1:p2" },
    // A kind this build has no home for, and a home whose Project the core has not named yet.
    { pane_id: "w1:p3", hide_ai: { task: { kind: "future_kind" } } },
    { pane_id: "w1:p4", hide_ai: { task: MEMORY, home: "suggestions" } },
  ]);
  actions.openListedAgent("w1:p1");
  expect(events.map((event) => [event.kind, event.payload])).toEqual(expect.arrayContaining([["memory_open", { workspace_id: "ws-1" }]]));
  expect(useUiStore.getState()).toMatchObject({ hideAiTab: "suggestions", openSuggestion: "m72" });
  expect(events.some((event) => event.kind === "focus_pane")).toBe(false);
  for (const pane of ["w1:p2", "w1:p3", "w1:p4"]) {
    events.length = 0;
    actions.openListedAgent(pane);
    expect(events.map((event) => event.kind), pane).toContain("focus_pane");
    expect(events.some((event) => event.kind === "memory_open"), pane).toBe(false);
  }
});

// An asking Hide AI agent waits on an answer only its session takes, so its row opens the pane, not the home.
it("opens the pane of a Hide AI agent that asks, even for a row its home owns", () => {
  const { events, actions } = open([
    { pane_id: "w1:p1", state: { asking: true } as AgentRow["state"], hide_ai: { task: MEMORY, home: "suggestions", workspace_id: "ws-1" } },
  ]);
  actions.openListedAgent("w1:p1");
  expect(events.map((event) => event.kind)).toContain("focus_pane");
  expect(events.some((event) => event.kind === "memory_open")).toBe(false);
});

// The core's answer to opening the pane already focused moves nothing, so the
// open itself takes the keys from the row or mark pressed (docs/UI_BEHAVIOR.md).
it("takes the keyboard when the pane opened is already the focused one on a Workspace", () => {
  const { events, actions } = open([{ pane_id: "w1:p1" }, { pane_id: "w1:p2" }]);
  useShellStore.setState({ focusedPaneId: "w1:p1" });
  useUiStore.setState({ screen: { kind: "workspace" } });
  vi.mocked(focusTerminal).mockClear();
  actions.openAgent("w1:p2");
  expect(focusTerminal).not.toHaveBeenCalled();
  actions.openAgent("w1:p1");
  expect(focusTerminal).toHaveBeenCalledWith("w1:p1");
  expect(events.filter((event) => event.kind === "focus_pane")).toHaveLength(2);
  // Off a Workspace the pane is not on screen; the Workspace coming forward focuses it.
  vi.mocked(focusTerminal).mockClear();
  useUiStore.setState({ screen: { kind: "main", deviceId: "mac" } });
  actions.openAgent("w1:p1");
  expect(focusTerminal).not.toHaveBeenCalled();
});

// Operator decision 2026-11-23: a row goes to its agent only, so its View stays to look at; the View's chip opens it in
// focus mode, the pane's Workspace brought forward, in one event.
it("opens only the pane for a row with a View to look at, and the View in focus mode from its chip", () => {
  const look = { group: "done", count: 1, view_id: "v9", label: "notes.md", workspace_id: "ws-1", checkout_id: "c-1" };
  const { events, actions } = open([{ pane_id: "w1:p1", mark: "look", look }]);
  actions.openListedAgent("w1:p1");
  expect(events.map((event) => event.kind)).toEqual(["focus_pane"]);
  expect(events[0]!.payload).not.toHaveProperty("reveal_unseen");
  events.length = 0;
  actions.openLook("w1:p1", look);
  expect(events).toEqual([{ schema_version: 2, kind: "pair_view", payload: { pane_id: "w1:p1", display_id: "v9", bring_forward: true } }]);
  expect(useUiStore.getState().pairEntry?.keys).toBe("desk");
});

// An automation run's View opens in the run's own screen, laid out as focus mode already (operator decision 2026-11-21).
it("opens an automation run's View in the run's screen", () => {
  const look = { group: "done", count: 1, view_id: "v9", label: "brief.html", workspace_id: null, checkout_id: null };
  const { events, actions } = open([{ pane_id: "w1:p1", mark: "look", look, hide_ai: { task: { kind: "automation", target: "a1" } } as never }]);
  useShellStore.setState({ automations: { items: [{ id: "a1", records: [{ kind: "run", number: 3, pane_id: "w1:p1" }] }] } as never });
  actions.openLook("w1:p1", look);
  expect(events.some((event) => event.kind === "pair_view")).toBe(false);
  expect(useUiStore.getState().screen).toMatchObject({ kind: "automations" });
});
