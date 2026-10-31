// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import type { SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { ProjectLanePin } from "./ProjectLanePin";
import { ToolRail } from "./Tools";
import { useUiStore, type DrawnLane } from "./ui";
import { emptyScope } from "../test/legacyAgentScope";
import type { DispatchFn } from "./ws";

// The lane's pin (PRD lane-two-columns B15, B19; session-docs-and-assistant
// D-73, B33; UI_BEHAVIOR.md, The right lane): beside the ✕ of the column at
// the lane's edge, the tool column's while one is drawn, never on the rail.

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

describe("the lane's pin", () => {
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

  async function render(pinned: boolean, mode: DrawnLane["mode"], tool: DrawnLane["tool"] = "explorer") {
    const actions = createActions((event) => { events.push(event); return true; });
    await act(async () => {
      useShellStore.setState({ connection: "live", rest: snapshot(pinned) });
      useUiStore.setState({ screen: { kind: "workspace" }, drawnLane: { mode, view: tool === null ? "documents" : "none", tool, stacked: false } });
      root.render(
        <TooltipProvider>
          <div data-column="view"><ProjectLanePin column="view" actions={actions} /></div>
          {/* A tool column's head is drawn only while it shows a tool. */}
          {tool !== null ? <div data-column="tool"><ProjectLanePin column="tool" actions={actions} /></div> : null}
          <ToolRail actions={actions} />
        </TooltipProvider>,
      );
    });
  }
  const pins = () => [...container.querySelectorAll<HTMLButtonElement>("[data-lane-pin]")];

  it("says what a press does in each state, in the tool column's head alone", async () => {
    const cases = [[false, "overlay", "off", "false", "Pin · keep it beside the agents"], [true, "docked", "on", "true", "Unpin · back over the agents"], [true, "overlay", "narrow", "true", "Pin · too narrow, drawn over"]] as const;
    for (const [pinned, mode, state, pressed, hint] of cases) {
      await render(pinned, mode);
      expect(pins()).toHaveLength(1);
      const pin = pins()[0]!;
      expect(pin.closest("[data-column]")?.getAttribute("data-column")).toBe("tool");
      expect(pin.getAttribute("data-lane-pin")).toBe(state);
      expect(pin.getAttribute("aria-pressed")).toBe(pressed);
      expect(pin.getAttribute("aria-description")).toBe(hint);
    }
  });

  it("moves to the viewing column's head when no tool column is drawn, and pins from there", async () => {
    await render(false, "overlay", null);
    expect(pins().map((pin) => pin.closest("[data-column]")?.getAttribute("data-column"))).toEqual(["view"]);
    await act(async () => pins()[0]!.click());
    expect(events.filter((event) => event.kind === "workspace_view").map((event) => event.payload)).toEqual([{ pinned: true }]);
  });

  it("is not on the rail", async () => {
    await render(false, "closed", null);
    expect(container.querySelector("[data-lane-rail] [data-lane-pin], nav [data-lane-pin]")).toBeNull();
  });
});
