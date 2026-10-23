// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import type { SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { ToolRail } from "./Tools";
import { useUiStore, type DrawnLane } from "./ui";
import { emptyScope } from "../test/legacyAgentScope";
import type { DispatchFn } from "./ws";

// The rail's pin (PRD lane-two-columns B15, B19; UI_BEHAVIOR.md, The right
// lane): one button at the rail's bottom, above the `…`, whatever the lane draws.

function snapshot(pinned: boolean): SnapshotRest {
  return {
    navigator: {
      focused_device_id: "local", focused_checkout_id: "c", focused_workspace_id: "p",
      devices: [{ id: "local", label: "This Mac", kind: "local", state: "local" }],
      workspaces: [{ id: "p", label: "herdr-ide", path: "/p", device_id: "local", is_git: true, agent_scope: emptyScope(), checkouts: [{ id: "c", workspace_id: "p", path: "/p", label: "main", tabs: [], active_tab_id: null, strip: [] }] }],
    },
    workspace_view: { device_id: "local", path: "/p", open: true, documents: false, tool: "explorer", pinned, document_width: null, detail_width: null, explorer_width: null, panel_width: null, lane_called: 0, lane_calls: 0, lane_called_side: "tool" },
    ui_state: { left_sidebar_visible: true, sidebar_lens: "projects", detail: { items: [], shown: false } },
  } as unknown as SnapshotRest;
}

describe("the rail's pin", () => {
  let container: HTMLDivElement;
  let root: Root;
  let saved: ReturnType<typeof useShellStore.getState>;
  let savedUi: ReturnType<typeof useUiStore.getState>;
  let events: Parameters<DispatchFn>[0][];

  beforeEach(() => {
    saved = useShellStore.getState();
    savedUi = useUiStore.getState();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    events = [];
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(saved, true);
    useUiStore.setState(savedUi, true);
    vi.unstubAllGlobals();
  });

  async function render(pinned: boolean, mode: DrawnLane["mode"]) {
    const actions = createActions((event) => { events.push(event); return true; });
    await act(async () => {
      useShellStore.setState({ connection: "live", rest: snapshot(pinned) });
      useUiStore.setState({ screen: { kind: "workspace" }, laneShown: mode !== "closed", drawnLane: { mode, view: "none", tool: mode === "closed" ? null : "explorer", stacked: false } });
      root.render(<TooltipProvider><ToolRail actions={actions} /></TooltipProvider>);
    });
  }
  const pin = () => container.querySelector<HTMLButtonElement>("[data-lane-pin]");

  it("says what a press does in each state and sits just above the `…`", async () => {
    const cases = [[false, "overlay", "off", "false", "Pin · keep it beside the agents"], [true, "docked", "on", "true", "Unpin · back over the agents"], [true, "overlay", "narrow", "true", "Pin · too narrow, drawn over"]] as const;
    for (const [pinned, mode, state, pressed, hint] of cases) {
      await render(pinned, mode);
      expect(pin()?.getAttribute("data-lane-pin")).toBe(state);
      expect(pin()?.getAttribute("aria-pressed")).toBe(pressed);
      expect(pin()?.getAttribute("aria-description")).toBe(hint);
      expect(pin()?.nextElementSibling?.getAttribute("data-lane-more")).toBe("true");
    }
  });

  it("stays on the rail with the lane closed and pins from there", async () => {
    await render(false, "closed");
    expect(pin()?.getAttribute("data-lane-pin")).toBe("off");
    await act(async () => pin()!.click());
    expect(events.filter((event) => event.kind === "workspace_view").map((event) => event.payload)).toEqual([{ pinned: true }]);
  });
});
