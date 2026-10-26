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
  useShellStore.setState({ rest, agents: agents as AgentRow[], connection: "live" });
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
    { pane_id: "w1:p4", hide_ai: { task: MEMORY, home: "memory" } },
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
